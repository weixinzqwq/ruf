use crate::filters::{FilterVertex, Filters};
use crate::layouts::BindLayouts;
use crate::pipelines::VERTEX_BUFFERS_DESCRIPTION_POS_UV;
use crate::shaders::Shaders;
use crate::resource_metrics::ResourceMetrics;
use crate::{
    BitmapSamplers, Pipelines, PosColorVertex, PosUvVertex, PosVertex, create_buffer_with_data,
};
use fnv::FnvHashMap;
use std::fmt::Debug;
use std::sync::{Arc, Mutex};
use wgpu::Backend;

pub struct Descriptors {
    pub(crate) resource_metrics: Arc<ResourceMetrics>,
    pub wgpu_instance: wgpu::Instance,
    pub adapter: wgpu::Adapter,
    pub device: wgpu::Device,
    pub limits: wgpu::Limits,
    pub backend: Backend,
    pub queue: wgpu::Queue,
    pub bitmap_samplers: BitmapSamplers,
    pub bind_layouts: BindLayouts,
    pub quad: Quad,
    copy_pipeline: Mutex<FnvHashMap<(u32, wgpu::TextureFormat), wgpu::RenderPipeline>>,
    pub shaders: Shaders,
    pipelines: Mutex<FnvHashMap<(u32, wgpu::TextureFormat), Arc<Pipelines>>>,
    pub filters: Filters,
}

impl Debug for Descriptors {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Descriptors")
    }
}

impl Descriptors {
    pub fn new(
        instance: wgpu::Instance,
        adapter: wgpu::Adapter,
        device: wgpu::Device,
        queue: wgpu::Queue,
    ) -> Self {
        let limits = device.limits();
        let bind_layouts = BindLayouts::new(&device);
        let bitmap_samplers = BitmapSamplers::new(&device);
        let shaders = Shaders::new(&device);
        let quad = Quad::new(&device);
        let filters = Filters::new(&device);
        let backend = adapter.get_info().backend;

        Self {
            resource_metrics: Arc::new(ResourceMetrics::default()),
            wgpu_instance: instance,
            adapter,
            device,
            limits,
            backend,
            queue,
            bitmap_samplers,
            bind_layouts,
            quad,
            copy_pipeline: Default::default(),
            shaders,
            pipelines: Default::default(),
            filters,
        }
    }

    pub fn copy_pipeline(
        &self,
        format: wgpu::TextureFormat,
        msaa_sample_count: u32,
    ) -> wgpu::RenderPipeline {
        let mut pipelines = self
            .copy_pipeline
            .lock()
            .expect("Pipelines should not be already locked");
        pipelines
            .entry((msaa_sample_count, format))
            .or_insert_with(|| {
                let copy_texture_pipeline_layout =
                    &self
                        .device
                        .create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                            label: create_debug_label!("Copy pipeline layout").as_deref(),
                            bind_group_layouts: &[
                                Some(&self.bind_layouts.globals),
                                None,
                                Some(&self.bind_layouts.bitmap),
                            ],
                            immediate_size: 0,
                        });
                self.device
                    .create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                        label: create_debug_label!("Copy pipeline").as_deref(),
                        layout: Some(copy_texture_pipeline_layout),
                        vertex: wgpu::VertexState {
                            module: &self.shaders.copy_shader,
                            entry_point: Some("main_vertex"),
                            buffers: &VERTEX_BUFFERS_DESCRIPTION_POS_UV,
                            compilation_options: Default::default(),
                        },
                        fragment: Some(wgpu::FragmentState {
                            module: &self.shaders.copy_shader,
                            entry_point: Some("main_fragment"),
                            targets: &[Some(wgpu::ColorTargetState {
                                format,
                                // All of our blending has been done by now, so we want
                                // to overwrite the target pixels without any blending
                                blend: Some(wgpu::BlendState::REPLACE),
                                write_mask: Default::default(),
                            })],
                            compilation_options: Default::default(),
                        }),
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
                        multiview_mask: None,
                        cache: None,
                    })
            })
            .clone()
    }

    pub fn pipelines(&self, msaa_sample_count: u32, format: wgpu::TextureFormat) -> Arc<Pipelines> {
        let mut pipelines = self
            .pipelines
            .lock()
            .expect("Pipelines should not be already locked");
        pipelines
            .entry((msaa_sample_count, format))
            .or_insert_with(|| {
                Arc::new(Pipelines::new(
                    &self.device,
                    &self.shaders,
                    format,
                    msaa_sample_count,
                    &self.bind_layouts,
                ))
            })
            .clone()
    }
}

pub struct Quad {
    pub vertices_pos: wgpu::Buffer,
    pub vertices_pos_uv: wgpu::Buffer,
    pub vertices_pos_color: wgpu::Buffer,
    pub filter_vertices: wgpu::Buffer,
    pub indices: wgpu::Buffer,
    pub indices_line: wgpu::Buffer,
    pub indices_line_rect: wgpu::Buffer,
}

impl Quad {
    pub fn new(device: &wgpu::Device) -> Self {
        let vertices_pos = [
            PosVertex {
                position: [0.0, 0.0],
            },
            PosVertex {
                position: [1.0, 0.0],
            },
            PosVertex {
                position: [1.0, 1.0],
            },
            PosVertex {
                position: [0.0, 1.0],
            },
        ];
        let vertices_pos_uv = [
            PosUvVertex {
                position: [0.0, 0.0],
                uv: [0.0, 0.0, 1.0],
            },
            PosUvVertex {
                position: [1.0, 0.0],
                uv: [1.0, 0.0, 1.0],
            },
            PosUvVertex {
                position: [1.0, 1.0],
                uv: [1.0, 1.0, 1.0],
            },
            PosUvVertex {
                position: [0.0, 1.0],
                uv: [0.0, 1.0, 1.0],
            },
        ];
        let vertices_pos_color = [
            PosColorVertex {
                position: [0.0, 0.0],
                color: [1.0, 1.0, 1.0, 1.0],
            },
            PosColorVertex {
                position: [1.0, 0.0],
                color: [1.0, 1.0, 1.0, 1.0],
            },
            PosColorVertex {
                position: [1.0, 1.0],
                color: [1.0, 1.0, 1.0, 1.0],
            },
            PosColorVertex {
                position: [0.0, 1.0],
                color: [1.0, 1.0, 1.0, 1.0],
            },
        ];
        let filter_vertices = [
            FilterVertex {
                position: [0.0, 0.0],
                uv: [0.0, 0.0],
            },
            FilterVertex {
                position: [1.0, 0.0],
                uv: [1.0, 0.0],
            },
            FilterVertex {
                position: [1.0, 1.0],
                uv: [1.0, 1.0],
            },
            FilterVertex {
                position: [0.0, 1.0],
                uv: [0.0, 1.0],
            },
        ];
        let indices: [u32; 6] = [0, 1, 2, 0, 2, 3];
        let indices_line: [u32; 2] = [0, 1];
        let indices_line_rect: [u32; 5] = [0, 1, 2, 3, 0];

        let vbo_pos = create_buffer_with_data(
            device,
            bytemuck::cast_slice(&vertices_pos),
            wgpu::BufferUsages::VERTEX,
            create_debug_label!("Quad vbo (pos)"),
        );

        let vbo_pos_uv = create_buffer_with_data(
            device,
            bytemuck::cast_slice(&vertices_pos_uv),
            wgpu::BufferUsages::VERTEX,
            create_debug_label!("Quad vbo (pos & uv)"),
        );

        let vbo_pos_color = create_buffer_with_data(
            device,
            bytemuck::cast_slice(&vertices_pos_color),
            wgpu::BufferUsages::VERTEX,
            create_debug_label!("Quad vbo (pos & color)"),
        );

        let vbo_filter = create_buffer_with_data(
            device,
            bytemuck::cast_slice(&filter_vertices),
            wgpu::BufferUsages::VERTEX,
            create_debug_label!("Quad vbo (filter)"),
        );

        let ibo = create_buffer_with_data(
            device,
            bytemuck::cast_slice(&indices),
            wgpu::BufferUsages::INDEX,
            create_debug_label!("Quad ibo"),
        );
        let ibo_line = create_buffer_with_data(
            device,
            bytemuck::cast_slice(&indices_line),
            wgpu::BufferUsages::INDEX,
            create_debug_label!("Line ibo"),
        );
        let ibo_line_rect = create_buffer_with_data(
            device,
            bytemuck::cast_slice(&indices_line_rect),
            wgpu::BufferUsages::INDEX,
            create_debug_label!("Line rect ibo"),
        );

        Self {
            vertices_pos: vbo_pos,
            vertices_pos_uv: vbo_pos_uv,
            vertices_pos_color: vbo_pos_color,
            filter_vertices: vbo_filter,
            indices: ibo,
            indices_line: ibo_line,
            indices_line_rect: ibo_line_rect,
        }
    }
}

#[cfg(all(test, windows))]
mod startup_tests {
    use super::Descriptors;
    use crate::backend::{WgpuRenderBackend, create_wgpu_instance, request_adapter_and_device};
    use crate::target::TextureTarget;
    use crate::utils::supported_sample_count;
    use ruffle_render::{backend::RenderBackend, commands::CommandList, quality::StageQuality};
    use std::sync::Arc;
    use swf::Color;

    #[test]
    #[ignore = "requires a DX12 adapter"]
    fn aola_first_frame_uses_only_the_selected_pipeline_set() {
        let backends = wgpu::Backends::DX12;
        let instance = create_wgpu_instance(backends, Default::default(), None);
        let (adapter, device, queue) = futures::executor::block_on(request_adapter_and_device(
            backends,
            &instance,
            None,
            wgpu::PowerPreference::LowPower,
        ))
        .unwrap();
        let descriptors = Arc::new(Descriptors::new(instance, adapter, device, queue));
        let target = TextureTarget::new(&descriptors.device, (8, 8)).unwrap();
        let format = target.format;
        let quality = StageQuality::High;
        let samples = supported_sample_count(&descriptors.adapter, quality.sample_count(), format);
        let mut renderer =
            WgpuRenderBackend::new_with_quality(descriptors.clone(), target, quality).unwrap();
        let assert_cached = || {
            let shape = descriptors.pipelines.lock().unwrap();
            assert_eq!(shape.len(), 1);
            assert!(shape.contains_key(&(samples, format)));
            let copy = descriptors.copy_pipeline.lock().unwrap();
            assert_eq!(copy.len(), 1);
            assert!(copy.contains_key(&(1, format)));
        };
        assert_cached();
        // PlayerBuilder applies the stage quality again before rendering.
        renderer.set_quality(quality);
        let clear = Color::from_rgb(0x5070b0, 255);
        renderer.submit_frame(clear, CommandList::new(), vec![]);
        let frame = renderer.capture_frame().unwrap();
        assert!(frame.pixels().all(|p| p.0 == [80, 112, 176, 255]));
        assert_cached();
        // A subsequent quality change can still acquire another set on demand.
        let native_count = || descriptors.device.get_internal_counters().hal.render_pipelines.read();
        let before = native_count();
        renderer.set_quality(StageQuality::Low);
        let sets = descriptors.pipelines.lock().unwrap();
        assert!(sets.contains_key(&(1, format)));
        println!(
            "startup samples={samples}, prepared stage sets=1, prepared copy pipelines=1, \
             additional native pipelines after requesting Low={}",
            native_count() - before
        );
    }
}
