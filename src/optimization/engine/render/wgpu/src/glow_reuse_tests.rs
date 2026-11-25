//! Explicit GPU regression: glow output, cropped inputs, and pool-pressure churn.
use crate::{aoqi::TexturePoolLimits, backend::WgpuRenderBackend};
use ruffle_render::{backend::RenderBackend, bitmap::{Bitmap, BitmapFormat, PixelRegion, PixelSnapping},
    commands::{Command, CommandList}, filters::Filter, quality::StageQuality, transform::Transform};
use std::{num::NonZeroU32, path::PathBuf};
use swf::{Color, Fixed8, Fixed16};

fn filter(case: u8) -> Filter {
    let mut flags = swf::GlowFilterFlags::from_passes(2) | swf::GlowFilterFlags::COMPOSITE_SOURCE;
    if case == 1 { flags |= swf::GlowFilterFlags::INNER_GLOW; }
    if case == 2 { flags = swf::GlowFilterFlags::from_passes(2) | swf::GlowFilterFlags::KNOCKOUT; }
    if case == 3 { flags = swf::GlowFilterFlags::COMPOSITE_SOURCE; }
    if case == 5 { flags = swf::GlowFilterFlags::from_passes(1) | swf::GlowFilterFlags::COMPOSITE_SOURCE; }
    if case == 7 { flags = swf::GlowFilterFlags::from_passes(3) | swf::GlowFilterFlags::COMPOSITE_SOURCE; }
    if case == 6 {
        return Filter::DropShadowFilter(swf::DropShadowFilter {
            color: Color::from_rgba(0x3060a0b0), blur_x: Fixed16::from_f32(5.5), blur_y: Fixed16::from_f32(4.0),
            angle: Fixed16::from_f32(0.7), distance: Fixed16::from_f32(3.5), strength: Fixed8::ONE,
            flags: swf::DropShadowFilterFlags::from_passes(2) | swf::DropShadowFilterFlags::COMPOSITE_SOURCE,
        });
    }
    Filter::GlowFilter(swf::GlowFilter {
        color: Color::from_rgba(0xe08030b0), strength: Fixed8::from_f32(if case == 7 { 3.5 } else { 1.0 }),
        blur_x: Fixed16::from_f32(if case == 4 { 1.0 } else { 5.5 }),
        blur_y: Fixed16::from_f32(if case == 4 || case == 5 { 1.0 } else { 4.0 }), flags,
    })
}

#[test]
#[ignore = "explicit DX12 glow reference pixels and allocation counts"]
fn glow_pixels_and_pressure_allocations_on_dx12() {
    let output = PathBuf::from(std::env::var("AOQI_GLOW_OUTPUT").expect("isolated output directory"));
    std::fs::create_dir_all(&output).unwrap();
    let reference = std::env::var("AOQI_GLOW_REFERENCE").ok().map(PathBuf::from);
    let mut renderer = WgpuRenderBackend::for_offscreen((96, 72), wgpu::Backends::DX12, wgpu::PowerPreference::HighPerformance).unwrap();
    renderer.set_quality(StageQuality::High);
    println!("adapter={:?}", renderer.descriptors().adapter.get_info().name);
    let mut pixels = Vec::new();
    for y in 0..48u16 { for x in 0..64u16 {
        let a = if (x / 8 + y / 6) % 2 == 0 { 200 } else { (x * 3 + y * 5) as u8 };
        pixels.extend_from_slice(&[a / 2, a / 3, a / 4, a]);
    }}
    let source = renderer.register_bitmap(Bitmap::new(64, 48, BitmapFormat::Rgba, pixels)).unwrap();
    let mut counts = Vec::new();
    for budget in [0, 96 * 1024 * 1024] {
        renderer.set_texture_pool_limits(TexturePoolLimits { idle_bytes: budget, idle_frames: 120 });
        for crop in [false, true] { for case in 0..8 {
            let destination = renderer.create_empty_texture(NonZeroU32::new(80).unwrap(), NonZeroU32::new(60).unwrap()).unwrap();
            let before = renderer.resource_stats().unwrap();
            let sync = renderer.apply_filter(source.clone(), if crop {(7,5)} else {(0,0)},
                if crop {(40,30)} else {(64,48)}, destination.clone(), if crop {(-3,2)} else {(2,2)}, filter(case)).unwrap();
            let after = renderer.resource_stats().unwrap();
            let created = after.temporary_texture.created_count - before.temporary_texture.created_count;
            let enabled = std::env::var("AOLA_FUSE_GLOW").as_deref() == Ok("1");
            let expected_fused = enabled && !crop && matches!(case, 0 | 1 | 2 | 7);
            assert_eq!(after.work.fused_glow_passes - before.work.fused_glow_passes, u64::from(expected_fused));
            if budget == 0 {
                assert_eq!(created, if case == 3 || case == 4 { 1 } else { 2 });
            }
            let mut commands = CommandList::new();
            commands.commands.push(Command::RenderBitmap { bitmap: destination, transform: Transform::default(),
                smoothing: true, pixel_snapping: PixelSnapping::Never, region: PixelRegion::for_whole_size(80,60) });
            renderer.submit_frame(Color::TRANSPARENT, commands, vec![]);
            let pixels = renderer.capture_frame().unwrap();
            drop(sync);
            let name = format!("budget-{budget}-crop-{crop}-case-{case}.rgba");
            if let Some(reference) = &reference {
                let expected = std::fs::read(reference.join(&name)).unwrap();
                assert_eq!(pixels.as_raw().len(), expected.len());
                let difference = pixels.as_raw().iter().zip(&expected).map(|(a,b)|a.abs_diff(*b)).max().unwrap();
                println!("comparison={name} max_channel_difference={difference}");
                assert_eq!(difference,0);
            }
            std::fs::write(output.join(&name), pixels.as_raw()).unwrap();
            counts.push(format!("{name} created={created}"));
            println!("glow_case={case} cropped={crop} idle_budget={budget} created={created}");
        }}
    }
    std::fs::write(output.join("allocations.txt"), counts.join("\n")).unwrap();
}

#[test]
#[ignore = "explicit 95-percent-glow workload competing with full-window targets"]
fn glow_pool_pressure_on_dx12() {
    let output=PathBuf::from(std::env::var("AOQI_GLOW_OUTPUT").expect("isolated output directory"));
    std::fs::create_dir_all(&output).unwrap();
    let reference=std::env::var("AOQI_GLOW_REFERENCE").ok().map(PathBuf::from);
    let mut renderer=WgpuRenderBackend::for_offscreen((1200,700),wgpu::Backends::DX12,wgpu::PowerPreference::HighPerformance).unwrap();
    renderer.set_quality(StageQuality::High);
    let mut handles=Vec::new();
    for phase in 0..3 { crate::backend_work_tests::render(&mut renderer,phase,true,true,true,&mut handles);renderer.capture_frame().unwrap(); }
    let before=renderer.resource_stats().unwrap();
    let mut elapsed = std::time::Duration::ZERO;
    for phase in 3..15 {
        let started = std::time::Instant::now();
        crate::backend_work_tests::render(&mut renderer,phase,true,true,true,&mut handles);
        renderer.device().poll(wgpu::PollType::Wait {submission_index:None,timeout:Some(std::time::Duration::from_secs(10))}).unwrap();
        elapsed += started.elapsed();
        if [3,7,14].contains(&phase) {
            let pixels=renderer.capture_frame().unwrap();let name=format!("glow-pressure-{phase}.rgba");
            if let Some(reference)=&reference {
                let expected=std::fs::read(reference.join(&name)).unwrap();
                assert_eq!(pixels.as_raw().len(),expected.len());
                let difference=pixels.as_raw().iter().zip(&expected).map(|(a,b)|a.abs_diff(*b)).max().unwrap();
                println!("comparison={name} max_channel_difference={difference}");assert_eq!(difference,0);
            }
            std::fs::write(output.join(name),pixels.as_raw()).unwrap();
        }
    }
    let after=renderer.resource_stats().unwrap();
    assert_eq!(after.work.queue_submissions-before.work.queue_submissions,12);
    assert_eq!(after.work.filter_calls[2]-before.work.filter_calls[2],38*12);
    let fused = after.work.fused_glow_passes - before.work.fused_glow_passes;
    let blurs = after.work.blur_passes - before.work.blur_passes;
    let enabled = std::env::var("AOLA_FUSE_GLOW").as_deref() == Ok("1");
    assert_eq!(fused, if enabled { 38 * 12 } else { 0 });
    assert_eq!(blurs + fused, 38 * 12 * 4);
    println!("fused_glow passes={fused} separate_blurs={blurs} total_ms={:.3}", elapsed.as_secs_f64() * 1000.0);
    println!("glow_pressure frames=12 created={} evicted={} rejected={} hits={} misses={}",
        after.temporary_texture.created_count-before.temporary_texture.created_count,
        after.temporary_pool_pressure_evictions-before.temporary_pool_pressure_evictions,
        after.temporary_pool_return_rejections-before.temporary_pool_return_rejections,
        after.temporary_pool_hits-before.temporary_pool_hits,after.temporary_pool_misses-before.temporary_pool_misses);
}
