//! Same-binary streamed/cached parameter comparisons; no game or network.
use crate::{backend::WgpuRenderBackend, filters::uniform_cache::FORCE_STREAMED_FILTERS, target::TextureTarget};
use ruffle_render::{backend::{BitmapCacheEntry, RenderBackend}, bitmap::{BitmapCacheHandle, PixelRegion, PixelSnapping},
    commands::{Command, CommandList}, filters::Filter, matrix::Matrix, quality::StageQuality, transform::Transform};
use std::{num::NonZeroU32, sync::Arc, time::{Duration, Instant}};
use swf::{Color, Fixed8, Fixed16, Twips};

fn draw(renderer: &mut WgpuRenderBackend<TextureTarget>, phase: u32, changing: bool, eviction: bool,
    handles: &mut Vec<(u32,u32,Arc<BitmapCacheHandle>)>) {
    let mut caches = Vec::new();
    let mut commands = CommandList::new();
    let count = if eviction { 80 } else { 26 };
    for i in 0..count {
        let variation = if changing { phase % 6 * 2 } else { 0 };
        let (w,h) = if eviction { (16+i%37, 12+i*7%29) } else { match i {
            0..12 => (128+i%3*16+variation,64+i%2*8),
            12..24 => (320+i%3*32+variation,192+i%2*16), _ => (800+variation,500),
        }};
        if handles.len() <= i as usize || (handles[i as usize].0,handles[i as usize].1) != (w,h) {
            let handle = renderer.create_bitmap_cache_texture(NonZeroU32::new(w).unwrap(),NonZeroU32::new(h).unwrap()).unwrap();
            if handles.len() <= i as usize { handles.push((w,h,handle)); }
            else { handles[i as usize] = (w,h,handle); }
        }
        let handle = handles[i as usize].2.get().unwrap();
        let mut contents = CommandList::new();
        for n in 0..4 {
            contents.commands.push(Command::DrawRect { color: Color {r:(30+n*7) as u8,g:(20+i*2) as u8,b:(15+phase*2) as u8,a:180},
                matrix: Matrix {a:(w/3) as f32,d:(h/3) as f32,tx:Twips::from_pixels(f64::from(n*7%w)),
                    ty:Twips::from_pixels(f64::from(n*5%h)),..Matrix::IDENTITY} });
        }
        let filters = (0..3).map(|n| {
            let seed = i*3+n+phase*997;
            Filter::GlowFilter(swf::GlowFilter {
                color: if eviction { Color {r:seed as u8,g:(seed/3) as u8,b:(seed/7) as u8,a:150} } else {Color::from_rgba(0xe08030b0)},
                blur_x: Fixed16::from_f32(if eviction {3.0+(seed%20) as f32/8.0} else {5.0}),
                blur_y: Fixed16::from_f32(if eviction {2.5+(seed%19) as f32/8.0} else {4.0}),
                strength: Fixed8::ONE, flags: swf::GlowFilterFlags::from_passes(2) | swf::GlowFilterFlags::COMPOSITE_SOURCE,
            })
        }).collect();
        caches.push(BitmapCacheEntry {handle:handle.clone(),commands:contents,clear:Color::TRANSPARENT,filters});
        commands.commands.push(Command::RenderBitmap {bitmap:handle,transform:Transform {matrix:Matrix {
            tx:Twips::from_pixels(f64::from(if eviction {i%10*70} else {i*43%900})),
            ty:Twips::from_pixels(f64::from(if eviction {i/10*60} else {i*21%400})),..Matrix::IDENTITY},..Default::default()},
            smoothing:true,pixel_snapping:PixelSnapping::Never,region:PixelRegion::for_whole_size(w,h)});
    }
    renderer.submit_frame(Color::from_rgba(0x303040ff),commands,caches);
}

#[test]
#[ignore = "explicit DX12 queued-draw correctness while immutable parameter cache evicts"]
fn filter_parameter_eviction_preserves_pixels_on_dx12() {
    let mut renderer = WgpuRenderBackend::for_offscreen((1200,700),wgpu::Backends::DX12,wgpu::PowerPreference::HighPerformance).unwrap();
    let mut handles = Vec::new();
    for quality in [StageQuality::Low,StageQuality::High] {
        renderer.set_quality(quality);
        for phase in 0..2 {
            FORCE_STREAMED_FILTERS.set(true);
            draw(&mut renderer,phase,false,true,&mut handles);
            let reference = renderer.capture_frame().unwrap();
            FORCE_STREAMED_FILTERS.set(false);
            let before = renderer.resource_stats().unwrap().work;
            draw(&mut renderer,phase,false,true,&mut handles);
            let pixels = renderer.capture_frame().unwrap();
            let after = renderer.resource_stats().unwrap().work;
            let difference = reference.as_raw().iter().zip(pixels.as_raw()).map(|(a,b)|a.abs_diff(*b)).max().unwrap();
            println!("filter_eviction quality={quality:?} phase={phase} max_channel_difference={difference} cache_misses={} evictions={}",
                after.filter_uniform_cache_misses-before.filter_uniform_cache_misses,
                after.filter_uniform_cache_evictions-before.filter_uniform_cache_evictions);
            assert_eq!(difference,0);
            assert!(after.filter_uniform_cache_evictions>before.filter_uniform_cache_evictions);
            assert!(after.filter_uniform_cache_misses-after.filter_uniform_cache_evictions<=320);
            assert_eq!(after.filter_uniform_uploads-before.filter_uniform_uploads,0);
            assert_eq!(after.glow_static_vertex_draws-before.glow_static_vertex_draws,240);
        }
    }
    let before_textures = renderer.driver_stats().unwrap().textures;
    renderer.set_texture_pool_limits(crate::aoqi::TexturePoolLimits {idle_bytes:0,idle_frames:120});
    renderer.device().poll(wgpu::PollType::Wait {submission_index:None,timeout:Some(Duration::from_secs(10))}).unwrap();
    let after_textures = renderer.driver_stats().unwrap().textures;
    let live = renderer.resource_stats().unwrap().temporary_texture.live_count;
    println!("binding_cache_release temporary_live={live} driver_textures_before={before_textures} driver_textures_after={after_textures}");
    assert_eq!(live,0);
    assert!(after_textures<before_textures);
}

#[test]
#[ignore = "explicit DX12 ABBA timing of 26 caches and 78 glow filters per frame"]
fn filter_submission_workload_on_dx12() {
    for changing in [false,true] {
        for (round,reference) in [true,false,false,true].into_iter().enumerate() {
            let mut renderer = WgpuRenderBackend::for_offscreen((1200,700),wgpu::Backends::DX12,wgpu::PowerPreference::HighPerformance).unwrap();
            renderer.set_quality(StageQuality::High);
            FORCE_STREAMED_FILTERS.set(reference);
            let mut handles = Vec::new();
            for phase in 0..8 {draw(&mut renderer,phase,changing,false,&mut handles);renderer.capture_frame().unwrap();}
            let before = renderer.resource_stats().unwrap();
            let native_before = wgpu::wgc::aoqi_timing::snapshot();
            let counts_before = wgpu::wgc::aoqi_timing::counts();
            let mut elapsed = 0.0;
            for phase in 8..20 {
                let started = Instant::now();
                draw(&mut renderer,phase,changing,false,&mut handles);
                renderer.device().poll(wgpu::PollType::Wait {submission_index:None,timeout:Some(Duration::from_secs(10))}).unwrap();
                elapsed += started.elapsed().as_secs_f64()*1000.0;
            }
            let after = renderer.resource_stats().unwrap();
            let a=after.work;let b=before.work;
            let native_after = wgpu::wgc::aoqi_timing::snapshot();
            let counts_after = wgpu::wgc::aoqi_timing::counts();
            println!("maintenance_full changing={changing} round={round} reference={reference} native_ms={:?} counts={:?}",
                native_after.iter().zip(native_before).map(|(a,b)| (a-b) as f64/1e6/12.0).collect::<Vec<_>>(),
                counts_after.iter().zip(counts_before).map(|(a,b)| a-b).collect::<Vec<_>>());
            assert_eq!(a.filter_calls[2]-b.filter_calls[2],78*12);
            assert_eq!(a.blur_passes-b.blur_passes,78*4*12);
            assert_eq!(a.queue_submissions-b.queue_submissions,12);
            assert!(a.native_submit_ns[0]>b.native_submit_ns[0]);
            assert!(a.native_submit_ns[4]>b.native_submit_ns[4]);
            assert_eq!(a.filter_uniform_uploads-b.filter_uniform_uploads,if reference {78*5*12} else {0});
            assert_eq!(a.glow_vertex_uploads-b.glow_vertex_uploads,if reference {78*12} else {0});
            let driver=renderer.driver_stats().unwrap();
            println!("filter_submit changing={changing} round={round} reference={reference} frames=12 mean_ms={:.3} glow_ms={:.3} finish_ms={:.3} execute_ms={:.3} uniform_prepare_ms={:.3} bind_ms={:.3} encode_ms={:.3} uniform_uploads={} vertex_uploads={} cache_hits={} cache_misses={} cache_evictions={} bind_reuses={} new_textures={} live_buffers={} local_usage={}",
                elapsed/12.0,(a.filter_ns[2]-b.filter_ns[2]) as f64/1e6/12.0,
                (a.command_finish_ns-b.command_finish_ns) as f64/1e6/12.0,(a.queue_execute_ns-b.queue_execute_ns) as f64/1e6/12.0,
                (a.filter_uniform_prepare_ns-b.filter_uniform_prepare_ns) as f64/1e6/12.0,
                (a.filter_bind_group_ns-b.filter_bind_group_ns) as f64/1e6/12.0,(a.filter_pass_encode_ns-b.filter_pass_encode_ns) as f64/1e6/12.0,
                a.filter_uniform_uploads-b.filter_uniform_uploads,a.glow_vertex_uploads-b.glow_vertex_uploads,
                a.filter_uniform_cache_hits-b.filter_uniform_cache_hits,a.filter_uniform_cache_misses-b.filter_uniform_cache_misses,
                a.filter_uniform_cache_evictions-b.filter_uniform_cache_evictions,a.filter_bind_group_reuses-b.filter_bind_group_reuses,
                after.temporary_texture.created_count-before.temporary_texture.created_count,driver.buffers,driver.local_usage.unwrap_or(0));
            println!("filter_pool changing={changing} round={round} reference={reference} msaa_created={} color_created={} stencil_created={} rejected={} sampler_live={} copy_helper_live={}",
                a.temporary_created[0]-b.temporary_created[0],a.temporary_created[1]-b.temporary_created[1],
                a.temporary_created[2]-b.temporary_created[2],
                after.temporary_pool_return_rejections-before.temporary_pool_return_rejections,
                driver.native_sampler_buffers.map_or(0,|s|s.live_count),driver.native_copy_buffers.map_or(0,|s|s.live_count));
            println!("command_submit changing={changing} round={round} reference={reference} encoder_create_ms={:.3} native_ms={:?} pool_candidates={} pool_lookup_ms={:.3}",
                (a.encoder_create_ns-b.encoder_create_ns) as f64/1e6/12.0,
                a.native_submit_ns.iter().zip(b.native_submit_ns).map(|(a,b)| (a-b) as f64/1e6/12.0).collect::<Vec<_>>(),
                a.pool_pressure_candidates-b.pool_pressure_candidates,
                (a.temporary_pool_lookup_ns-b.temporary_pool_lookup_ns) as f64/1e6/12.0);
            println!("command_maintenance changing={changing} round={round} reference={reference} counts={:?}", a.native_submit_counts.iter().zip(b.native_submit_counts).map(|(a,b)| a-b).collect::<Vec<_>>());
            println!("command_recording changing={changing} round={round} reference={reference} values={:?}",a.native_command_recording.iter().zip(b.native_command_recording).map(|(a,b)| (a-b) as f64/12.0).collect::<Vec<_>>());
            FORCE_STREAMED_FILTERS.set(false);
        }
    }
}

#[test]
#[ignore = "read-only DX12 allocation-info probe for actual native alignment"]
fn dx12_texture_allocation_sizes() {
    let renderer = WgpuRenderBackend::for_offscreen((64,64),wgpu::Backends::DX12,wgpu::PowerPreference::HighPerformance).unwrap();
    println!("allocation_adapter={:?}",renderer.descriptors().adapter.get_info().name);
    for (width,height) in [(32,32),(128,64),(320,192),(1200,700)] {
        for samples in [1,4] {
            let texture = renderer.device().create_texture(&wgpu::TextureDescriptor {
                label:None,size:wgpu::Extent3d {width,height,depth_or_array_layers:1},mip_level_count:1,sample_count:samples,
                dimension:wgpu::TextureDimension::D2,format:wgpu::TextureFormat::Rgba8Unorm,
                usage:wgpu::TextureUsages::RENDER_ATTACHMENT,view_formats:&[wgpu::TextureFormat::Rgba8Unorm],
            });
            // Keep both HAL guards alive while querying the native resource description.
            let device = unsafe {renderer.device().as_hal::<wgpu::hal::api::Dx12>()}.unwrap();
            let native = unsafe {texture.as_hal::<wgpu::hal::api::Dx12>()}.unwrap();
            let desc = unsafe {native.raw_resource().GetDesc()};
            let allocation = unsafe {device.raw_device().GetResourceAllocationInfo(0,&[desc])};
            let descriptor_bytes = u64::from(width)*u64::from(height)*u64::from(samples)*4;
            println!("native_allocation width={width} height={height} samples={samples} descriptor_bytes={descriptor_bytes} native_bytes={} alignment={}",allocation.SizeInBytes,allocation.Alignment);
            assert!(allocation.SizeInBytes>=descriptor_bytes);
            assert!(allocation.SizeInBytes<u64::MAX);
        }
    }
}
