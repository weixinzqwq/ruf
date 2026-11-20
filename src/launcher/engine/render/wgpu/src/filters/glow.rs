use crate::backend::RenderTargetMode;
use crate::buffer_pool::TexturePool;
use crate::descriptors::Descriptors;
use crate::filters::blur::BlurFilter;
use crate::filters::uniform_cache::{FilterUniformCache, streamed_reference};
use crate::filters::{
    FilterSource, FilterVertexWithBlur, VERTEX_BUFFERS_DESCRIPTION_FILTERS_WITH_BLUR,
};
use crate::surface::target::CommandTarget;
use crate::utils::SampleCountMap;
use bytemuck::{Pod, Zeroable};
use std::sync::OnceLock;
use swf::GlowFilter as GlowFilterArgs;
use wgpu::util::StagingBelt;

#[repr(C)]
#[derive(Copy, Clone, Debug, Pod, Zeroable, PartialEq)]
struct GlowUniform {
    color: [f32; 4],
    strength: f32,
    inner: u32,            // a wasteful bool, but we need to be aligned anyway
    knockout: u32,         // a wasteful bool, but we need to be aligned anyway
    composite_source: u32, // undocumented flash feature, another bool
}

pub struct GlowFilter {
    bind_group_layout: wgpu::BindGroupLayout,
    pipeline_layout: wgpu::PipelineLayout,
    vertex_buffer: wgpu::Buffer,
    full_vertices: wgpu::Buffer,
    #[cfg(test)]
    uniform_buffer: wgpu::Buffer,
    uniform_cache: FilterUniformCache,
    vertices_size: wgpu::BufferSize,
    #[cfg(test)]
    uniform_size: wgpu::BufferSize,
    pipeline: SampleCountMap<OnceLock<wgpu::RenderPipeline>>,
}

impl GlowFilter {
    pub fn new(device: &wgpu::Device) -> Self {
        #[cfg(test)]
        let uniform_size = std::mem::size_of::<GlowUniform>() as u64;
        let bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        multisampled: false,
                        sample_type: wgpu::TextureSampleType::Float { filterable: false },
                        view_dimension: wgpu::TextureViewDimension::D2,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::NonFiltering),
                    count: None,
                },
            ],
            label: create_debug_label!("Blur filter binds").as_deref(),
        });

        let vertices_size = std::mem::size_of::<[FilterVertexWithBlur; 4]>() as u64;
        let vertex_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: None,
            size: vertices_size,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let full_vertices = crate::create_buffer_with_data(device,
            bytemuck::cast_slice(&[[0.0,0.0], [1.0,0.0], [1.0,1.0], [0.0,1.0]].map(|position|
                FilterVertexWithBlur { position, source_uv: position, blur_uv: position })),
            wgpu::BufferUsages::VERTEX, None);
        #[cfg(test)]
        let uniform_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: None,
            size: uniform_size,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let uniform_cache = FilterUniformCache::new(device, 64);
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: None,
            bind_group_layouts: &[Some(&bind_group_layout), Some(&bind_group_layout), Some(&uniform_cache.layout)],
            immediate_size: 0,
        });

        Self {
            pipeline: Default::default(),
            pipeline_layout,
            vertex_buffer,
            full_vertices,
            #[cfg(test)]
            uniform_buffer,
            uniform_cache,
            bind_group_layout,
            #[cfg(test)]
            uniform_size: wgpu::BufferSize::new(uniform_size).expect("Definitely not zero."),
            vertices_size: wgpu::BufferSize::new(vertices_size).expect("Definitely not zero."),
        }
    }

    fn pipeline(&self, descriptors: &Descriptors, msaa_sample_count: u32) -> &wgpu::RenderPipeline {
        self.pipeline.get_or_init(msaa_sample_count, || {
            let label = create_debug_label!("Glow Filter ({} msaa)", msaa_sample_count);
            descriptors
                .device
                .create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                    label: label.as_deref(),
                    layout: Some(&self.pipeline_layout),
                    vertex: wgpu::VertexState {
                        module: &descriptors.shaders.glow_filter,
                        entry_point: Some("main_vertex"),
                        buffers: &VERTEX_BUFFERS_DESCRIPTION_FILTERS_WITH_BLUR,
                        compilation_options: Default::default(),
                    },
                    primitive: wgpu::PrimitiveState {
                        topology: wgpu::PrimitiveTopology::TriangleList,
                        strip_index_format: None,
                        front_face: wgpu::FrontFace::Ccw,
                        cull_mode: None,
                        polygon_mode: wgpu::PolygonMode::default(),
                        unclipped_depth: false,
                        conservative: false,
                    },
                    depth_stencil: None,
                    multisample: wgpu::MultisampleState {
                        count: msaa_sample_count,
                        mask: !0,
                        alpha_to_coverage_enabled: false,
                    },
                    fragment: Some(wgpu::FragmentState {
                        module: &descriptors.shaders.glow_filter,
                        entry_point: Some("main_fragment"),
                        targets: &[Some(wgpu::TextureFormat::Rgba8Unorm.into())],
                        compilation_options: Default::default(),
                    }),
                    multiview_mask: None,
                    cache: None,
                })
        })
    }

    #[expect(clippy::too_many_arguments)]
    pub fn apply(
        &self,
        descriptors: &Descriptors,
        texture_pool: &mut TexturePool,
        draw_encoder: &mut wgpu::CommandEncoder,
        staging_belt: &mut StagingBelt,
        source: &FilterSource,
        filter: &GlowFilterArgs,
        blur_filter: &BlurFilter,
        blur_offset: (f32, f32),
    ) -> CommandTarget {
        let sample_count = source.texture.sample_count();
        let format = source.texture.format();
        let pipeline = self.pipeline(descriptors, sample_count);
        let (blurred, scratch) = blur_filter.apply_with_scratch(
            descriptors,
            texture_pool,
            draw_encoder,
            staging_belt,
            source,
            &filter.inner_blur_filter(),
        ).map_or((None, None), |(blurred, scratch)| (Some(blurred), Some(scratch)));
        if let Some(blurred) = &blurred {
            blurred.ensure_cleared(draw_encoder);
        }

        // Reuse blur's inactive target directly. Returning it to a full idle
        // pool and immediately requesting it again can destroy/recreate it.
        let target = if let Some(mut scratch) = scratch {
            scratch.clear_on_next_pass(wgpu::Color::TRANSPARENT);
            scratch
        } else {
            CommandTarget::new(
                descriptors,
                texture_pool,
                wgpu::Extent3d {
                    width: source.size.0,
                    height: source.size.1,
                    depth_or_array_layers: 1,
                },
                format,
                sample_count,
                RenderTargetMode::FreshWithColor(wgpu::Color::TRANSPARENT),
                draw_encoder,
            )
        };
        let prepare_started = web_time::Instant::now();
        let uniform = GlowUniform {
                color: [
                    f32::from(filter.color.r) / 255.0,
                    f32::from(filter.color.g) / 255.0,
                    f32::from(filter.color.b) / 255.0,
                    f32::from(filter.color.a) / 255.0,
                ],
                strength: filter.strength.to_f32(),
                inner: if filter.is_inner() { 1 } else { 0 },
                knockout: if filter.is_knockout() { 1 } else { 0 },
                composite_source: if filter.composite_source() { 1 } else { 0 },
            };
        #[cfg(test)]
        let legacy_buffer = if streamed_reference() {
            staging_belt.write_buffer(draw_encoder, &self.uniform_buffer, 0, self.uniform_size)
                .copy_from_slice(bytemuck::bytes_of(&uniform));
            descriptors.resource_metrics.record_work(|s| s.filter_uniform_uploads += 1);
            Some(self.uniform_buffer.clone())
        } else { None };
        #[cfg(not(test))]
        let legacy_buffer: Option<wgpu::Buffer> = None;
        let (uniform_group, uniform_bind_time) = legacy_buffer.map(|buffer| self.uniform_cache.bind(descriptors, &buffer))
            .unwrap_or_else(|| self.uniform_cache.get(descriptors, bytemuck::bytes_of(&uniform)));
        descriptors.resource_metrics.record_work(|s| s.filter_uniform_prepare_ns += prepare_started.elapsed().saturating_sub(uniform_bind_time).as_nanos() as u64);
        let vertices = if !streamed_reference() && blur_offset == (0.0, 0.0)
            && source.point == (0, 0) && source.size == (source.texture.width(), source.texture.height())
        {
            descriptors.resource_metrics.record_work(|s| s.glow_static_vertex_draws += 1);
            &self.full_vertices
        } else {
            staging_belt.write_buffer(draw_encoder, &self.vertex_buffer, 0, self.vertices_size)
                .copy_from_slice(bytemuck::cast_slice(&source.vertices_with_blur_offset(blur_offset)));
            descriptors.resource_metrics.record_work(|s| s.glow_vertex_uploads += 1);
            &self.vertex_buffer
        };
        let bind_started = web_time::Instant::now();
        let source_view = source.view.cloned().unwrap_or_else(|| source.texture.create_view(&Default::default()));
        let blurred_view = blurred.as_ref().map(|target| target.color_view()).unwrap_or(&source_view);
        let create_group = |view: &wgpu::TextureView| descriptors.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: create_debug_label!("Glow texture group").as_deref(),
            layout: &self.bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(view) },
                wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::Sampler(
                    descriptors.bitmap_samplers.get_sampler(false, false)) },
            ],
        });
        let source_group = if !streamed_reference() && let Some(bindings) = source.bindings {
            bindings.get(descriptors, 1, || create_group(&source_view))
        } else { create_group(&source_view) };
        let blurred_group = if !streamed_reference() && let Some(blurred) = &blurred {
            blurred.filter_bind_group(descriptors, 1, || create_group(blurred_view))
        } else if blurred.is_none() { source_group.clone() }
        else { create_group(blurred_view) };
        let bind_ns = bind_started.elapsed().as_nanos() as u64;
        let encode_started = web_time::Instant::now();
        let mut render_pass = draw_encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: create_debug_label!("Glow filter").as_deref(),
            color_attachments: &[target.color_attachments()],
            ..Default::default()
        });
        render_pass.set_pipeline(pipeline);

        render_pass.set_bind_group(0, &source_group, &[]);
        render_pass.set_bind_group(1, &blurred_group, &[]);
        render_pass.set_bind_group(2, &uniform_group, &[]);

        render_pass.set_vertex_buffer(0, vertices.slice(..));
        render_pass.set_index_buffer(
            descriptors.quad.indices.slice(..),
            wgpu::IndexFormat::Uint32,
        );
        render_pass.draw_indexed(0..6, 0, 0..1);
        drop(render_pass);
        descriptors.resource_metrics.record_work(|s| {
            s.filter_bind_group_ns += bind_ns;
            s.filter_pass_encode_ns += encode_started.elapsed().as_nanos() as u64;
        });
        target
    }
}
