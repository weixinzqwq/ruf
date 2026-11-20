//! Explicit GPU regression/benchmark; no game, account, window or network.
use crate::{backend::WgpuRenderBackend, target::TextureTarget};
use ruffle_render::{
    backend::RenderBackend,
    bitmap::{Bitmap, BitmapFormat, BitmapHandle, PixelRegion, PixelSnapping},
    commands::{Command, CommandList, RenderBlendMode},
    matrix::Matrix, quality::StageQuality, transform::Transform,
};
use swf::{BlendMode, Color, Twips};
use std::time::{Duration, Instant};

fn bitmap(renderer: &mut WgpuRenderBackend<TextureTarget>) -> BitmapHandle {
    let mut pixels = Vec::new();
    for y in 0..24u16 {
        for x in 0..32u16 {
            let alpha = ((x * 7 + y * 11) % 256) as u8;
            pixels.extend_from_slice(&[alpha / 2, alpha / 3, alpha / 4, alpha]);
        }
    }
    renderer.register_bitmap(Bitmap::new(32, 24, BitmapFormat::Rgba, pixels)).unwrap()
}

fn scene(bitmap: &BitmapHandle, mode: BlendMode, reference: bool, fractional: bool, count: u32) -> CommandList {
    let mut commands = CommandList::new();
    // Overlapping opaque and translucent destination colors exercise blending.
    commands.commands.push(Command::DrawRect {
        color: Color { r: 60, g: 15, b: 30, a: 128 },
        matrix: Matrix::scale(100.0, 80.0),
    });
    for i in 0..count {
        let mut inner = CommandList::new();
        inner.commands.push(Command::RenderBitmap {
            bitmap: bitmap.clone(),
            transform: Transform { matrix: Matrix {
                tx: Twips::from_pixels(12.0 + f64::from((i * 19) % 1080) + if fractional { 0.25 } else { 0.0 }),
                ty: Twips::from_pixels(8.0 + f64::from((i * 7) % 600)),
                ..Matrix::IDENTITY
            }, ..Default::default() },
            smoothing: true, pixel_snapping: PixelSnapping::Never,
            region: PixelRegion::for_whole_size(32, 24),
        });
        if reference {
            // Transparent no-op forces the unchanged multi-command isolation
            // path without a production-only bypass flag.
            inner.commands.push(Command::DrawRect { color: Color::TRANSPARENT, matrix: Matrix::IDENTITY });
        }
        commands.commands.push(Command::Blend(inner, RenderBlendMode::Builtin(mode)));
    }
    commands
}

#[test]
#[ignore = "explicit DX12 pixel comparison and timing on the user's adapter"]
fn single_bitmap_blends_match_reference_on_dx12() {
    let mut renderer = WgpuRenderBackend::for_offscreen(
        (1200, 700), wgpu::Backends::DX12, wgpu::PowerPreference::HighPerformance,
    ).unwrap();
    println!("adapter={:?}", renderer.descriptors().adapter.get_info().name);
    let texture = bitmap(&mut renderer);
    for quality in [StageQuality::Low, StageQuality::High] {
        renderer.set_quality(quality);
        for mode in [BlendMode::Normal, BlendMode::Layer, BlendMode::Add, BlendMode::Subtract, BlendMode::Screen, BlendMode::Multiply] {
            for fractional in [false, true] {
                renderer.submit_frame(Color::from_rgba(0x47698fb0), scene(&texture, mode, true, fractional, 4), vec![]);
                let reference = renderer.capture_frame().unwrap();
                let before = renderer.resource_stats().unwrap().work.direct_bitmap_blends;
                renderer.submit_frame(Color::from_rgba(0x47698fb0), scene(&texture, mode, false, fractional, 4), vec![]);
                let optimized = renderer.capture_frame().unwrap();
                let fast = renderer.resource_stats().unwrap().work.direct_bitmap_blends - before;
                assert_eq!(fast, if fractional || mode == BlendMode::Multiply { 0 } else { 4 });
                let max_difference = reference.as_raw().iter().zip(optimized.as_raw()).map(|(a,b)| a.abs_diff(*b)).max().unwrap();
                println!("quality={quality:?} mode={mode:?} fractional={fractional} max_channel_difference={max_difference} fast_paths={fast}");
                assert!(max_difference <= 1, "Unexpected pixel change: {quality:?} {mode:?} fractional={fractional}");
            }
        }
    }
    renderer.set_quality(StageQuality::High);
    for case in ["masked", "color-transform", "scaled", "cropped"] {
        let make_scene = |reference| {
            let mut commands = scene(&texture, BlendMode::Add, reference, false, 1);
            if let Command::Blend(inner, _) = &mut commands.commands[1]
                && let Command::RenderBitmap { transform, region, .. } = &mut inner.commands[0]
            {
                match case {
                    "color-transform" => transform.color_transform.r_add = -30,
                    "scaled" => transform.matrix.a = 1.5,
                    "cropped" => { region.x_min = 3; region.x_max = 25; },
                    _ => {},
                }
            }
            if case == "masked" {
                let rectangle = Command::DrawRect { color: Color::WHITE, matrix: Matrix::scale(25.0, 25.0) };
                let mut masked = CommandList::new();
                masked.commands.extend([Command::PushMask, rectangle.clone(), Command::ActivateMask]);
                masked.commands.extend(commands.commands);
                masked.commands.extend([Command::DeactivateMask, rectangle, Command::PopMask]);
                masked
            } else { commands }
        };
        renderer.submit_frame(Color::from_rgba(0x47698fb0), make_scene(true), vec![]);
        let reference = renderer.capture_frame().unwrap();
        let before = renderer.resource_stats().unwrap().work.direct_bitmap_blends;
        renderer.submit_frame(Color::from_rgba(0x47698fb0), make_scene(false), vec![]);
        let optimized = renderer.capture_frame().unwrap();
        let fast = renderer.resource_stats().unwrap().work.direct_bitmap_blends - before;
        assert_eq!(fast, if case == "cropped" { 1 } else { 0 });
        let max_difference = reference.as_raw().iter().zip(optimized.as_raw()).map(|(a,b)| a.abs_diff(*b)).max().unwrap();
        println!("case={case} max_channel_difference={max_difference} fast_paths={fast}");
        assert!(max_difference <= 1, "Unexpected pixel change: {case}");
    }
    for reference in [true, false] {
        // Warm shaders and pools before taking a bounded, synchronized sample.
        renderer.submit_frame(Color::BLACK, scene(&texture, BlendMode::Add, reference, false, 32), vec![]);
        renderer.capture_frame().unwrap();
        let before = renderer.resource_stats().unwrap();
        let started = Instant::now();
        for _ in 0..12 {
            renderer.submit_frame(Color::BLACK, scene(&texture, BlendMode::Add, reference, false, 32), vec![]);
            renderer.device().poll(wgpu::PollType::Wait { submission_index: None, timeout: Some(Duration::from_secs(10)) }).unwrap();
        }
        let elapsed = started.elapsed().as_secs_f64() * 1000.0;
        let after = renderer.resource_stats().unwrap();
        println!("synthetic reference={reference} frames=12 sprites_per_frame=32 total_ms={elapsed:.3} mean_ms={:.3} surfaces={} new_temporary_textures={}",
            elapsed / 12.0, after.work.surface_passes-before.work.surface_passes,
            after.temporary_texture.created_count-before.temporary_texture.created_count);
    }
}
