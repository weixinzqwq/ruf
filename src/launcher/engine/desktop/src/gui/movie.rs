use ruffle_render_wgpu::descriptors::Descriptors;
use ruffle_render_wgpu::target::{RenderTarget, RenderTargetFrame};
use std::borrow::Cow;
use std::sync::Arc;
use wgpu::util::DeviceExt;

pub(crate) const AOQI_RENDER_SIZE: (u32, u32) = (1200, 700);

/// One physical-pixel transform shared by blitting, input, IME and native views.
#[derive(Debug, Clone, Copy)]
pub(crate) struct MoviePresentation {
    pub rect: [f64; 4],
    render_size: (u32, u32),
}

impl MoviePresentation {
    pub fn new(width: u32, height: u32, menu_height: f64, fixed: bool) -> Self {
        let width = f64::from(width.max(1));
        let height = f64::from(height.max(1));
        let menu_height = menu_height.clamp(0.0, height - 1.0);
        let content_height = height - menu_height;
        if fixed {
            let (render_width, render_height) = AOQI_RENDER_SIZE;
            let scale = (width / f64::from(render_width))
                .min(content_height / f64::from(render_height));
            let presented_width = f64::from(render_width) * scale;
            let presented_height = f64::from(render_height) * scale;
            Self {
                rect: [
                    (width - presented_width) / 2.0,
                    menu_height + (content_height - presented_height) / 2.0,
                    presented_width,
                    presented_height,
                ],
                render_size: AOQI_RENDER_SIZE,
            }
        } else {
            Self {
                rect: [0.0, menu_height, width, content_height],
                render_size: (width as u32, content_height as u32),
            }
        }
    }

    pub fn movie_to_window(&self, x: f64, y: f64) -> (f64, f64) {
        (
            self.rect[0] + x * self.rect[2] / f64::from(self.render_size.0),
            self.rect[1] + y * self.rect[3] / f64::from(self.render_size.1),
        )
    }

    pub fn window_to_movie(&self, x: f64, y: f64) -> (f64, f64) {
        (
            (x - self.rect[0]) * f64::from(self.render_size.0) / self.rect[2],
            (y - self.rect[1]) * f64::from(self.render_size.1) / self.rect[3],
        )
    }

    pub fn movie_rect_to_window(&self, [x, y, width, height]: [f64; 4]) -> [f64; 4] {
        let (left, top) = self.movie_to_window(x, y);
        [
            left,
            top,
            width * self.rect[2] / f64::from(self.render_size.0),
            height * self.rect[3] / f64::from(self.render_size.1),
        ]
    }
}

#[derive(Debug)]
pub struct MovieViewRenderer {
    bind_group_layout: wgpu::BindGroupLayout,
    pipeline: wgpu::RenderPipeline,
    sampler: wgpu::Sampler,
    vertices: wgpu::Buffer,
}

fn get_vertices(width: u32, height: u32, presentation: MoviePresentation) -> [[f32; 4]; 6] {
    let [x, y, movie_width, movie_height] = presentation.rect;
    let width = f64::from(width.max(1));
    let height = f64::from(height.max(1));
    let left = (2.0 * x / width - 1.0) as f32;
    let right = (2.0 * (x + movie_width) / width - 1.0) as f32;
    let top = (1.0 - 2.0 * y / height) as f32;
    let bottom = (1.0 - 2.0 * (y + movie_height) / height) as f32;
    // x y u v
    [
        [left, top, 0.0, 0.0],
        [right, top, 1.0, 0.0],
        [right, bottom, 1.0, 1.0],
        [right, bottom, 1.0, 1.0],
        [left, bottom, 0.0, 1.0],
        [left, top, 0.0, 0.0],
    ]
}

impl MovieViewRenderer {
    pub fn new(
        device: &wgpu::Device,
        surface_format: wgpu::TextureFormat,
        width: u32,
        height: u32,
        presentation: MoviePresentation,
    ) -> Self {
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: None,
            source: wgpu::ShaderSource::Wgsl(Cow::Borrowed(include_str!("blit.wgsl"))),
        });
        let bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: None,
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        multisampled: false,
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: None,
            bind_group_layouts: &[Some(&bind_group_layout)],
            immediate_size: 0,
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: None,
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                entry_point: Some("vs_main"),
                module: &module,
                buffers: &[Some(wgpu::VertexBufferLayout {
                    array_stride: 4 * 4,
                    step_mode: wgpu::VertexStepMode::Vertex,
                    // 0: vec2 position
                    // 1: vec2 texture coordinates
                    attributes: &wgpu::vertex_attr_array![0 => Float32x2, 1 => Float32x2],
                })],
                compilation_options: Default::default(),
            },
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                unclipped_depth: false,
                conservative: false,
                cull_mode: None,
                front_face: wgpu::FrontFace::default(),
                polygon_mode: wgpu::PolygonMode::default(),
                strip_index_format: None,
            },
            depth_stencil: None,
            multisample: wgpu::MultisampleState {
                alpha_to_coverage_enabled: false,
                count: 1,
                mask: !0,
            },

            fragment: Some(wgpu::FragmentState {
                module: &module,
                entry_point: Some(if surface_format.is_srgb() {
                    "fs_main_srgb_framebuffer"
                } else {
                    "fs_main_linear_framebuffer"
                }),
                targets: &[Some(wgpu::ColorTargetState {
                    format: surface_format,
                    blend: Some(wgpu::BlendState::REPLACE),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
                compilation_options: Default::default(),
            }),
            multiview_mask: None,
            cache: None,
        });
        let vertices = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: None,
            contents: bytemuck::cast_slice(&get_vertices(width, height, presentation)),
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
        });

        Self {
            bind_group_layout,
            pipeline,
            sampler,
            vertices,
        }
    }

    pub fn update_resolution(
        &self,
        descriptors: &Descriptors,
        width: u32,
        height: u32,
        presentation: MoviePresentation,
    ) {
        descriptors.queue.write_buffer(
            &self.vertices,
            0,
            bytemuck::cast_slice(&get_vertices(width, height, presentation)),
        );
    }
}

#[derive(Debug)]
pub struct MovieView {
    renderer: Arc<MovieViewRenderer>,
    texture: wgpu::Texture,
    bind_group: wgpu::BindGroup,
    fixed_resolution: bool,
    #[cfg(feature = "tracy_images")]
    tracy_frame_captures: crate::tracy::FrameCapturesHolder,
}

impl MovieView {
    pub fn new(
        renderer: Arc<MovieViewRenderer>,
        device: &wgpu::Device,
        width: u32,
        height: u32,
        fixed_resolution: bool,
        #[cfg(feature = "tracy_images")] tracy_frame_captures: crate::tracy::FrameCapturesHolder,
    ) -> Self {
        let (width, height) = if fixed_resolution { AOQI_RENDER_SIZE } else { (width, height) };
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: None,
            size: wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });
        let view = texture.create_view(&Default::default());
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: None,
            layout: &renderer.bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&renderer.sampler),
                },
            ],
        });
        #[cfg(feature = "tracy_images")]
        tracy_frame_captures.set_target(device, Some(&texture));
        Self {
            renderer,
            texture,
            bind_group,
            fixed_resolution,
            #[cfg(feature = "tracy_images")]
            tracy_frame_captures,
        }
    }

    pub fn render(
        &self,
        renderer: &MovieViewRenderer,
        render_pass: &mut wgpu::RenderPass<'static>,
    ) {
        render_pass.set_pipeline(&renderer.pipeline);
        render_pass.set_bind_group(0, &self.bind_group, &[]);
        render_pass.set_vertex_buffer(0, renderer.vertices.slice(..));
        render_pass.draw(0..6, 0..1);
    }
}

impl RenderTarget for MovieView {
    type Frame = MovieViewFrame;

    fn resize(&mut self, device: &wgpu::Device, width: u32, height: u32) {
        if self.fixed_resolution {
            // The offscreen target is independent of the swapchain. Reusing it
            // also avoids reallocating identical textures on every window resize.
            return;
        }
        *self = MovieView::new(
            self.renderer.clone(),
            device,
            width,
            height,
            self.fixed_resolution,
            #[cfg(feature = "tracy_images")]
            self.tracy_frame_captures.clone(),
        );
    }

    fn format(&self) -> wgpu::TextureFormat {
        self.texture.format()
    }

    fn width(&self) -> u32 {
        self.texture.width()
    }

    fn height(&self) -> u32 {
        self.texture.height()
    }

    fn get_next_texture(&mut self) -> Option<Self::Frame> {
        Some(MovieViewFrame(
            self.texture.create_view(&Default::default()),
        ))
    }

    fn submit<I: IntoIterator<Item = wgpu::CommandBuffer>>(
        &self,
        _device: &wgpu::Device,
        queue: &wgpu::Queue,
        command_buffers: I,
        _frame: Self::Frame,
    ) -> wgpu::SubmissionIndex {
        queue.submit(command_buffers)
    }
}

#[derive(Debug)]
pub struct MovieViewFrame(wgpu::TextureView);

impl RenderTargetFrame for MovieViewFrame {
    fn into_view(self) -> wgpu::TextureView {
        self.0
    }

    fn view(&self) -> &wgpu::TextureView {
        &self.0
    }
}

#[cfg(test)]
mod presentation_tests {
    use super::*;

    fn near(actual: f64, expected: f64) {
        assert!((actual - expected).abs() < 0.000_001, "{actual} != {expected}");
    }

    #[test]
    fn fixed_render_size_survives_window_dpi_and_fullscreen_sizes() {
        for (width, height, menu) in [(960, 584, 24.0), (1440, 876, 36.0), (3840, 2160, 0.0)] {
            let view = MoviePresentation::new(width, height, menu, true);
            assert_eq!(view.render_size, (1200, 700));
            near(view.rect[2] / view.rect[3], 12.0 / 7.0);
            let (x, y) = view.window_to_movie(view.rect[0] + view.rect[2] / 2.0, view.rect[1] + view.rect[3] / 2.0);
            near(x, 600.0);
            near(y, 350.0);
        }
    }

    #[test]
    fn high_dpi_menu_and_tall_window_letterbox_share_input_transform() {
        let view = MoviePresentation::new(1200, 1200, 48.0, true);
        assert_eq!(view.rect, [0.0, 274.0, 1200.0, 700.0]);
        assert_eq!(view.movie_to_window(0.0, 0.0), (0.0, 274.0));
        assert_eq!(view.window_to_movie(1200.0, 974.0), (1200.0, 700.0));
        assert!(view.window_to_movie(600.0, 100.0).1 < 0.0);
    }

    #[test]
    fn wide_fullscreen_has_side_bars_and_preserves_unclamped_pointer() {
        let view = MoviePresentation::new(1920, 1080, 0.0, true);
        near(view.rect[0], 240.0 / 7.0);
        near(view.rect[1], 0.0);
        near(view.rect[2], 12960.0 / 7.0);
        near(view.rect[3], 1080.0);
        assert!(view.window_to_movie(0.0, 540.0).0 < 0.0);
        let vertices = get_vertices(1920, 1080, view);
        assert!(vertices[0][0] > -1.0 && vertices[1][0] < 1.0);
        near(f64::from(vertices[0][1]), 1.0);
        near(f64::from(vertices[2][1]), -1.0);
    }

    #[test]
    fn webview_and_ime_rectangles_follow_the_same_stage_and_dpi_mapping() {
        let view = MoviePresentation::new(1920, 1200, 36.0, true);
        // A 960x560 SHOW_ALL stage maps to the fixed target with scale 1.25.
        let stage_rect = [230.0, 40.0, 500.0, 500.0];
        let internal = stage_rect.map(|value| value * 1.25);
        let physical = view.movie_rect_to_window(internal);
        let dip = physical.map(|value| value / 1.5);
        let restored = view.window_to_movie(dip[0] * 1.5, dip[1] * 1.5);
        near(restored.0 / 1.25, stage_rect[0]);
        near(restored.1 / 1.25, stage_rect[1]);
        near(physical[2] / physical[3], 1.0);
        let corner = view.window_to_movie(physical[0] + physical[2], physical[1] + physical[3]);
        near(corner.0, internal[0] + internal[2]);
        near(corner.1, internal[1] + internal[3]);
    }

    #[test]
    fn ordinary_ruffle_presentation_keeps_full_content_area() {
        let view = MoviePresentation::new(960, 584, 24.0, false);
        assert_eq!(view.rect, [0.0, 24.0, 960.0, 560.0]);
        assert_eq!(view.window_to_movie(40.0, 54.0), (40.0, 30.0));
        let vertices = get_vertices(960, 584, view);
        assert_eq!(vertices[0][0], -1.0);
        assert_eq!(vertices[1][0], 1.0);
        assert_eq!(vertices[2][1], -1.0);
    }

    #[test]
    fn minimized_dimensions_do_not_generate_nan_or_zero_render_targets() {
        let view = MoviePresentation::new(0, 0, 48.0, true);
        assert_eq!(view.render_size, (1200, 700));
        assert!(view.rect.iter().all(|value| value.is_finite()));
        assert!(view.rect[2] > 0.0 && view.rect[3] > 0.0);
        let (x, y) = view.window_to_movie(0.0, 0.0);
        assert!(x.is_finite() && y.is_finite());
    }
}
