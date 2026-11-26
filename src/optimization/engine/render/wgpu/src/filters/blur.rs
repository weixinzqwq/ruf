use crate::backend::RenderTargetMode;
use crate::buffer_pool::TexturePool;
use crate::descriptors::Descriptors;
use crate::filters::{FilterSource, FilterVertex, VERTEX_BUFFERS_DESCRIPTION_FILTERS};
use crate::filters::uniform_cache::FilterUniformCache;
use crate::surface::target::CommandTarget;
use crate::utils::SampleCountMap;
use bytemuck::{Pod, Zeroable};
use std::sync::OnceLock;
use swf::BlurFilter as BlurFilterArgs;
use wgpu::util::StagingBelt;
use wgpu::{BufferSlice, CommandEncoder, RenderPipeline, TextureView};

/// This is a 1:1 match of `struct Filter` in `blur.wgsl`. See that, and the usage below, for more info.
/// Since WebGL requires 16 byte struct size (alignment), some of these fields (namely m2 and last_weight)
/// are passed in precomputed, even though they are trivial to get (addition/multiplication by constant).
/// The struct would have to be padded with dummy data otherwise anyway - these are at least useful.
#[repr(C)]
#[derive(Copy, Clone, Debug, Pod, Zeroable, PartialEq)]
struct BlurUniform {
    direction: [f32; 2],
    full_size: f32,
    m: f32,
    m2: f32,
    first_weight: f32,
    last_offset: f32,
    last_weight: f32,
}

pub(super) struct BlurTargets {
    pub output: CommandTarget,
    pub scratch: CommandTarget,
    pub deferred_uniform: Option<wgpu::BindGroup>,
}

pub struct BlurFilter {
    pub(super) shared_parameters: bool,
    pub(super) bind_group_layout: wgpu::BindGroupLayout,
    pipeline_layout: wgpu::PipelineLayout,
    vertex_buffer: wgpu::Buffer,
    #[cfg(test)]
    uniform_buffer: wgpu::Buffer,
    pub(super) uniform_cache: FilterUniformCache,
    vertices_size: wgpu::BufferSize,
    #[cfg(test)]
    uniform_size: wgpu::BufferSize,
    pipelines: SampleCountMap<OnceLock<wgpu::RenderPipeline>>,
}

impl BlurFilter {
    pub fn new(device: &wgpu::Device, shared_parameters: bool) -> Self {
        let texture = wgpu::BindGroupLayoutEntry {
            binding: 0,
            visibility: if shared_parameters { wgpu::ShaderStages::VERTEX_FRAGMENT } else { wgpu::ShaderStages::FRAGMENT },
            ty: wgpu::BindingType::Texture {
                multisampled: false,
                sample_type: wgpu::TextureSampleType::Float { filterable: true },
                view_dimension: wgpu::TextureViewDimension::D2,
            },
            count: None,
        };
        let sampling = wgpu::BindGroupLayoutEntry {
            binding: 1,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
            count: None,
        };
        #[cfg(test)]
        let uniform_size = std::mem::size_of::<BlurUniform>() as u64;
        let bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            entries: &[texture, sampling],
            label: create_debug_label!("Blur texture binds").as_deref(),
        });

        let vertices_size = std::mem::size_of::<[FilterVertex; 4]>() as u64;
        let vertex_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: None,
            size: vertices_size,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        #[cfg(test)]
        let uniform_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: None,
            size: uniform_size,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let uniform_cache = FilterUniformCache::new(device, 256);
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: None,
            bind_group_layouts: &[Some(&bind_group_layout), Some(&uniform_cache.layout)],
            immediate_size: 0,
        });

        Self {
            shared_parameters,
            pipelines: Default::default(),
            pipeline_layout,
            vertex_buffer,
            #[cfg(test)]
            uniform_buffer,
            uniform_cache,
            bind_group_layout,
            vertices_size: wgpu::BufferSize::new(vertices_size).expect("Definitely not zero."),
            #[cfg(test)]
            uniform_size: wgpu::BufferSize::new(uniform_size).expect("Definitely not zero."),
        }
    }

    fn pipeline(&self, descriptors: &Descriptors, msaa_sample_count: u32) -> &wgpu::RenderPipeline {
        self.pipelines.get_or_init(msaa_sample_count, || {
            let label = create_debug_label!("Blur Filter ({} msaa)", msaa_sample_count);
            descriptors
                .device
                .create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                    label: label.as_deref(),
                    layout: Some(&self.pipeline_layout),
                    vertex: wgpu::VertexState {
                        module: &descriptors.shaders.blur_filter,
                        entry_point: Some(if self.shared_parameters { "axis_vertex" } else { "main_vertex" }),
                        buffers: &VERTEX_BUFFERS_DESCRIPTION_FILTERS,
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
                        module: &descriptors.shaders.blur_filter,
                        entry_point: Some("main_fragment"),
                        targets: &[Some(wgpu::TextureFormat::Rgba8Unorm.into())],
                        compilation_options: Default::default(),
                    }),
                    multiview_mask: None,
                    cache: None,
                })
        })
    }

    pub fn apply(
        &self,
        descriptors: &Descriptors,
        texture_pool: &mut TexturePool,
        draw_encoder: &mut wgpu::CommandEncoder,
        staging_belt: &mut StagingBelt,
        source: &FilterSource,
        filter: &BlurFilterArgs,
    ) -> Option<CommandTarget> {
        self.apply_with_scratch(descriptors, texture_pool, draw_encoder, staging_belt, source, filter, false, None)
            .map(|targets| targets.output)
    }

    /// Keep the unused ping-pong target for the caller's following filter pass.
    pub(super) fn apply_with_scratch(
        &self,
        descriptors: &Descriptors,
        texture_pool: &mut TexturePool,
        draw_encoder: &mut wgpu::CommandEncoder,
        staging_belt: &mut StagingBelt,
        source: &FilterSource,
        filter: &BlurFilterArgs,
        defer_last_vertical: bool,
        first_slot: Option<&wgpu::Texture>,
    ) -> Option<BlurTargets> {
        if filter.num_passes() == 0
            || (filter.blur_x.to_f32() <= 1.0 && filter.blur_y.to_f32() <= 1.0)
        {
            return None;
        }
        let sample_count = source.texture.sample_count();
        let format = source.texture.format();
        let pipeline = self.pipeline(descriptors, sample_count);

        let initial_mode = match first_slot {
            Some(texture) => RenderTargetMode::ExistingWithColor(texture.clone(), wgpu::Color::TRANSPARENT),
            None => RenderTargetMode::FreshWithColor(wgpu::Color::TRANSPARENT),
        };
        let mut flip = CommandTarget::new(
            descriptors,
            texture_pool,
            wgpu::Extent3d {
                width: source.size.0,
                height: source.size.1,
                depth_or_array_layers: 1,
            },
            format,
            sample_count,
            initial_mode,
            draw_encoder,
        );
        let mut flop = CommandTarget::new(
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
        );

        let vertices = if source.point == (0, 0)
            && source.size == (source.texture.width(), source.texture.height())
        {
            descriptors.resource_metrics.record_work(|s| s.full_texture_filter_quads += 1);
            &descriptors.quad.filter_vertices
        } else {
            descriptors.resource_metrics.record_work(|s| s.cropped_filter_vertex_uploads += 1);
            staging_belt
                .write_buffer(draw_encoder, &self.vertex_buffer, 0, self.vertices_size)
                .copy_from_slice(bytemuck::cast_slice(&[source.vertices()]));
            &self.vertex_buffer
        };

        let source_view = source.view.cloned().unwrap_or_else(|| source.texture.create_view(&Default::default()));
        let mut first = true;
        for pass in 0..filter.num_passes() {
            for i in 0..2 {
                let horizontal = i % 2 == 0;
                let strength = if horizontal {
                    filter.blur_x.to_f32()
                } else {
                    filter.blur_y.to_f32()
                };
                // Full width of the kernel (left edge to right edge)
                let full_size = strength.min(255.0);
                if full_size <= 1.0 {
                    // A width of 1 or less is a noop (it'd just sample itself and nothing else)
                    continue;
                }

                let previous_target = if first { None } else { Some(&flip) };
                let (previous_view, previous_vertices, previous_width, previous_height) = if first {
                    first = false;
                    (
                        &source_view,
                        vertices.slice(..),
                        source.texture.width() as f32,
                        source.texture.height() as f32,
                    )
                } else {
                    (
                        flip.color_view(),
                        descriptors.quad.filter_vertices.slice(..),
                        flip.width() as f32,
                        flip.height() as f32,
                    )
                };

                // See this article for additional information on the fractional blur algorithm, as this
                // implementation was inspired by it: https://fgiesen.wordpress.com/2012/08/01/fast-blurs-2/

                // This is how much the blur "extends past" the center pixel to either side.
                let radius = (full_size - 1.0) / 2.0;

                // This is how many simple double-1 weighted pixel pairs we can sample in the center.
                // Note how we're not using floor() here. This is to guarantee that alpha is not 0 when
                // radius is a whole number: That would cause the division below to end the universe,
                // and more importantly, also waste at least one sampling of the texture (the first one).
                // This way, alpha is 1 instead in those cases (with m being one smaller), and the last
                // two samplings can be fused into one, at the right place and with the right weight.
                let m = radius.ceil() - 1.0;
                // Not the transparency kind. It's almost the fractional part of radius.
                // If radius is a whole number, however, it's 1 instead of 0.
                // The rounding is done to imitate the fixed-point calculations in Flash Player,
                // improving emulation accuracy somewhat.
                let alpha = ((radius - m) * 255.0).floor() / 255.0;

                // These control how and where the last pair of pixels are to be sampled,
                // so that the next-to-last will end up with an effective weight of 1.0,
                // and the last one with a weight of alpha. Note that the offset is relative
                // to the center of the next-to-last sampled pixel, in the range of 0 to 0.5.
                let last_offset = 1.0 / ((1.0 / alpha) + 1.0);
                let last_weight = alpha + 1.0;

                let uniform = BlurUniform {
                    // In the shared layout the vertex shader normalizes the axis
                    // using the actual input level, including a cropped first pass.
                    direction: match (horizontal, self.shared_parameters) {
                        (true, true) => [1.0, 0.0],
                        (false, true) => [0.0, 1.0],
                        (true, false) => [1.0 / previous_width, 0.0],
                        (false, false) => [0.0, 1.0 / previous_height],
                    },
                    full_size,
                    m,
                    m2: m * 2.0,
                    first_weight: alpha,
                    last_offset,
                    last_weight,
                };
                let prepare_started = web_time::Instant::now();
                #[cfg(test)]
                let legacy_buffer = if super::uniform_cache::streamed_reference() {
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

                if defer_last_vertical && !horizontal && pass + 1 == filter.num_passes() {
                    // The caller requires at least one horizontal pass first.
                    return Some(BlurTargets {
                        output: flip,
                        scratch: flop,
                        deferred_uniform: Some(uniform_group),
                    });
                }

                self.render_with_uniform_buffers(
                    descriptors,
                    draw_encoder,
                    pipeline,
                    &flop,
                    previous_view,
                    previous_vertices,
                    &uniform_group,
                    previous_target,
                    if previous_target.is_none() { source.bindings } else { None },
                );

                std::mem::swap(&mut flip, &mut flop);
            }
        }

        if first {
            // Nothing happened, don't return an empty unused texture
            None
        } else {
            Some(BlurTargets { output: flip, scratch: flop, deferred_uniform: None })
        }
    }

    pub(super) fn texture_group(
        &self,
        descriptors: &Descriptors,
        source: &TextureView,
    ) -> wgpu::BindGroup {
        descriptors
            .device
            .create_bind_group(&wgpu::BindGroupDescriptor {
                label: create_debug_label!("Filter group").as_deref(),
                layout: &self.bind_group_layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: wgpu::BindingResource::TextureView(source),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: wgpu::BindingResource::Sampler(
                            descriptors.bitmap_samplers.get_sampler(false, true),
                        ),
                    },
                ],
            })
    }

    fn render_with_uniform_buffers(
        &self,
        descriptors: &Descriptors,
        draw_encoder: &mut CommandEncoder,
        pipeline: &RenderPipeline,
        destination: &CommandTarget,
        source: &TextureView,
        vertices: BufferSlice,
        uniform_group: &wgpu::BindGroup,
        source_target: Option<&CommandTarget>,
        source_bindings: Option<&crate::buffer_pool::BlurBindings>,
    ) {
        let bind_started = web_time::Instant::now();
        let create_group = || self.texture_group(descriptors, source);
        let filter_group = if !super::uniform_cache::streamed_reference()
            && let Some(source_target) = source_target {
            source_target.filter_bind_group(descriptors, 0, create_group)
        } else if !super::uniform_cache::streamed_reference() && let Some(bindings) = source_bindings {
            bindings.get(descriptors, 0, create_group)
        } else { create_group() };

        let bind_ns = bind_started.elapsed().as_nanos() as u64;
        let encode_started = web_time::Instant::now();
        let mut render_pass = draw_encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: create_debug_label!("Blur filter").as_deref(),
            color_attachments: &[destination.color_attachments()],
            ..Default::default()
        });
        render_pass.set_pipeline(pipeline);

        render_pass.set_bind_group(0, &filter_group, &[]);
        render_pass.set_bind_group(1, uniform_group, &[]);

        render_pass.set_vertex_buffer(0, vertices);
        render_pass.set_index_buffer(
            descriptors.quad.indices.slice(..),
            wgpu::IndexFormat::Uint32,
        );
        render_pass.draw_indexed(0..6, 0, 0..1);
        drop(render_pass);
        descriptors.resource_metrics.record_work(|s| {
            s.blur_passes += 1;
            s.filter_bind_group_ns += bind_ns;
            s.filter_pass_encode_ns += encode_started.elapsed().as_nanos() as u64;
        });
    }
}
