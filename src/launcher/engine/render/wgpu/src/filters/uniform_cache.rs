//! Immutable, bounded filter parameters avoid overwriting a buffer between passes.
use crate::{Descriptors, create_buffer_with_data};
use fnv::FnvHashMap;
use std::sync::Mutex;

// Both the blur and glow shader parameter blocks contain exactly 32 bytes.
pub(super) struct FilterUniformCache {
    pub layout: wgpu::BindGroupLayout,
    capacity: usize,
    entries: Mutex<(u64, FnvHashMap<[u8; 32], (wgpu::BindGroup, u64)>)>,
}

impl FilterUniformCache {
    pub fn new(device: &wgpu::Device, capacity: usize) -> Self {
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: None,
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0, visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false, min_binding_size: wgpu::BufferSize::new(32) }, count: None,
            }],
        });
        Self { layout, capacity, entries: Default::default() }
    }

    pub fn bind(&self, descriptors: &Descriptors, buffer: &wgpu::Buffer) -> (wgpu::BindGroup, web_time::Duration) {
        let started = web_time::Instant::now();
        let group = descriptors.device.create_bind_group(&wgpu::BindGroupDescriptor { label: None, layout: &self.layout,
            entries: &[wgpu::BindGroupEntry { binding: 0, resource: buffer.as_entire_binding() }] });
        let elapsed = started.elapsed();
        descriptors.resource_metrics.record_work(|s| s.filter_bind_group_ns += elapsed.as_nanos() as u64);
        (group, elapsed)
    }

    pub fn get(&self, descriptors: &Descriptors, bytes: &[u8]) -> (wgpu::BindGroup, web_time::Duration) {
        let key: [u8; 32] = bytes.try_into().expect("Filter parameter block size");
        let mut cache = self.entries.lock().unwrap();
        cache.0 += 1;
        let access = cache.0;
        if let Some((buffer, last_used)) = cache.1.get_mut(&key) {
            *last_used = access;
            descriptors.resource_metrics.record_work(|s| s.filter_uniform_cache_hits += 1);
            return (buffer.clone(), web_time::Duration::ZERO);
        }
        let evicted = cache.1.len() == self.capacity;
        if evicted {
            let oldest = *cache.1.iter().min_by_key(|(_, (_, used))| *used).unwrap().0;
            // Encoded/in-flight commands retain their own wgpu references.
            // Never rewrite an evicted buffer with different parameter bytes.
            cache.1.remove(&oldest);
        }
        let buffer = create_buffer_with_data(&descriptors.device, bytes,
            wgpu::BufferUsages::UNIFORM, None);
        let (buffer, bind_time) = self.bind(descriptors, &buffer);
        cache.1.insert(key, (buffer.clone(), access));
        descriptors.resource_metrics.record_work(|s| {
            s.filter_uniform_cache_misses += 1;
            s.filter_uniform_cache_evictions += u64::from(evicted);
        });
        (buffer, bind_time)
    }
}

#[cfg(test)]
thread_local! { pub(crate) static FORCE_STREAMED_FILTERS: std::cell::Cell<bool> = const { std::cell::Cell::new(false) }; }

pub(super) fn streamed_reference() -> bool {
    #[cfg(test)]
    { FORCE_STREAMED_FILTERS.get() }
    #[cfg(not(test))]
    { false }
}
