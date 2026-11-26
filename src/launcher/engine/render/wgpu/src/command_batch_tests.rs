//! Ordered color-pass recording versus the original split-list reference path.
use crate::backend::WgpuRenderBackend;
use std::time::Duration;

struct SplitReference(bool);
impl SplitReference {
    fn new(force: bool) -> Self {
        Self(wgpu::wgc::aoqi_timing::force_split_passes_for_test(force))
    }
}
impl Drop for SplitReference {
    fn drop(&mut self) {
        wgpu::wgc::aoqi_timing::force_split_passes_for_test(self.0);
    }
}

fn wait(device: &wgpu::Device) {
    device.poll(wgpu::PollType::Wait {
        submission_index: None, timeout: Some(Duration::from_secs(10)),
    }).unwrap();
}

fn read(device: &wgpu::Device, buffer: &wgpu::Buffer) -> Vec<u8> {
    let (tx, rx) = std::sync::mpsc::channel();
    buffer.slice(..).map_async(wgpu::MapMode::Read, move |r| tx.send(r).unwrap());
    wait(device);
    rx.recv().unwrap().unwrap();
    let data = buffer.slice(..).get_mapped_range().unwrap().to_vec();
    buffer.unmap();
    data
}

fn pipeline(device: &wgpu::Device) -> wgpu::RenderPipeline {
    let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("inline ordering regression"),
        source: wgpu::ShaderSource::Wgsl(r#"
            @group(0) @binding(0) var source: texture_2d<f32>;
            @group(0) @binding(1) var<uniform> add: vec4<f32>;
            @vertex fn vs(@builtin(vertex_index) i: u32) -> @builtin(position) vec4<f32> {
                let positions = array(vec2(-1.0,-1.0),vec2(3.0,-1.0),vec2(-1.0,3.0));
                return vec4(positions[i],0.0,1.0);
            }
            @fragment fn fs(@builtin(position) p: vec4<f32>) -> @location(0) vec4<f32> {
                return textureLoad(source,vec2<i32>(p.xy),0) + add;
            }
        "#.into()),
    });
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: None, layout: None,
        vertex: wgpu::VertexState { module: &shader, entry_point: Some("vs"),
            compilation_options: Default::default(), buffers: &[] },
        primitive: Default::default(), depth_stencil: None, multisample: Default::default(),
        fragment: Some(wgpu::FragmentState { module: &shader, entry_point: Some("fs"),
            compilation_options: Default::default(), targets: &[Some(wgpu::TextureFormat::Rgba8Unorm.into())] }),
        multiview_mask: None, cache: None,
    })
}

fn texture(device: &wgpu::Device) -> wgpu::Texture {
    device.create_texture(&wgpu::TextureDescriptor {
        label: None, size: wgpu::Extent3d { width:16,height:16,depth_or_array_layers:1 },
        mip_level_count:1,sample_count:1,dimension:wgpu::TextureDimension::D2,
        format:wgpu::TextureFormat::Rgba8Unorm,
        usage:wgpu::TextureUsages::RENDER_ATTACHMENT|wgpu::TextureUsages::TEXTURE_BINDING|wgpu::TextureUsages::COPY_SRC,
        view_formats:&[],
    })
}

fn attachment(view: &wgpu::TextureView, load: wgpu::LoadOp<wgpu::Color>, store: wgpu::StoreOp) -> Option<wgpu::RenderPassColorAttachment<'_>> {
    Some(wgpu::RenderPassColorAttachment { view, depth_slice:None, resolve_target:None,
        ops:wgpu::Operations {load,store} })
}

// case 0: initialized ping-pong, interleaved buffer copy, and attachment Load;
// case 1: discard then read (must use the original immediate zero-init fixup);
// case 2: first reads of uninitialized texture and uniform must produce zero;
// case 3: timestamp pass + deferred query resolve between inline blocks.
fn run(case: u8, force_split: bool) -> (Vec<u8>, [u64; 19]) {
    let _reference = SplitReference::new(force_split);
    let renderer = WgpuRenderBackend::for_offscreen((16,16),wgpu::Backends::DX12,wgpu::PowerPreference::HighPerformance).unwrap();
    let device=renderer.device(); let queue=&renderer.descriptors().queue;
    let errors=device.push_error_scope(wgpu::ErrorFilter::Validation);
    let pipe=pipeline(device);
    let textures=[texture(device),texture(device)];
    let views=textures.each_ref().map(|t|t.create_view(&Default::default()));
    let uniform=device.create_buffer(&wgpu::BufferDescriptor {label:None,size:16,
        usage:wgpu::BufferUsages::UNIFORM|wgpu::BufferUsages::COPY_DST,mapped_at_creation:false});
    let groups=views.each_ref().map(|v|device.create_bind_group(&wgpu::BindGroupDescriptor {
        label:None,layout:&pipe.get_bind_group_layout(0),entries:&[
            wgpu::BindGroupEntry {binding:0,resource:wgpu::BindingResource::TextureView(v)},
            wgpu::BindGroupEntry {binding:1,resource:uniform.as_entire_binding()},
        ],
    }));
    let upload=device.create_buffer(&wgpu::BufferDescriptor {label:None,size:16,
        usage:wgpu::BufferUsages::COPY_SRC,mapped_at_creation:true});
    upload.slice(..).get_mapped_range_mut().unwrap().copy_from_slice(bytemuck::cast_slice(&[0f32,0.0,0.125,0.0]));
    upload.unmap();
    let output=device.create_buffer(&wgpu::BufferDescriptor {label:None,size:256*16,
        usage:wgpu::BufferUsages::COPY_DST|wgpu::BufferUsages::MAP_READ,mapped_at_creation:false});
    let queries=(case==3).then(||device.create_query_set(&wgpu::QuerySetDescriptor {label:None,ty:wgpu::QueryType::Timestamp,count:2}));
    let query_buffer=queries.as_ref().map(|_|device.create_buffer(&wgpu::BufferDescriptor {label:None,size:256,
        usage:wgpu::BufferUsages::QUERY_RESOLVE|wgpu::BufferUsages::COPY_SRC,mapped_at_creation:false}));
    let query_output=queries.as_ref().map(|_|device.create_buffer(&wgpu::BufferDescriptor {label:None,size:16,
        usage:wgpu::BufferUsages::COPY_DST|wgpu::BufferUsages::MAP_READ,mapped_at_creation:false}));
    let before=wgpu::wgc::aoqi_timing::counts();
    let mut encoder=device.create_command_encoder(&Default::default());
    if case!=2 {
        drop(encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            color_attachments:&[attachment(&views[0],wgpu::LoadOp::Clear(wgpu::Color {r:0.25,g:0.5,b:0.0,a:1.0}),wgpu::StoreOp::Store)],
            ..Default::default()
        }));
    }
    for step in 0..12 {
        let src=step%2; let dst=1-src;
        if case==0 && step==4 { encoder.copy_buffer_to_buffer(&upload,0,&uniform,0,16); }
        if case==1 && step==4 {
            drop(encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                color_attachments:&[attachment(&views[src],wgpu::LoadOp::Clear(wgpu::Color::RED),wgpu::StoreOp::Discard)],
                ..Default::default()
            }));
        }
        let timestamps=queries.as_ref().filter(|_|step==5).map(|q|wgpu::RenderPassTimestampWrites {
            query_set:q,beginning_of_pass_write_index:Some(0),end_of_pass_write_index:Some(1),
        });
        {
            let load=if step%3==0 {wgpu::LoadOp::Load} else {wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT)};
            let mut pass=encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label:Some("ordered pass"),color_attachments:&[attachment(&views[dst],load,wgpu::StoreOp::Store)],
                timestamp_writes:timestamps,..Default::default()
            });
            pass.set_pipeline(&pipe);pass.set_bind_group(0,&groups[src],&[]);
            pass.push_debug_group("draw");pass.draw(0..3,0..1);pass.pop_debug_group();
        }
        if case==3 && step==5 {
            encoder.resolve_query_set(queries.as_ref().unwrap(),0..2,query_buffer.as_ref().unwrap(),0);
            encoder.copy_buffer_to_buffer(query_buffer.as_ref().unwrap(),0,query_output.as_ref().unwrap(),0,16);
        }
    }
    encoder.copy_texture_to_buffer(textures[0].as_image_copy(),wgpu::TexelCopyBufferInfo {
        buffer:&output,layout:wgpu::TexelCopyBufferLayout {offset:0,bytes_per_row:Some(256),rows_per_image:None},
    },textures[0].size());
    let commands=encoder.finish();
    // The submitted trackers, not application handles, must retain everything.
    drop((groups,views,textures,uniform,upload,pipe,query_buffer,queries));
    queue.submit([commands]);wait(device);
    let after=wgpu::wgc::aoqi_timing::counts();
    let bytes=read(device,&output);
    let pixels=bytes.chunks_exact(256).flat_map(|r|r[..64].iter().copied()).collect::<Vec<_>>();
    if let Some(output)=query_output {
        let data=read(device,&output);
        let begin=u64::from_le_bytes(data[..8].try_into().unwrap());
        let end=u64::from_le_bytes(data[8..16].try_into().unwrap());
        assert!(begin>0 && end>=begin,"query resolve must remain after timestamp writes");
    }
    assert!(futures::executor::block_on(errors.pop()).is_none());
    (pixels,std::array::from_fn(|i|after[i]-before[i]))
}

#[test]
#[ignore = "DX12 inline/split equivalence, copies, Load, discard fixups, initialization and early handle drop"]
fn ordered_color_passes_preserve_pixels_and_lifetimes() {
    for case in 0..3 {
        let (old,old_counts)=run(case,true);let (new,counts)=run(case,false);
        assert_eq!(new,old,"case {case}");
        if case==0 { assert!(new.iter().any(|&x|x!=0)); }
        else { assert!(new.iter().all(|&x|x==0),"discarded/uninitialized reads must be zero"); }
        assert!(counts[15]>=10 && counts[17]>=8);
        assert!(counts[2]*2<old_counts[2],"native command lists must decrease");
        if case==1 { assert!(counts[16]>=2,"discard and subsequent read must fall back"); }
        println!("inline_ordering case={case} max_channel_difference=0 lists_before={} lists_after={} inline={} reused={} split={}",old_counts[2],counts[2],counts[15],counts[17],counts[16]);
    }
}

#[test]
#[ignore = "DX12 timestamp/deferred resolve boundaries between merged color passes"]
fn ordered_color_passes_preserve_query_resolves() {
    let (old,_)=run(3,true);let (new,counts)=run(3,false);
    assert_eq!(new,old);assert!(counts[15]>=10 && counts[16]>=1);
    println!("inline_queries max_channel_difference=0 writes_and_resolves_ordered=true");
}

#[test]
#[ignore = "DX12 validation errors must survive both fast and fallback recording paths"]
fn ordered_color_passes_keep_validation_errors() {
    for force_split in [true,false] {
        let _reference=SplitReference::new(force_split);
        let renderer=WgpuRenderBackend::for_offscreen((16,16),wgpu::Backends::DX12,wgpu::PowerPreference::HighPerformance).unwrap();
        let device=renderer.device();let pipe=pipeline(device);
        for case in 0..3 {
            let tex=texture(device);let view=tex.create_view(&Default::default());
            let uniform=device.create_buffer(&wgpu::BufferDescriptor {label:None,size:16,usage:wgpu::BufferUsages::UNIFORM,mapped_at_creation:false});
            let group=device.create_bind_group(&wgpu::BindGroupDescriptor {label:None,layout:&pipe.get_bind_group_layout(0),entries:&[
                wgpu::BindGroupEntry {binding:0,resource:wgpu::BindingResource::TextureView(&view)},
                wgpu::BindGroupEntry {binding:1,resource:uniform.as_entire_binding()},
            ]});
            let errors=device.push_error_scope(wgpu::ErrorFilter::Validation);
            let mut encoder=device.create_command_encoder(&Default::default());
            {
                let mut pass=encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    color_attachments:&[attachment(&view,wgpu::LoadOp::Clear(wgpu::Color::BLACK),wgpu::StoreOp::Store)],..Default::default()
                });
                match case {
                    0=>pass.draw(0..3,0..1), // missing pipeline, eligible fast header
                    1=>{pass.set_pipeline(&pipe);pass.set_bind_group(0,&group,&[]);pass.draw(0..3,0..1);}, // read/write alias must fall back and fail
                    _=>pass.set_scissor_rect(0,0,128,128), // fast path must still reject bad bounds
                }
            }
            let commands=encoder.finish();drop(commands);
            assert!(matches!(futures::executor::block_on(errors.pop()),Some(wgpu::Error::Validation {..})),"case={case} split={force_split}");
        }
    }
    println!("inline_validation missing_pipeline=true attachment_alias=true scissor_bounds=true");
}
