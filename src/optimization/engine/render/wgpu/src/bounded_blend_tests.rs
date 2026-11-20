//! Explicit DX12 comparisons of bounded canvases against the full-target path.
use crate::{backend::WgpuRenderBackend, surface::blend_bounds::FORCE_FULL_BLEND, target::TextureTarget};
use ruffle_render::{
    backend::{RenderBackend, ShapeHandle},
    bitmap::{Bitmap, BitmapFormat, BitmapHandle, BitmapSize, BitmapSource, PixelRegion, PixelSnapping},
    commands::{Command, CommandList, RenderBlendMode}, matrix::Matrix, quality::StageQuality,
    shape_utils::{DistilledShape, DrawCommand, DrawPath}, transform::Transform,
};
use std::time::{Duration, Instant};
use swf::{BlendMode, Color, Twips};

struct NoBitmaps;
impl BitmapSource for NoBitmaps {
    fn bitmap_size(&self, _: u16) -> Option<BitmapSize> { None }
    fn bitmap_handle(&self, _: u16, _: &mut dyn RenderBackend) -> Option<BitmapHandle> { None }
}

fn assets(renderer: &mut WgpuRenderBackend<TextureTarget>) -> (BitmapHandle, ShapeHandle) {
    let pixels: Vec<u8> = (0..32*24).flat_map(|i| { let a = (i * 17 % 256) as u8; [a / 2, a / 3, a / 4, a] }).collect();
    let bitmap = renderer.register_bitmap(Bitmap::new(32, 24, BitmapFormat::Rgba, pixels)).unwrap();
    let style = swf::FillStyle::Color(Color::from_rgba(0x609030b0));
    let point = |x, y| swf::Point { x: Twips::from_pixels(x), y: Twips::from_pixels(y) };
    let bounds = swf::Rectangle { x_min: Twips::ZERO, x_max: Twips::from_pixels(32.0), y_min: Twips::ZERO, y_max: Twips::from_pixels(24.0) };
    let shape = renderer.register_shape(DistilledShape {
        paths: vec![DrawPath::Fill { style: &style, winding_rule: ruffle_render::shape_utils::FillRule::EvenOdd,
            commands: vec![DrawCommand::MoveTo(point(0.0, 0.0)), DrawCommand::LineTo(point(32.0, 5.0)),
                DrawCommand::QuadraticCurveTo { control: point(18.0, 24.0), anchor: point(2.0, 17.0) }, DrawCommand::LineTo(point(0.0, 0.0))] }],
        shape_bounds: bounds, edge_bounds: bounds, id: 1,
    }, &NoBitmaps);
    (bitmap, shape)
}

fn rectangle(x: f64, y: f64, w: f32, h: f32, color: Color) -> Command {
    Command::DrawRect { color, matrix: Matrix { a: w, d: h, tx: Twips::from_pixels(x), ty: Twips::from_pixels(y), ..Matrix::IDENTITY } }
}
fn group(commands: Vec<Command>) -> CommandList { let mut list = CommandList::new(); list.commands = commands; list }
fn blend(commands: CommandList, mode: BlendMode) -> Command { Command::Blend(commands, RenderBlendMode::Builtin(mode)) }

fn scene(bitmap: &BitmapHandle, shape: &ShapeHandle, mode: BlendMode, case: u32) -> CommandList {
    let color = Color::from_rgba(0x70503090);
    let mut inner = group(vec![rectangle(71.25, 52.5, 18.0, 12.0, color), rectangle(79.0, 55.0, 22.0, 18.0, color)]);
    match case {
        1 | 2 => {
            inner.commands.push(Command::RenderBitmap { bitmap: bitmap.clone(), transform: Transform {
                matrix: Matrix { a: if case == 1 { 1.2 } else { -0.8 }, b: 0.3, c: -0.2, d: 1.1,
                    tx: Twips::from_pixels(73.25), ty: Twips::from_pixels(59.5) }, ..Default::default() },
                smoothing: true, pixel_snapping: if case == 1 { PixelSnapping::Never } else { PixelSnapping::Always },
                region: PixelRegion { x_min: 3, x_max: 29, y_min: 2, y_max: 21 } });
        }
        3 => {
            inner.commands.push(Command::RenderShape { shape: shape.clone(), transform: Transform {
                matrix: Matrix { a: -1.1, b: 0.25, c: 0.2, d: 0.9, tx: Twips::from_pixels(97.25), ty: Twips::from_pixels(50.5) }, ..Default::default() } });
            inner.commands.push(Command::DrawLine { color, matrix: Matrix { a: 30.0, d: 19.0, tx: Twips::from_pixels(65.5), ty: Twips::from_pixels(47.5), ..Matrix::IDENTITY } });
            inner.commands.push(Command::DrawLineRect { color, matrix: Matrix { a: 35.0, d: 21.0, tx: Twips::from_pixels(65.0), ty: Twips::from_pixels(47.0), ..Matrix::IDENTITY } });
        }
        4 => {
            let mask = rectangle(68.5, 50.0, 24.0, 21.0, Color::WHITE);
            inner = group(vec![Command::PushMask, mask.clone(), Command::ActivateMask,
                blend(inner, BlendMode::Screen), Command::DeactivateMask, mask, Command::PopMask]);
        }
        5 => { inner = group(vec![rectangle(-8.5, -7.5, 21.0, 18.0, color), rectangle(1195.0, 696.0, 20.0, 15.0, color)]); }
        6 => { inner.commands.push(blend(group(vec![rectangle(78.5, 55.0, 12.0, 10.0, color)]), BlendMode::Alpha)); }
        7 => { inner = group(vec![Command::RenderAlphaMask { maskee_commands: inner, mask_commands: group(vec![rectangle(70.0, 51.0, 25.0, 20.0, color)]) }]); }
        8 => { inner = group(vec![rectangle(1400.0, 900.0, 30.0, 40.0, color)]); }
        _ => (),
    }
    let mut commands = group(vec![rectangle(0.0, 0.0, 1200.0, 700.0, Color::from_rgba(0x30405090)),
        rectangle(63.0, 45.0, 37.0, 33.0, Color::from_rgba(0x607020c0))]);
    let foreground = blend(inner, mode);
    if matches!(mode, BlendMode::Alpha | BlendMode::Erase) {
        commands.commands.push(blend(group(vec![rectangle(40.0, 30.0, 90.0, 60.0, color), foreground]), BlendMode::Layer));
    } else if case == 4 {
        // An external stencil must also survive the regional blend viewport.
        let mask = rectangle(60.0, 48.0, 36.0, 31.0, Color::WHITE);
        commands.commands.extend([Command::PushMask, mask.clone(), Command::ActivateMask, foreground,
            Command::DeactivateMask, mask, Command::PopMask]);
    } else { commands.commands.push(foreground); }
    commands
}

#[test]
#[ignore = "explicit DX12 full-target versus regional blend pixel comparisons"]
fn bounded_blends_match_full_targets_on_dx12() {
    let mut renderer = WgpuRenderBackend::for_offscreen((1200, 700), wgpu::Backends::DX12, wgpu::PowerPreference::HighPerformance).unwrap();
    println!("adapter={:?}", renderer.descriptors().adapter.get_info().name);
    let (bitmap, shape) = assets(&mut renderer);
    for quality in [StageQuality::Low, StageQuality::High] {
        renderer.set_quality(quality);
        for mode in [BlendMode::Normal, BlendMode::Layer, BlendMode::Add, BlendMode::Subtract, BlendMode::Screen,
            BlendMode::Multiply, BlendMode::Lighten, BlendMode::Darken, BlendMode::Difference, BlendMode::Invert,
            BlendMode::Overlay, BlendMode::HardLight, BlendMode::Alpha, BlendMode::Erase] {
            for case in 0..9 {
                FORCE_FULL_BLEND.set(true);
                renderer.submit_frame(Color::from_rgba(0x47698fb0), scene(&bitmap, &shape, mode, case), vec![]);
                let reference = renderer.capture_frame().unwrap();
                FORCE_FULL_BLEND.set(false);
                let before = renderer.resource_stats().unwrap().work;
                renderer.submit_frame(Color::from_rgba(0x47698fb0), scene(&bitmap, &shape, mode, case), vec![]);
                let optimized = renderer.capture_frame().unwrap();
                let after = renderer.resource_stats().unwrap().work;
                if case == 0 && !matches!(mode, BlendMode::Alpha | BlendMode::Erase) {
                    assert_eq!(after.bounded_blend_targets - before.bounded_blend_targets, 1);
                    assert!(after.blend_target_pixels - before.blend_target_pixels < 2000);
                }
                let difference = reference.as_raw().iter().zip(optimized.as_raw()).map(|(a,b)| a.abs_diff(*b)).max().unwrap();
                println!("quality={quality:?} mode={mode:?} case={case} max_channel_difference={difference}");
                assert_eq!(difference, 0, "pixel regression: {quality:?} {mode:?} case={case}");
            }
        }
    }
}

#[test]
#[ignore = "DX12 dynamic local-canvas sizes: exact pixels, bounded native bytes, fewer creations"]
fn aola_bucketed_blend_sizes_reuse_dynamic_targets_on_dx12() {
    use crate::surface::blend_bounds::FORCE_EXACT_BLEND_SIZE;
    use crate::aoqi::TexturePoolLimits;
    let mut reference = Vec::new();
    let mut creations = Vec::new();
    for exact in [true, false] {
        FORCE_EXACT_BLEND_SIZE.set(exact);
        let mut renderer = WgpuRenderBackend::for_offscreen((1200,700), wgpu::Backends::DX12,
            wgpu::PowerPreference::HighPerformance).unwrap();
        renderer.set_quality(StageQuality::High);
        renderer.set_texture_pool_limits(TexturePoolLimits {idle_bytes:256*1024*1024,idle_frames:120});
        let before=renderer.resource_stats().unwrap();
        let started=Instant::now();
        for frame in 0..24u32 {
            let commands=group((0..16u32).map(|i| {
                let x=f64::from(20+(i%4)*260);
                let y=f64::from(20+(i/4)*150);
                let width=(33+(i%4)*16+frame%8) as f32;
                let height=(19+(i/4)*16+frame%8) as f32;
                blend(group(vec![rectangle(x,y,width,height,Color::from_rgba(0x604020a0)),
                    rectangle(x+4.0,y+3.0,width-8.0,height-6.0,Color::from_rgba(0x20405080))]),BlendMode::Screen)
            }).collect());
            renderer.submit_frame(Color::from_rgba(0x305070ff),commands,vec![]);
            let pixels=renderer.capture_frame().unwrap().into_raw();
            if exact { reference.push(pixels); }
            else { assert!(pixels==reference[frame as usize],"pixel regression at frame {frame}"); }
            assert!(renderer.resource_stats().unwrap().temporary_pool_idle_allocation_bytes<=256*1024*1024);
        }
        let after=renderer.resource_stats().unwrap();
        let created=after.temporary_texture.created_count-before.temporary_texture.created_count;
        creations.push(created);
        println!("aola_canvas exact={exact} frames=24 creations={created} evictions={} rejections={} recurrent_misses={} padded_targets={} padding_pixels={} idle_native_bytes={} elapsed_ms={:.3}",
            after.temporary_pool_pressure_evictions-before.temporary_pool_pressure_evictions,
            after.temporary_pool_return_rejections-before.temporary_pool_return_rejections,
            after.temporary_pool_recurrent_misses-before.temporary_pool_recurrent_misses,
            after.work.blend_padded_targets-before.work.blend_padded_targets,
            after.work.blend_padding_pixels-before.work.blend_padding_pixels,
            after.temporary_pool_idle_allocation_bytes,started.elapsed().as_secs_f64()*1000.0);
    }
    FORCE_EXACT_BLEND_SIZE.set(false);
    assert!(creations[1]*2<creations[0],"size buckets did not reduce creations: {creations:?}");
}

#[test]
#[ignore = "explicit synchronized DX12 synthetic pressure measurement"]
fn bounded_blend_pressure_on_dx12() {
    for workload in ["blends", "filtered-glow"] {
    for (round, reference) in [true, false, false, true].into_iter().enumerate() {
        let mut renderer = WgpuRenderBackend::for_offscreen((1200, 700), wgpu::Backends::DX12, wgpu::PowerPreference::HighPerformance).unwrap();
        renderer.set_quality(StageQuality::High);
        FORCE_FULL_BLEND.set(reference);
        let mut handles = Vec::new();
        let mut render = |renderer: &mut WgpuRenderBackend<TextureTarget>, phase| {
            if workload == "filtered-glow" {
                crate::backend_work_tests::render(renderer, phase, true, true, true, &mut handles);
            } else {
                let commands = group((0..24u32).map(|i| blend(group(vec![
                    rectangle(f64::from(i*43), f64::from(i*17), 50.0, 40.0, Color::from_rgba(0x50464164)),
                    rectangle(f64::from(i*43+8), f64::from(i*17), 50.0, 40.0, Color::from_rgba(0x50464164)),
                ]), BlendMode::Multiply)).collect());
                renderer.submit_frame(Color::from_rgba(0x303040ff), commands, vec![]);
            }
        };
        for phase in 0..3 { render(&mut renderer, phase); renderer.capture_frame().unwrap(); }
        let before = renderer.resource_stats().unwrap();
        let started = Instant::now();
        for phase in 3..15 {
            render(&mut renderer, phase);
            renderer.device().poll(wgpu::PollType::Wait { submission_index: None, timeout: Some(Duration::from_secs(10)) }).unwrap();
        }
        let elapsed = started.elapsed().as_secs_f64() * 1000.0;
        let after = renderer.resource_stats().unwrap();
        let a = after.work; let b = before.work;
        let area = a.blend_target_pixels - b.blend_target_pixels;
        let full_area = a.blend_full_pixels - b.blend_full_pixels;
        if !reference { assert!(area * 100 < full_area); }
        assert_eq!(a.queue_submissions - b.queue_submissions, 12);
        println!("bounded_pressure workload={workload} round={round} reference={reference} frames=12 mean_ms={:.3} full_pixels={full_area} allocated_pixels={area} parent_copy_pixels={} new_textures={} created_estimated_bytes={} hits={} misses={} evictions={} rejections={} cache_ms={:.3} main_ms={:.3} finish_ms={:.3} execute_ms={:.3}",
            elapsed / 12.0, a.blend_parent_copy_pixels - b.blend_parent_copy_pixels,
            after.temporary_texture.created_count - before.temporary_texture.created_count,
            a.temporary_created_bytes.iter().sum::<u64>() - b.temporary_created_bytes.iter().sum::<u64>(),
            after.temporary_pool_hits - before.temporary_pool_hits, after.temporary_pool_misses - before.temporary_pool_misses,
            after.temporary_pool_pressure_evictions - before.temporary_pool_pressure_evictions,
            after.temporary_pool_return_rejections - before.temporary_pool_return_rejections,
            (a.cache_draw_ns-b.cache_draw_ns) as f64/1e6, (a.main_draw_ns-b.main_draw_ns) as f64/1e6,
            (a.command_finish_ns-b.command_finish_ns) as f64/1e6, (a.queue_execute_ns-b.queue_execute_ns) as f64/1e6);
        FORCE_FULL_BLEND.set(false);
    }
    }
}
