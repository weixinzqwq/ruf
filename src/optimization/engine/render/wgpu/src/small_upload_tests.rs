use super::*;
use ruffle_render::bitmap::PixelSnapping;
use ruffle_render::commands::Command;

const WIDTH: u32 = 96;
const HEIGHT: u32 = 72;
type Renderer = WgpuRenderBackend<TextureTarget>;

fn update(renderer: &mut Renderer, source: &BitmapHandle, pixels: &mut [u8], region: PixelRegion, value: u8) {
    for y in region.y_min..region.y_max {
        for x in region.x_min..region.x_max {
            let offset = ((y * WIDTH + x) * 4) as usize;
            pixels[offset..offset + 4].copy_from_slice(&[value, x as u8, y as u8, 255]);
        }
    }
    renderer.update_texture(source, Bitmap::new(WIDTH, HEIGHT, BitmapFormat::Rgba, &*pixels), region).unwrap();
}

fn draw(source: &BitmapHandle) -> CommandList {
    let mut commands = CommandList::new();
    commands.commands.push(Command::RenderBitmap {
        bitmap: source.clone(),
        transform: Default::default(),
        smoothing: false,
        pixel_snapping: PixelSnapping::Never,
        region: PixelRegion::for_whole_size(WIDTH, HEIGHT),
    });
    commands
}

fn check_readback(renderer: &mut Renderer, sync: Box<dyn SyncHandle>, expected: &[u8]) {
    let mut checked = false;
    renderer.resolve_sync_handle(sync, Box::new(|pixels, stride| {
        for (actual, expected) in pixels.chunks_exact(stride as usize).zip(expected.chunks_exact(WIDTH as usize * 4)) {
            assert_eq!(&actual[..expected.len()], expected);
        }
        assert_eq!(pixels.len(), stride as usize * HEIGHT as usize);
        checked = true;
    })).unwrap();
    assert!(checked);
}

#[test]
#[ignore = "explicit DX12 small BitmapData upload ordering, frame and readback comparison"]
fn bounded_small_uploads_match_pixels_on_dx12() {
    let mut renderer = Renderer::for_offscreen((WIDTH, HEIGHT), wgpu::Backends::DX12, wgpu::PowerPreference::HighPerformance).unwrap();
    renderer.set_quality(StageQuality::Low);
    let whole = PixelRegion::for_whole_size(WIDTH, HEIGHT);
    let small = PixelRegion::for_region(5, 2, 64, 64);
    for enabled in [false, true, true, false] {
        renderer.active_frame.batch_small_uploads = enabled;
        let mut pixels = [0, 0, 0, 255].repeat((WIDTH * HEIGHT) as usize);
        let source = renderer.register_bitmap(Bitmap::new(WIDTH, HEIGHT, BitmapFormat::Rgba, pixels.as_slice())).unwrap();
        let snapshot = renderer.create_empty_texture(WIDTH.try_into().unwrap(), HEIGHT.try_into().unwrap()).unwrap();
        let before = renderer.resource_stats().unwrap().work;
        for phase in 0..4u8 {
            for i in 0..17u8 {
                update(&mut renderer, &source, &mut pixels, PixelRegion::for_region(u32::from(i), 1, 64, 64), phase * 32 + i);
            }
            for region in [PixelRegion::for_region(0, 2, 65, 64), PixelRegion::for_region(3, 3, 64, 65), small, small] {
                update(&mut renderer, &source, &mut pixels, region, phase + 180);
            }
            renderer.submit_frame(Color::BLACK, draw(&source), Vec::new());
            assert_eq!(renderer.capture_frame().unwrap().as_raw(), &pixels);
        }
        let after = renderer.resource_stats().unwrap().work;
        let skips = after.small_upload_submissions_skipped - before.small_upload_submissions_skipped;
        let direct = after.direct_submissions - before.direct_submissions;
        println!("small_uploads enabled={enabled} frames=4 updates=84 skipped={skips} direct={direct}");
        assert_eq!(skips, if enabled { 60 } else { 0 });
        assert_eq!(direct, if enabled { 24 } else { 84 });
        assert_eq!(after.queue_submissions - before.queue_submissions, direct + 4);

        // Each producer records a sample of 'source'. A subsequent queue write must
        // not overtake that draw, including when the filter clips its output entirely.
        for case in ["draw", "filter", "clipped", "register"] {
            update(&mut renderer, &source, &mut pixels, small, 15);
            let previous = pixels.clone();
            let sync = match case {
                "draw" => renderer.render_offscreen(snapshot.clone(), draw(&source), StageQuality::Low, whole),
                "filter" | "clipped" => renderer.apply_filter(source.clone(), (0,0), (WIDTH, HEIGHT),
                    snapshot.clone(), (if case == "clipped" { WIDTH as i32 } else { 0 }, 0),
                    Filter::ColorMatrixFilter(Default::default())),
                _ => {
                    renderer.register_bitmap(Bitmap::new(WIDTH, HEIGHT, BitmapFormat::Rgba, previous.as_slice())).unwrap();
                    None
                }
            };
            assert_eq!(sync.is_some(), matches!(case, "draw" | "filter"));
            let before = renderer.resource_stats().unwrap().work.direct_submissions;
            update(&mut renderer, &source, &mut pixels, small, 219);
            assert_eq!(renderer.resource_stats().unwrap().work.direct_submissions, before + 1, "{case}");
            if let Some(sync) = sync {
                check_readback(&mut renderer, sync, &previous);
                assert_eq!(renderer.resource_stats().unwrap().work.direct_submissions, before + 2);
            }
            renderer.submit_frame(Color::BLACK, draw(&source), Vec::new());
            assert_eq!(renderer.capture_frame().unwrap().as_raw(), &pixels);
        }
    }
}
