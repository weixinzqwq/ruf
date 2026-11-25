use crate::{backend::WgpuRenderBackend, buffer_pool::TexturePool, aoqi::TexturePoolLimits};
use ruffle_render::backend::RenderBackend;

#[test]
#[ignore = "requires DX12; run twice with AOLA_LONG_RUN_MAINTENANCE=0 and 1"]
fn dx12_long_run_texture_pool_rounds() {
    let renderer = WgpuRenderBackend::for_offscreen((64, 64), wgpu::Backends::DX12,
        wgpu::PowerPreference::HighPerformance).unwrap();
    let long_run = ruffle_render::backend::aola_long_run_maintenance();
    let mut main = TexturePool::new();
    let mut offscreen = TexturePool::sharing_with(&main);
    let take_four = |pool: &mut TexturePool, width| {
        let size = wgpu::Extent3d { width, height: 32, depth_or_array_layers: 1 };
        // All four identical allocations must coexist before they return to the pool.
        let entries = [(); 4].map(|_| pool.get_texture(renderer.descriptors(), size,
            wgpu::TextureUsages::RENDER_ATTACHMENT, wgpu::TextureFormat::Rgba8Unorm, 1));
        drop(entries);
    };
    let start = renderer.resource_stats().unwrap();
    let mut hot_rebuilds = 0;
    for round in 0..6 {
        let first_cold = 128 + 4 * round;
        for frame in 0..40 {
            let before = renderer.resource_stats().unwrap().temporary_texture.created_count;
            take_four(&mut main, 64);
            if round != 0 || frame != 0 {
                hot_rebuilds += renderer.resource_stats().unwrap().temporary_texture.created_count - before;
            }
            if frame == 0 {
                for width in first_cold..first_cold + 4 { take_four(&mut offscreen, width); }
            }
            if frame == 12 { take_four(&mut offscreen, first_cold); }
            main.end_frame();
        }
        let stats = renderer.resource_stats().unwrap();
        let (bytes, items, buckets) = main.idle_stats();
        println!("long_run={long_run} round={} hits={} created={} hot_rebuilds={hot_rebuilds} idle_bytes={bytes} idle_items={items} buckets={buckets}",
            round + 1, stats.temporary_pool_hits - start.temporary_pool_hits,
            stats.temporary_texture.created_count - start.temporary_texture.created_count);
    }
    let stats = renderer.resource_stats().unwrap();
    let (_, items, buckets) = main.idle_stats();
    // The four-per-frame hot burst remains reusable in either mode. Each cold
    // shape revisited after twelve frames needs three extra allocations in B.
    assert_eq!(hot_rebuilds, 0);
    assert_eq!(stats.temporary_texture.created_count - start.temporary_texture.created_count,
               if long_run { 118 } else { 100 });
    assert_eq!(stats.temporary_pool_hits - start.temporary_pool_hits, if long_run { 962 } else { 980 });
    assert_eq!((items, buckets), if long_run { (5, 2) } else { (52, 13) });
    for _ in 0..120 { main.end_frame(); }
    assert_eq!(main.idle_stats(), (0, 0, 0));
}

#[test]
#[ignore = "requires a DX12 device; verifies native charging and hot-set survival"]
fn dx12_allocation_pool_pressure() {
    let renderer = WgpuRenderBackend::for_offscreen((64,64), wgpu::Backends::DX12, wgpu::PowerPreference::HighPerformance).unwrap();
    let descriptors = renderer.descriptors();
    let mut pool = TexturePool::new();
    pool.set_limits(TexturePoolLimits { idle_bytes: 96 * 1024 * 1024, idle_frames: 120 });
    let take = |pool: &mut TexturePool, width| pool.get_texture(descriptors,
        wgpu::Extent3d { width, height:32, depth_or_array_layers:1 },
        wgpu::TextureUsages::RENDER_ATTACHMENT, wgpu::TextureFormat::Rgba8Unorm, 4);
    for width in 32..48 { drop(take(&mut pool, width)); }
    let initial = renderer.resource_stats().unwrap().temporary_texture;
    assert!(initial.live_allocation_bytes > initial.live_estimated_bytes);
    let mut hot_misses = 0;
    for frame in 0..6 {
        // The recurring working set is used, followed by many one-use sizes.
        let before = renderer.resource_stats().unwrap().temporary_texture.created_count;
        for width in 32..48 { drop(take(&mut pool, width)); }
        hot_misses += renderer.resource_stats().unwrap().temporary_texture.created_count - before;
        for width in (128 + frame*32)..(160 + frame*32) { drop(take(&mut pool, width)); }
        assert!(pool.idle_stats().0 <= 96 * 1024 * 1024);
        pool.end_frame();
    }
    assert_eq!(hot_misses, 0);
    println!("allocation_pool hot_misses={hot_misses} idle_allocation_bytes={} descriptor_bytes={} native_bytes={}",
        pool.idle_stats().0,initial.live_estimated_bytes,initial.live_allocation_bytes);
    let held = take(&mut pool, 32);
    pool.set_limits(TexturePoolLimits { idle_bytes: 16 * 1024 * 1024, idle_frames: 120 });
    let retained = pool.idle_stats().0;
    assert!(retained > 0 && retained <= 16 * 1024 * 1024);
    pool.set_limits(TexturePoolLimits { idle_bytes: 0, idle_frames: 120 });
    assert_eq!(pool.idle_stats().0, 0);
    assert_eq!(held.0.width(), 32);
    drop(held);
    assert_eq!(pool.idle_stats().0, 0);
    drop(pool);
    assert_eq!(renderer.resource_stats().unwrap().temporary_texture.live_allocation_bytes, 0);
}

#[test]
#[ignore = "requires DX12; shape-style binding outlives the bitmap wrapper"]
fn dx12_binding_owner_lifetime() {
    use ruffle_render::bitmap::{Bitmap, BitmapFormat};
    let mut renderer = WgpuRenderBackend::for_offscreen((64,64), wgpu::Backends::DX12, wgpu::PowerPreference::HighPerformance).unwrap();
    renderer.descriptors().queue.submit([]);
    renderer.device().poll(wgpu::PollType::Wait {submission_index:None,timeout:Some(std::time::Duration::from_secs(10))}).unwrap();
    let before_sampler=renderer.driver_stats().unwrap().native_sampler_buffers.unwrap();
    let bitmap = renderer.register_bitmap(Bitmap::new(32,32,BitmapFormat::Rgba,vec![255;32*32*4])).unwrap();
    let descriptors=renderer.descriptors();
    let binding=crate::as_texture(&bitmap).bind_group(false,false,&descriptors.device,
        &descriptors.bind_layouts.bitmap,bitmap.clone(),&descriptors.bitmap_samplers).clone();
    assert!(bitmap.0.has_external_bindings());
    let live_native=renderer.driver_stats().unwrap();
    assert_eq!(live_native.native_sampler_buffers.unwrap().created_count,before_sampler.created_count+1);
    assert_eq!(live_native.native_buffers.unwrap().live_count
        - live_native.native_sampler_buffers.unwrap().live_count
        - live_native.native_copy_buffers.unwrap().live_count,live_native.buffers as u64);
    drop(bitmap);
    let stats=renderer.resource_stats().unwrap().uploaded_bitmap;
    assert_eq!((stats.live_count,stats.live_wrapper_count,stats.binding_references),(1,0,1));
    assert!(stats.binding_only_allocation_bytes>=65536);
    drop(binding);
    let stats=renderer.resource_stats().unwrap().uploaded_bitmap;
    assert_eq!((stats.live_count,stats.live_allocation_bytes,stats.binding_references),(0,0,0));
    renderer.descriptors().queue.submit([]);
    renderer.device().poll(wgpu::PollType::Wait {submission_index:None,timeout:Some(std::time::Duration::from_secs(10))}).unwrap();
    let after_sampler=renderer.driver_stats().unwrap().native_sampler_buffers.unwrap();
    assert_eq!(after_sampler.live_count,before_sampler.live_count);
    println!("binding_owner last_binding_releases=true sampler_buffer_retires=true");
}

#[test]
#[ignore = "requires DX12; recurring native working set larger than the old cap"]
fn dx12_recurring_pool_and_probation() {
    let renderer = WgpuRenderBackend::for_offscreen((64,64), wgpu::Backends::DX12, wgpu::PowerPreference::HighPerformance).unwrap();
    let descriptors = renderer.descriptors();
    let take = |pool: &mut TexturePool, width| pool.get_texture(descriptors,
        wgpu::Extent3d { width, height:32, depth_or_array_layers:1 },
        wgpu::TextureUsages::RENDER_ATTACHMENT, wgpu::TextureFormat::Rgba8Unorm, 4);
    let mut misses = Vec::new();
    for budget in [96, 256] {
        let mut pool = TexturePool::new();
        pool.set_limits(TexturePoolLimits {idle_bytes:budget*1024*1024,idle_frames:120});
        for width in 32..80 { drop(take(&mut pool,width)); }
        assert!(pool.idle_stats().0 <= budget*1024*1024, "first-use sizes exceeded native cap");
        pool.end_frame();
        for width in 32..80 { drop(take(&mut pool,width)); }
        pool.end_frame();
        let mut hot_misses = 0;
        for frame in 0..5 {
            let before = renderer.resource_stats().unwrap().temporary_texture.created_count;
            for width in 32..80 { drop(take(&mut pool,width)); }
            hot_misses += renderer.resource_stats().unwrap().temporary_texture.created_count-before;
            // Changing one-use dimensions must not displace recurring textures.
            for width in 128+frame*32..160+frame*32 { drop(take(&mut pool,width)); }
            assert!(pool.idle_stats().0 <= budget*1024*1024);
            pool.end_frame();
        }
        println!("recurring_pool budget_mib={budget} hot_misses={hot_misses} idle_bytes={}",pool.idle_stats().0);
        misses.push(hot_misses);
        for _ in 0..120 { pool.end_frame(); }
        assert_eq!(pool.idle_stats().0,0,"idle working set must expire");
    }
    assert!(misses[0] >= 120);
    assert_eq!(misses[1],0);
    assert_eq!(renderer.resource_stats().unwrap().temporary_texture.live_allocation_bytes,0);
}

#[test]
#[ignore = "requires DX12; verify mesh/gradient ownership and HAL retirement"]
fn dx12_mesh_gradient_and_native_retirement() {
    use ruffle_render::{bitmap::{BitmapSource,BitmapSize,BitmapHandle},shape_utils::{DistilledShape,DrawPath,DrawCommand,FillRule}};
    use swf::{Color,Twips};
    struct NoBitmaps;
    impl BitmapSource for NoBitmaps {
        fn bitmap_size(&self,_:u16)->Option<BitmapSize>{None}
        fn bitmap_handle(&self,_:u16,_:&mut dyn RenderBackend)->Option<BitmapHandle>{None}
    }
    let mut renderer = WgpuRenderBackend::for_offscreen((64,64),wgpu::Backends::DX12,wgpu::PowerPreference::HighPerformance).unwrap();
    let before = renderer.driver_stats().unwrap();
    let style = swf::FillStyle::LinearGradient(swf::Gradient {matrix:swf::Matrix::IDENTITY,
        spread:swf::GradientSpread::Pad,interpolation:swf::GradientInterpolation::Rgb,
        records:vec![swf::GradientRecord{ratio:0,color:Color::BLACK},swf::GradientRecord{ratio:255,color:Color::WHITE}]});
    let point=|x,y|swf::Point{x:Twips::from_pixels(x),y:Twips::from_pixels(y)};
    let bounds=swf::Rectangle{x_min:Twips::ZERO,y_min:Twips::ZERO,x_max:Twips::from_pixels(32.0),y_max:Twips::from_pixels(32.0)};
    let shape=renderer.register_shape(DistilledShape{id:1,shape_bounds:bounds,edge_bounds:bounds,
        paths:vec![DrawPath::Fill{style:&style,winding_rule:FillRule::EvenOdd,
            commands:vec![DrawCommand::MoveTo(point(0.0,0.0)),DrawCommand::LineTo(point(32.0,0.0)),DrawCommand::LineTo(point(32.0,32.0)),DrawCommand::LineTo(point(0.0,0.0))]}]},&NoBitmaps);
    let clone=shape.clone();
    let live=renderer.resource_stats().unwrap();
    assert_eq!((live.mesh.live_count,live.gradient_texture.live_count),(1,1));
    assert_eq!(live.mesh_vertex_buffer.live_count,1);
    assert_eq!(live.mesh_index_buffer.live_count,1);
    assert_eq!(live.mesh_uniform_buffer.live_count,1);
    assert!(live.gradient_texture.binding_only_allocation_bytes>=65536);
    assert!(live.mesh_uniform_buffer.binding_only_allocation_bytes>=65536);
    assert_eq!(live.gradient_texture.live_wrapper_count,0);
    assert_eq!(live.mesh_uniform_buffer.live_wrapper_count,0);
    let native=renderer.driver_stats().unwrap();
    assert!(native.native_buffers.unwrap().created_count>=before.native_buffers.unwrap().created_count+3);
    assert!(native.native_textures.unwrap().created_count>=before.native_textures.unwrap().created_count+1);
    drop(shape);
    assert_eq!(renderer.resource_stats().unwrap().mesh.live_count,1);
    drop(clone);
    let released=renderer.resource_stats().unwrap();
    for c in [released.mesh,released.gradient_texture,released.mesh_vertex_buffer,released.mesh_index_buffer,released.mesh_uniform_buffer] {
        assert_eq!(c.live_count,0);
        assert_eq!(c.created_allocation_bytes,c.released_allocation_bytes);
    }
    renderer.descriptors().queue.submit([]);
    renderer.descriptors().device.poll(wgpu::PollType::Wait {submission_index:None,timeout:Some(std::time::Duration::from_secs(10))}).unwrap();
    let retired=renderer.driver_stats().unwrap();
    for c in [retired.native_buffers.unwrap(),retired.native_textures.unwrap()] {
        assert_eq!(c.created_count-c.released_count,c.live_count);
        assert_eq!(c.created_allocation_bytes-c.released_allocation_bytes,c.live_allocation_bytes);
    }
    assert!(retired.native_buffers.unwrap().released_count>=before.native_buffers.unwrap().released_count+3);
    assert!(retired.native_textures.unwrap().released_count>=before.native_textures.unwrap().released_count+1);
    println!("mesh_gradient last_owner_release=true native_retirement=true");
}

#[test]
#[ignore = "requires DX12; exercises both misaligned-copy intermediate buffer paths"]
fn dx12_unaligned_copy_buffers_retire() {
    let renderer = WgpuRenderBackend::for_offscreen((64,64),wgpu::Backends::DX12,wgpu::PowerPreference::HighPerformance).unwrap();
    let device = renderer.device();
    let queue = &renderer.descriptors().queue;
    let wait = || device.poll(wgpu::PollType::Wait {submission_index:None,timeout:Some(std::time::Duration::from_secs(10))}).unwrap();
    queue.submit([]); wait();
    let baseline = renderer.driver_stats().unwrap().native_buffers.unwrap();
    let copy_before = renderer.driver_stats().unwrap().native_copy_buffers.unwrap();
    let buffer = device.create_buffer(&wgpu::BufferDescriptor {label:None,size:512,
        usage:wgpu::BufferUsages::COPY_SRC,mapped_at_creation:true});
    let mut initial = [0u8;512];
    initial[1] = 173;
    buffer.slice(..).get_mapped_range_mut().unwrap().copy_from_slice(&initial);
    buffer.unmap();
    let readback = device.create_buffer(&wgpu::BufferDescriptor {label:None,size:512,
        usage:wgpu::BufferUsages::COPY_DST|wgpu::BufferUsages::MAP_READ,mapped_at_creation:false});
    let texture = device.create_texture(&wgpu::TextureDescriptor {label:None,
        size:wgpu::Extent3d {width:1,height:1,depth_or_array_layers:1},mip_level_count:1,sample_count:1,
        dimension:wgpu::TextureDimension::D2,format:wgpu::TextureFormat::R8Unorm,
        usage:wgpu::TextureUsages::COPY_DST|wgpu::TextureUsages::COPY_SRC,view_formats:&[]});
    for _ in 0..8 {
        let mut encoder = device.create_command_encoder(&Default::default());
        let layout=wgpu::TexelCopyBufferLayout {offset:1,bytes_per_row:Some(256),rows_per_image:None};
        let extent=wgpu::Extent3d {width:1,height:1,depth_or_array_layers:1};
        encoder.copy_buffer_to_texture(wgpu::TexelCopyBufferInfo {buffer:&buffer,layout},texture.as_image_copy(),extent);
        encoder.copy_texture_to_buffer(texture.as_image_copy(),wgpu::TexelCopyBufferInfo {buffer:&readback,layout},extent);
        queue.submit([encoder.finish()]); wait();
    }
    let (tx,rx)=std::sync::mpsc::channel();
    readback.slice(..).map_async(wgpu::MapMode::Read, move |result|tx.send(result).unwrap());
    wait(); rx.recv().unwrap().unwrap();
    assert_eq!(readback.slice(..).get_mapped_range().unwrap()[1], 173);
    readback.unmap();
    drop((texture,buffer,readback));
    // Queue retirement returns completed encoders and their helper allocations.
    queue.submit([]); wait();
    let after=renderer.driver_stats().unwrap().native_buffers.unwrap();
    let copy_after = renderer.driver_stats().unwrap().native_copy_buffers.unwrap();
    assert_eq!(copy_after.created_count-copy_before.created_count,16,"both helper paths must execute");
    assert_eq!(copy_after.live_count,copy_before.live_count);
    assert_eq!(copy_after.released_count-copy_before.released_count,16);
    assert_eq!(after.live_count,baseline.live_count);
    assert_eq!(after.live_allocation_bytes,baseline.live_allocation_bytes);
    assert_eq!(after.created_count-baseline.created_count,after.released_count-baseline.released_count);
    println!("unaligned_copy pixel_match=true helpers_retired=true created={} released={}",
        after.created_count-baseline.created_count,after.released_count-baseline.released_count);
}

#[test]
#[ignore = "requires DX12; changing sizes replace cold owners without exceeding native cap"]
fn dx12_pool_new_sizes_replace_cold_idle() {
    let renderer=WgpuRenderBackend::for_offscreen((64,64),wgpu::Backends::DX12,wgpu::PowerPreference::HighPerformance).unwrap();
    let mut pool=TexturePool::new();
    pool.set_limits(TexturePoolLimits {idle_bytes:256*1024*1024,idle_frames:120});
    let take=|pool:&mut TexturePool,width| pool.get_texture(renderer.descriptors(),
        wgpu::Extent3d {width,height:32,depth_or_array_layers:1},wgpu::TextureUsages::RENDER_ATTACHMENT,
        wgpu::TextureFormat::Rgba8Unorm,4);
    for width in 32..96 {drop(take(&mut pool,width));}
    assert_eq!(pool.idle_stats().0,256*1024*1024);
    pool.end_frame();pool.end_frame();
    let before=renderer.resource_stats().unwrap();
    for width in 128..160 {drop(take(&mut pool,width));}
    let warm=renderer.resource_stats().unwrap();
    for width in 128..160 {drop(take(&mut pool,width));}
    let after=renderer.resource_stats().unwrap();
    assert_eq!(after.temporary_texture.created_count,warm.temporary_texture.created_count);
    assert_eq!(after.temporary_pool_return_rejections,before.temporary_pool_return_rejections);
    assert!(pool.idle_stats().0<=256*1024*1024);
    println!("new_size_pool hot_misses=0 return_rejections=0 idle_bytes={}",pool.idle_stats().0);
}

#[test]
#[ignore = "DX12: run with long-run maintenance on, with and without the battle texture pool"]
fn dx12_battle_pool_filter_working_set() {
    assert!(ruffle_render::backend::aola_long_run_maintenance());
    let candidate = crate::buffer_pool::battle_texture_pool_enabled();
    let renderer = WgpuRenderBackend::for_offscreen((64, 64), wgpu::Backends::DX12,
        wgpu::PowerPreference::HighPerformance).unwrap();
    let mut pool = TexturePool::new();
    let mut limits = TexturePoolLimits { idle_bytes: 4 * 1024 * 1024, idle_frames: 120 };
    pool.set_limits(limits);
    let color = wgpu::TextureFormat::Rgba8Unorm;
    let filter_usage = wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC
        | wgpu::TextureUsages::COPY_DST | wgpu::TextureUsages::TEXTURE_BINDING;
    let take = |pool: &mut TexturePool, width, samples, usage| pool.get_texture(
        renderer.descriptors(), wgpu::Extent3d { width, height: 16, depth_or_array_layers: 1 },
        usage, color, samples,
    );
    // A 4 MiB MSAA request competes with 64 distinct 64 KiB filtered outputs.
    for width in 16..80 { drop(take(&mut pool, width, 1, filter_usage)); }
    for _ in 0..3 { pool.end_frame(); }
    assert_eq!(pool.idle_stats().0, limits.idle_bytes);
    let before = renderer.resource_stats().unwrap();
    drop(take(&mut pool, 80, 4, wgpu::TextureUsages::RENDER_ATTACHMENT));
    let first = renderer.resource_stats().unwrap();
    let displaced = first.temporary_pool_pressure_evictions - before.temporary_pool_pressure_evictions;
    assert_eq!(displaced, if candidate { 0 } else { 64 });
    // A recurring MSAA key can still replace the cold single-sample working set.
    drop(take(&mut pool, 80, 4, wgpu::TextureUsages::RENDER_ATTACHMENT));
    let before_hit = renderer.resource_stats().unwrap().temporary_pool_hits;
    drop(take(&mut pool, 80, 4, wgpu::TextureUsages::RENDER_ATTACHMENT));
    assert_eq!(renderer.resource_stats().unwrap().temporary_pool_hits, before_hit + 1);
    assert_eq!(pool.idle_stats().0, limits.idle_bytes);

    pool.clear();
    for usage in [filter_usage, wgpu::TextureUsages::RENDER_ATTACHMENT] {
        let pair = [(); 2].map(|_| take(&mut pool, 32, 1, usage));
        drop(pair);
        for _ in 0..3 { pool.end_frame(); }
        let before = renderer.resource_stats().unwrap().temporary_pool_misses;
        drop([(); 2].map(|_| take(&mut pool, 32, 1, usage)));
        let rebuilds = renderer.resource_stats().unwrap().temporary_pool_misses - before;
        assert_eq!(rebuilds, u64::from(!(candidate && usage == filter_usage)));
        println!("candidate={candidate} usage={usage:?} pair_rebuilds={rebuilds}");
    }
    limits.idle_bytes = 64 * 1024;
    pool.set_limits(limits);
    assert!(pool.idle_stats().0 <= limits.idle_bytes);
    for _ in 0..120 { pool.end_frame(); }
    assert_eq!(pool.idle_stats(), (0, 0, 0));
    println!("candidate={candidate} first_msaa_displacements={displaced} all_expired=true");
}

#[test]
#[ignore = "requires DX12; released meshes/gradients/bitmaps rebuild identical pixels"]
fn dx12_rebuilt_library_resources_preserve_pixels() {
    use ruffle_render::{bitmap::{Bitmap,BitmapFormat,PixelRegion,PixelSnapping},
        commands::{Command,CommandList},shape_utils::{DistilledShape,DrawPath,DrawCommand,FillRule},transform::Transform};
    use swf::{Color,Twips};
    let mut renderer=WgpuRenderBackend::for_offscreen((64,64),wgpu::Backends::DX12,wgpu::PowerPreference::HighPerformance).unwrap();
    let mut reference=None;
    for cycle in 0..4 {
        let style=swf::FillStyle::LinearGradient(swf::Gradient {matrix:swf::Matrix::IDENTITY,
            spread:swf::GradientSpread::Pad,interpolation:swf::GradientInterpolation::Rgb,
            records:vec![swf::GradientRecord{ratio:0,color:Color::BLACK},swf::GradientRecord{ratio:255,color:Color::WHITE}]});
        let point=|x,y|swf::Point{x:Twips::from_pixels(x),y:Twips::from_pixels(y)};
        let bounds=swf::Rectangle{x_min:Twips::ZERO,y_min:Twips::ZERO,x_max:Twips::from_pixels(64.0),y_max:Twips::from_pixels(64.0)};
        let mesh=renderer.register_shape(DistilledShape{id:1,shape_bounds:bounds,edge_bounds:bounds,
            paths:vec![DrawPath::Fill{style:&style,winding_rule:FillRule::EvenOdd,
                commands:vec![DrawCommand::MoveTo(point(0.0,0.0)),DrawCommand::LineTo(point(64.0,0.0)),
                    DrawCommand::LineTo(point(64.0,64.0)),DrawCommand::LineTo(point(0.0,0.0))]}]},&ruffle_render::backend::null::NullBitmapSource);
        let bitmap=renderer.register_bitmap(Bitmap::new(16,16,BitmapFormat::Rgba,
            (0..16*16).flat_map(|i|[i as u8,80,120,200]).collect::<Vec<_>>())).unwrap();
        let mut commands=CommandList::new();
        commands.commands.push(Command::RenderShape {shape:mesh.clone(),transform:Transform::default()});
        commands.commands.push(Command::RenderBitmap {bitmap:bitmap.clone(),transform:Transform::default(),smoothing:true,
            pixel_snapping:PixelSnapping::Never,region:PixelRegion::for_whole_size(16,16)});
        renderer.submit_frame(Color::from_rgba(0x305070ff),commands,Vec::new());
        let pixels=renderer.capture_frame().unwrap().into_raw();
        if let Some(reference)=&reference {
            let difference=pixels.iter().zip(reference).map(|(a,b):(&u8,&u8)|a.abs_diff(*b)).max().unwrap();
            assert_eq!(difference,0);
            println!("library_resource_rebuild cycle={cycle} max_channel_difference={difference}");
        } else { reference=Some(pixels); }
        drop((mesh,bitmap));
        let stats=renderer.resource_stats().unwrap();
        assert_eq!((stats.mesh.live_count,stats.gradient_texture.live_count,stats.uploaded_bitmap.live_count),(0,0,0));
    }
}

#[test]
#[ignore = "requires DX12; cold MSAA eviction preserves small filter textures"]
fn dx12_pool_pressure_preserves_filter_targets() {
    let renderer=WgpuRenderBackend::for_offscreen((64,64),wgpu::Backends::DX12,wgpu::PowerPreference::HighPerformance).unwrap();
    let mut pool=TexturePool::new();
    pool.set_limits(TexturePoolLimits {idle_bytes:256*1024*1024,idle_frames:120});
    let take=|pool:&mut TexturePool,width,height,samples| pool.get_texture(renderer.descriptors(),
        wgpu::Extent3d {width,height,depth_or_array_layers:1},wgpu::TextureUsages::RENDER_ATTACHMENT,
        wgpu::TextureFormat::Rgba8Unorm,samples);
    for width in [511,512] {drop(take(&mut pool,width,512,1));}
    for width in 32..95 {drop(take(&mut pool,width,32,4));}
    pool.end_frame();pool.end_frame();
    drop(take(&mut pool,128,32,4));
    let before=renderer.resource_stats().unwrap().temporary_texture.created_count;
    for width in [511,512] {drop(take(&mut pool,width,512,1));}
    assert_eq!(renderer.resource_stats().unwrap().temporary_texture.created_count,before);
    assert!(pool.idle_stats().0<=256*1024*1024);
    println!("filter_target_priority small_target_rebuilds=0 idle_bytes={}",pool.idle_stats().0);
}

#[test]
#[ignore = "explicit DX12 lazy buffer initialization across queued and repeated submissions"]
fn buffer_initialization_preserves_partial_writes_on_dx12() {
    let renderer = WgpuRenderBackend::for_offscreen((64,64),wgpu::Backends::DX12,wgpu::PowerPreference::HighPerformance).unwrap();
    let device = renderer.device();
    let queue = &renderer.descriptors().queue;
    let source = device.create_buffer(&wgpu::BufferDescriptor {label:None,size:32,
        usage:wgpu::BufferUsages::COPY_SRC|wgpu::BufferUsages::COPY_DST,mapped_at_creation:false});
    let upload = crate::create_buffer_with_data(device,&[173u8;8],wgpu::BufferUsages::COPY_SRC,None);
    // Finish both command buffers before submitting either: the read's init
    // requirements must be reconciled with the preceding partial write.
    let mut first = device.create_command_encoder(&Default::default());
    first.copy_buffer_to_buffer(&upload,0,&source,8,8);
    let mut first = Some(first.finish());
    for round in 0..3 {
        let readback = device.create_buffer(&wgpu::BufferDescriptor {label:None,size:32,
            usage:wgpu::BufferUsages::COPY_DST|wgpu::BufferUsages::MAP_READ,mapped_at_creation:false});
        let mut encoder = device.create_command_encoder(&Default::default());
        encoder.copy_buffer_to_buffer(&source,0,&readback,0,32);
        let commands = encoder.finish();
        if round == 0 { queue.submit([first.take().unwrap(),commands]); } else { queue.submit([commands]); }
        let (sender, receiver) = std::sync::mpsc::channel();
        readback.slice(..).map_async(wgpu::MapMode::Read,move |result| sender.send(result).unwrap());
        device.poll(wgpu::PollType::Wait {submission_index:None,timeout:Some(std::time::Duration::from_secs(10))}).unwrap();
        receiver.recv().unwrap().unwrap();
        let bytes = readback.slice(..).get_mapped_range().unwrap();
        assert_eq!(&bytes[..8],&[0;8]);
        assert_eq!(&bytes[8..16],&[173;8]);
        assert_eq!(&bytes[16..],&[0;16]);
        drop(bytes);readback.unmap();
    }
    println!("buffer_initialization partial_writes_preserved=true first_reads_zero=true repeated_submission=true");
}

#[test]
#[ignore = "DX12 immutable sampler array sharing, ordering and last-owner release"]
fn dx12_sampler_index_sharing_and_retirement() {
    let renderer = WgpuRenderBackend::for_offscreen((64,64), wgpu::Backends::DX12, wgpu::PowerPreference::HighPerformance).unwrap();
    let device = renderer.device();
    let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: None, entries: &[0,1].map(|binding| wgpu::BindGroupLayoutEntry {
            binding, visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering), count: None,
        }),
    });
    let nearest = device.create_sampler(&Default::default());
    let linear = device.create_sampler(&wgpu::SamplerDescriptor { mag_filter: wgpu::FilterMode::Linear, ..Default::default() });
    let create = |a: &wgpu::Sampler,b: &wgpu::Sampler| device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: None, layout: &layout, entries: &[
            wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::Sampler(a) },
            wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::Sampler(b) },
        ],
    });
    let stats = || renderer.driver_stats().unwrap().native_sampler_buffers.unwrap();
    let before = stats();
    let mut groups: Vec<_> = (0..256).map(|_| create(&nearest,&linear)).collect();
    assert_eq!(stats().created_count-before.created_count,1);
    let reverse = create(&linear,&nearest);
    assert_eq!(stats().created_count-before.created_count,2);
    let last = groups.pop().unwrap();
    drop(groups);
    assert_eq!(stats().live_count-before.live_count,2);
    drop(last);
    assert_eq!(stats().live_count-before.live_count,1);
    drop(reverse);
    assert_eq!(stats().live_count,before.live_count);
    assert_eq!(stats().live_allocation_bytes,before.live_allocation_bytes);
    let recreated = create(&nearest,&linear);
    assert_eq!(stats().created_count-before.created_count,3);
    drop(recreated);
    assert_eq!(stats().released_count-before.released_count,3);
    println!("sampler_sharing groups=256 allocations=1 ordered_arrays_distinct=true last_owner_release=true expired_recreated=true");
}
