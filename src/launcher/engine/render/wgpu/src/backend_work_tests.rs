//! Bounded DX12 workloads sized from the captured counters (26 caches, 40
//! filters, roughly 480 commands/frame). This is synthetic, not a game replay.
use crate::{backend::WgpuRenderBackend, target::TextureTarget};
use ruffle_render::{
    backend::{BitmapCacheEntry, RenderBackend}, bitmap::{BitmapCacheHandle, PixelRegion, PixelSnapping},
    commands::{Command, CommandList, RenderBlendMode}, filters::Filter, matrix::Matrix,
    quality::StageQuality, transform::Transform,
};
use std::{num::NonZeroU32, path::PathBuf, sync::Arc, time::{Duration, Instant}};
use swf::{Color, Fixed8, Fixed16, Twips};

fn dimensions(index: u32, phase: u32, changing: bool) -> (u32, u32) {
    let variation = if changing { phase % 6 * 2 } else { 0 };
    match index {
        0..12 => (128 + index % 3 * 16 + variation, 64 + index % 2 * 8),
        12..24 => (320 + index % 3 * 32 + variation, 192 + index % 2 * 16),
        _ => (800 + variation, 500),
    }
}

fn filters(index: u32) -> Vec<Filter> {
    let mut matrix = swf::ColorMatrixFilter::default();
    matrix.matrix[0] = 0.9; matrix.matrix[9] = 12.0;
    let first = match index % 4 {
        0 => Filter::GlowFilter(swf::GlowFilter { color: Color::from_rgba(0xe08030b0), blur_x: Fixed16::from_f32(5.0), blur_y: Fixed16::from_f32(4.0), strength: Fixed8::ONE,
            flags: swf::GlowFilterFlags::from_passes(2) | swf::GlowFilterFlags::COMPOSITE_SOURCE }),
        1 => Filter::BlurFilter(swf::BlurFilter { blur_x: Fixed16::from_f32(4.5), blur_y: Fixed16::from_f32(3.0), flags: swf::BlurFilterFlags::from_passes(2) }),
        2 => Filter::DropShadowFilter(swf::DropShadowFilter { color: Color::from_rgba(0x204080a0), blur_x: Fixed16::from_f32(4.0), blur_y: Fixed16::from_f32(4.0), strength: Fixed8::ONE,
            angle: Fixed16::from_f32(0.6), distance: Fixed16::from_f32(3.0), flags: swf::DropShadowFilterFlags::from_passes(1) | swf::DropShadowFilterFlags::COMPOSITE_SOURCE }),
        _ => Filter::ColorMatrixFilter(matrix.clone()),
    };
    let mut result = vec![first];
    if index < 14 { result.push(Filter::ColorMatrixFilter(matrix)); }
    result
}

pub(super) fn render(renderer: &mut WgpuRenderBackend<TextureTarget>, phase: u32, changing: bool, pressure: bool, glow_dominated: bool,
    handles: &mut Vec<(u32, u32, Arc<BitmapCacheHandle>)>) {
    let mut caches = Vec::new();
    let mut commands = CommandList::new();
    for i in 0..26u32 {
        let (w,h) = dimensions(i,phase,changing);
        if handles.len() <= i as usize || (handles[i as usize].0, handles[i as usize].1) != (w,h) {
            let handle = renderer.create_bitmap_cache_texture(NonZeroU32::new(w).unwrap(),NonZeroU32::new(h).unwrap()).unwrap();
            if handles.len() <= i as usize { handles.push((w,h,handle)); }
            else { handles[i as usize]=(w,h,handle); }
        }
        let handle=handles[i as usize].2.get().unwrap();
        let mut contents=CommandList::new();
        for n in 0..17u32 {
            contents.commands.push(Command::DrawRect { color: Color { r:(30+n*3) as u8,g:(20+i*3) as u8,b:(15+phase*2) as u8,a:180 },
                matrix: Matrix { a: (w/8) as f32, d:(h/6) as f32, tx:Twips::from_pixels(f64::from(n*19%(w-30))),ty:Twips::from_pixels(f64::from(n*7%(h-20))),..Matrix::IDENTITY } });
        }
        caches.push(BitmapCacheEntry { handle:handle.clone(),commands:contents,clear:Color::TRANSPARENT,filters: if glow_dominated {
            let mut glow = filters(0);
            glow.truncate(1);
            if i < 14 { glow.push(if i < 12 { glow[0].clone() } else { filters(3).remove(0) }); }
            glow
        } else { filters(i) } });
        commands.commands.push(Command::RenderBitmap {bitmap:handle,transform:Transform {matrix:Matrix {tx:Twips::from_pixels(f64::from(i*43%900)),ty:Twips::from_pixels(f64::from(i*21%400)),..Matrix::IDENTITY},..Default::default()},smoothing:true,pixel_snapping:PixelSnapping::Never,region:PixelRegion::for_whole_size(w,h)});
    }
    if pressure {
        // Same-size full-window blend intermediates coexist, then compete with
        // small filtered caches next frame: exercise the observed pool churn.
        for i in 0..24u32 {
            let mut group=CommandList::new();
            for n in 0..2u32 {
                group.commands.push(Command::DrawRect {color:Color {r:80,g:70,b:65,a:100},matrix:Matrix {
                    a:50.0,d:40.0,tx:Twips::from_pixels(f64::from(i*43+n*8)),ty:Twips::from_pixels(f64::from(i*17)),..Matrix::IDENTITY}});
            }
            commands.commands.push(Command::Blend(group,RenderBlendMode::Builtin(swf::BlendMode::Multiply)));
        }
    }
    renderer.submit_frame(Color::from_rgba(0x303040ff),commands,caches);
}

#[test]
#[ignore = "explicit DX12 filtered-cache benchmark and reference image comparison"]
fn filtered_cache_workload_on_dx12() {
    let output=PathBuf::from(std::env::var("AOQI_BACKEND_OUTPUT").expect("isolated evidence directory"));
    std::fs::create_dir_all(&output).unwrap();
    let reference=std::env::var("AOQI_BACKEND_REFERENCE").ok().map(PathBuf::from);
    for (name,changing,pressure) in [("stable",false,false),("changing",true,false),("pressure",true,true)] {
        let mut renderer=WgpuRenderBackend::for_offscreen((1200,700),wgpu::Backends::DX12,wgpu::PowerPreference::HighPerformance).unwrap();
        renderer.set_quality(StageQuality::High);
        println!("workload={name} adapter={:?}",renderer.descriptors().adapter.get_info().name);
        let mut handles=Vec::new();
        for phase in 0..3 {render(&mut renderer,phase,changing,pressure,false,&mut handles);renderer.capture_frame().unwrap();}
        let before=renderer.resource_stats().unwrap();
        let mut total=0.0;
        for phase in 3..15 {
            let started=Instant::now();
            render(&mut renderer,phase,changing,pressure,false,&mut handles);
            renderer.device().poll(wgpu::PollType::Wait {submission_index:None,timeout:Some(Duration::from_secs(10))}).unwrap();
            total+=started.elapsed().as_secs_f64()*1000.0;
            if [3,7,14].contains(&phase) {
                let pixels=renderer.capture_frame().unwrap();let file=format!("{name}-{phase}.rgba");
                if let Some(reference)=&reference {
                    let expected=std::fs::read(reference.join(&file)).unwrap();
                    assert_eq!(pixels.as_raw().len(),expected.len());
                    let difference=pixels.as_raw().iter().zip(&expected).map(|(a,b)|a.abs_diff(*b)).max().unwrap();
                    println!("comparison={file} max_channel_difference={difference}");assert_eq!(difference,0);
                }
                std::fs::write(output.join(file),pixels.as_raw()).unwrap();
            }
        }
        let after=renderer.resource_stats().unwrap();let a=after.work;let b=before.work;
        // This workload is below the existing large-workload flush threshold.
        // A fixed per-eight-cache flush must not return unnoticed.
        assert_eq!(a.queue_submissions-b.queue_submissions,12);
        assert_eq!(a.direct_submissions-b.direct_submissions,0);
        assert_eq!(a.filter_calls.iter().sum::<u64>()-b.filter_calls.iter().sum::<u64>(),480);
        assert_eq!(a.temporary_created.iter().sum::<u64>()-b.temporary_created.iter().sum::<u64>(),
            after.temporary_texture.created_count-before.temporary_texture.created_count);
        println!("workload={name} queue_submissions={} direct_submissions={} full_filter_quads={} cropped_uploads={}",
            a.queue_submissions-b.queue_submissions,a.direct_submissions-b.direct_submissions,
            a.full_texture_filter_quads-b.full_texture_filter_quads,a.cropped_filter_vertex_uploads-b.cropped_filter_vertex_uploads);
        println!("workload={name} frames=12 mean_ms={:.3} cache_ms={:.3} main_ms={:.3} finish_ms={:.3} execute_ms={:.3} recall_ms={:.3} new_textures={} hits={} misses={} rejected={}",
            total/12.0,(a.cache_draw_ns-b.cache_draw_ns) as f64/1e6,(a.main_draw_ns-b.main_draw_ns) as f64/1e6,
            (a.command_finish_ns-b.command_finish_ns) as f64/1e6,(a.queue_execute_ns-b.queue_execute_ns) as f64/1e6,(a.staging_recall_ns-b.staging_recall_ns) as f64/1e6,
            after.temporary_texture.created_count-before.temporary_texture.created_count,after.temporary_pool_hits-before.temporary_pool_hits,
            after.temporary_pool_misses-before.temporary_pool_misses,after.temporary_pool_return_rejections-before.temporary_pool_return_rejections);
    }
}
