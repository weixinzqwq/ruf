use ruffle_render::backend::{RenderResourceCategoryStats, RenderResourceStats, RenderWorkStats};
use std::sync::{Arc, Mutex};
use std::sync::atomic::{AtomicBool, Ordering};

pub(crate) fn allocation_bytes(device: &wgpu::Device, texture: &wgpu::Texture, fallback: u64) -> u64 {
    #[cfg(windows)]
    unsafe {
        if let (Some(device), Some(texture)) = (device.as_hal::<wgpu::hal::api::Dx12>(), texture.as_hal::<wgpu::hal::api::Dx12>()) {
            let description = texture.raw_resource().GetDesc();
            let size = device.raw_device().GetResourceAllocationInfo(0, &[description]).SizeInBytes;
            if size >= fallback && size < u64::MAX { return size; }
        }
    }
    let _ = (device, texture);
    fallback
}

#[derive(Clone, Copy, Debug)]
pub(crate) enum ResourceKind {
    UploadedBitmap, OffscreenBitmap, BitmapCache, TemporaryTexture, ReadbackBuffer,
    Mesh, MeshVertexBuffer, MeshIndexBuffer, MeshUniformBuffer, GradientTexture,
}

#[derive(Debug, Default)]
pub(crate) struct ResourceMetrics(Mutex<RenderResourceStats>);

fn category(stats: &mut RenderResourceStats, kind: ResourceKind) -> &mut RenderResourceCategoryStats {
    match kind {
        ResourceKind::UploadedBitmap => &mut stats.uploaded_bitmap,
        ResourceKind::OffscreenBitmap => &mut stats.offscreen_bitmap,
        ResourceKind::BitmapCache => &mut stats.bitmap_cache,
        ResourceKind::TemporaryTexture => &mut stats.temporary_texture,
        ResourceKind::ReadbackBuffer => &mut stats.readback_buffer,
        ResourceKind::Mesh => &mut stats.mesh,
        ResourceKind::MeshVertexBuffer => &mut stats.mesh_vertex_buffer,
        ResourceKind::MeshIndexBuffer => &mut stats.mesh_index_buffer,
        ResourceKind::MeshUniformBuffer => &mut stats.mesh_uniform_buffer,
        ResourceKind::GradientTexture => &mut stats.gradient_texture,
    }
}

impl ResourceMetrics {
    pub fn buffer_lease(self: &Arc<Self>, device: &wgpu::Device, kind: ResourceKind, buffer: &wgpu::Buffer, elapsed: std::time::Duration) -> ResourceLease {
        let bytes = buffer.size();
        let mut allocated = bytes;
        #[cfg(windows)]
        unsafe {
            if let (Some(device), Some(buffer)) = (device.as_hal::<wgpu::hal::api::Dx12>(), buffer.as_hal::<wgpu::hal::api::Dx12>()) {
                let size = device.raw_device().GetResourceAllocationInfo(0, &[buffer.raw_resource().GetDesc()]).SizeInBytes;
                if size >= bytes && size < u64::MAX { allocated = size; }
            }
        }
        let _ = device;
        self.texture_lease(kind, bytes, allocated, elapsed)
    }
    pub fn texture_lease(self: &Arc<Self>, kind: ResourceKind, bytes: u64, allocated: u64, elapsed: std::time::Duration) -> ResourceLease {
        let mut lease = self.timed_lease(kind, bytes, elapsed);
        let extra = allocated.saturating_sub(bytes);
        let mut snapshot = self.0.lock().unwrap();
        let stats = category(&mut snapshot, kind);
        stats.live_allocation_bytes += extra;
        stats.created_allocation_bytes += extra;
        stats.peak_allocation_bytes = stats.peak_allocation_bytes.max(stats.live_allocation_bytes);
        lease.allocation_bytes = allocated.max(bytes);
        lease
    }
    pub fn snapshot(&self) -> RenderResourceStats { *self.0.lock().unwrap() }

    pub fn record_work(&self, update: impl FnOnce(&mut RenderWorkStats)) {
        update(&mut self.0.lock().unwrap().work);
    }

    pub fn timed_lease(self: &Arc<Self>, kind: ResourceKind, bytes: u64, elapsed: std::time::Duration) -> ResourceLease {
        let lease = self.lease(kind, bytes);
        let mut snapshot = self.0.lock().unwrap();
        let stats = category(&mut snapshot, kind);
        let ns = elapsed.as_nanos().min(u128::from(u64::MAX)) as u64;
        stats.timed_creation_count += 1;
        stats.creation_cpu_ns = stats.creation_cpu_ns.saturating_add(ns);
        stats.max_creation_cpu_ns = stats.max_creation_cpu_ns.max(ns);
        lease
    }

    pub fn pool_request(&self, reused: bool, evicted: u64) {
        let mut stats = self.0.lock().unwrap();
        if reused { stats.temporary_pool_hits += 1; } else { stats.temporary_pool_misses += 1; }
        stats.temporary_pool_pressure_evictions += evicted;
    }

    pub fn pool_recurrent_miss(&self) { self.0.lock().unwrap().temporary_pool_recurrent_misses += 1; }

    pub fn lease(self: &Arc<Self>, kind: ResourceKind, bytes: u64) -> ResourceLease {
        let mut snapshot = self.0.lock().unwrap();
        let stats = category(&mut snapshot, kind);
        stats.live_count += 1;
        stats.live_wrapper_count += 1;
        stats.live_allocation_bytes += bytes;
        stats.created_allocation_bytes += bytes;
        stats.peak_allocation_bytes = stats.peak_allocation_bytes.max(stats.live_allocation_bytes);
        stats.live_estimated_bytes += bytes;
        stats.peak_count = stats.peak_count.max(stats.live_count);
        stats.peak_estimated_bytes = stats.peak_estimated_bytes.max(stats.live_estimated_bytes);
        stats.created_count += 1;
        stats.created_estimated_bytes += bytes;
        ResourceLease { metrics: self.clone(), kind, bytes, allocation_bytes: bytes, pool_reusable: false, wrapper_alive: AtomicBool::new(true) }
    }

    pub fn permit_cache(&self, bytes: u64, budget: u64) -> bool {
        let mut stats = self.0.lock().unwrap();
        if bytes > budget.saturating_sub(stats.bitmap_cache.live_allocation_bytes) {
            stats.cache_budget_rejected_count += 1;
            stats.cache_budget_rejected_estimated_bytes = stats.cache_budget_rejected_estimated_bytes.saturating_add(bytes);
            false
        } else { true }
    }

    pub fn cache_budget_rejected(&self, bytes: u64) {
        let mut stats = self.0.lock().unwrap();
        stats.cache_budget_rejected_count += 1;
        stats.cache_budget_rejected_estimated_bytes = stats.cache_budget_rejected_estimated_bytes.saturating_add(bytes);
    }

    #[cfg(not(target_family = "wasm"))]
    pub fn cache_allocation_failed(&self, bytes: u64) {
        let mut stats = self.0.lock().unwrap();
        stats.cache_allocation_failed_count += 1;
        stats.cache_allocation_failed_estimated_bytes = stats.cache_allocation_failed_estimated_bytes.saturating_add(bytes);
    }
}

/// Owned by the allocation wrapper, and moved with its pool item. Not Clone.
#[derive(Debug)]
pub(crate) struct ResourceLease {
    metrics: Arc<ResourceMetrics>,
    kind: ResourceKind,
    bytes: u64,
    pub allocation_bytes: u64,
    pub pool_reusable: bool,
    wrapper_alive: AtomicBool,
}

impl ResourceLease {
    pub fn wrapper_released(&self) {
        if self.wrapper_alive.swap(false, Ordering::Relaxed) {
            let mut stats = self.metrics.0.lock().unwrap();
            let stats = category(&mut stats, self.kind);
            stats.live_wrapper_count -= 1;
            stats.binding_only_allocation_bytes += self.allocation_bytes;
        }
    }
    pub fn idle_return_rejected(&self, probation: bool) {
        let mut stats = self.metrics.0.lock().unwrap();
        stats.temporary_pool_return_rejections += 1;
        if probation { stats.temporary_pool_probation_rejections += 1; }
    }
}

impl Drop for ResourceLease {
    fn drop(&mut self) {
        let mut snapshot = self.metrics.0.lock().unwrap();
        let stats = category(&mut snapshot, self.kind);
        stats.live_count -= 1;
        stats.live_allocation_bytes -= self.allocation_bytes;
        stats.released_allocation_bytes += self.allocation_bytes;
        if self.wrapper_alive.load(Ordering::Relaxed) { stats.live_wrapper_count -= 1; }
        else { stats.binding_only_allocation_bytes -= self.allocation_bytes; }
        stats.live_estimated_bytes -= self.bytes;
        stats.released_count += 1;
        stats.released_estimated_bytes += self.bytes;
    }
}

/// Binding groups can outlive the BitmapHandle through a shape mesh. Keep the
/// accounting lease, never the Texture wrapper (which itself caches bindings).
#[derive(Debug)]
pub(crate) struct BindingLease(Arc<ResourceLease>);
impl BindingLease {
    pub fn new(lease: Arc<ResourceLease>) -> Self {
        category(&mut lease.metrics.0.lock().unwrap(), lease.kind).binding_references += 1;
        Self(lease)
    }
}
impl Clone for BindingLease { fn clone(&self) -> Self { Self::new(self.0.clone()) } }
impl Drop for BindingLease {
    fn drop(&mut self) { category(&mut self.0.metrics.0.lock().unwrap(), self.0.kind).binding_references -= 1; }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn native_charge_and_binding_only_ownership_survive_wrapper_release() {
        let metrics = Arc::new(ResourceMetrics::default());
        let owner = Arc::new(metrics.texture_lease(ResourceKind::UploadedBitmap, 16, 65536, std::time::Duration::ZERO));
        let binding = BindingLease::new(owner.clone());
        let cloned = binding.clone();
        owner.wrapper_released();
        drop(owner);
        let stats = metrics.snapshot().uploaded_bitmap;
        assert_eq!((stats.live_count, stats.live_wrapper_count, stats.binding_references), (1, 0, 2));
        assert_eq!((stats.live_estimated_bytes, stats.live_allocation_bytes, stats.binding_only_allocation_bytes), (16, 65536, 65536));
        drop(binding); drop(cloned);
        let stats = metrics.snapshot().uploaded_bitmap;
        assert_eq!((stats.live_count, stats.binding_references, stats.binding_only_allocation_bytes), (0, 0, 0));
        assert_eq!(stats.created_allocation_bytes, stats.released_allocation_bytes);
    }

    #[test]
    fn allocation_timing_and_pool_reuse_are_separate_from_live_bytes() {
        let metrics = Arc::new(ResourceMetrics::default());
        let first = metrics.timed_lease(ResourceKind::TemporaryTexture, 64, std::time::Duration::from_micros(300));
        let second = metrics.timed_lease(ResourceKind::TemporaryTexture, 32, std::time::Duration::from_micros(100));
        metrics.pool_request(false, 1);
        metrics.pool_request(true, 0);
        drop(first);
        drop(second);
        let stats = metrics.snapshot();
        assert_eq!(stats.temporary_texture.creation_cpu_ns, 400_000);
        assert_eq!(stats.temporary_texture.max_creation_cpu_ns, 300_000);
        assert_eq!(stats.temporary_texture.timed_creation_count, 2);
        assert_eq!(stats.temporary_texture.live_estimated_bytes, 0);
        assert_eq!((stats.temporary_pool_hits, stats.temporary_pool_misses, stats.temporary_pool_pressure_evictions), (1, 1, 1));
    }

    #[test]
    fn clones_of_resource_owner_do_not_double_count_and_last_drop_releases() {
        let metrics = Arc::new(ResourceMetrics::default());
        let owner = Arc::new(metrics.lease(ResourceKind::UploadedBitmap, 48));
        let reference = owner.clone();
        drop(owner);
        assert_eq!(metrics.snapshot().uploaded_bitmap.live_count, 1);
        drop(reference);
        let stats = metrics.snapshot().uploaded_bitmap;
        assert_eq!((stats.created_count, stats.released_count), (1, 1));
        assert_eq!((stats.live_count, stats.live_estimated_bytes), (0, 0));
        assert_eq!((stats.peak_count, stats.peak_estimated_bytes), (1, 48));
        assert_eq!(stats.created_estimated_bytes, stats.released_estimated_bytes);
    }

    #[test]
    fn optional_cache_refusal_never_releases_existing_resources() {
        let metrics = Arc::new(ResourceMetrics::default());
        let live = metrics.lease(ResourceKind::BitmapCache, 80);
        assert!(!metrics.permit_cache(32, 100));
        let stats = metrics.snapshot();
        assert_eq!(stats.bitmap_cache.live_estimated_bytes, 80);
        assert_eq!(stats.cache_budget_rejected_count, 1);
        assert_eq!(stats.cache_budget_rejected_estimated_bytes, 32);
        drop(live);
        assert!(metrics.permit_cache(32, 100));
    }

    #[test]
    fn bitmapdata_is_not_charged_to_the_optional_cache_budget() {
        let metrics = Arc::new(ResourceMetrics::default());
        let _uploaded = metrics.lease(ResourceKind::UploadedBitmap, 1000);
        let _offscreen = metrics.lease(ResourceKind::OffscreenBitmap, 2000);
        assert!(metrics.permit_cache(80, 100));
        assert!(!metrics.permit_cache(1, 0));
        let stats = metrics.snapshot();
        assert_eq!(stats.uploaded_bitmap.live_estimated_bytes, 1000);
        assert_eq!(stats.offscreen_bitmap.live_estimated_bytes, 2000);
        assert_eq!(stats.bitmap_cache.created_count, 0);
    }

    #[test]
    #[cfg(not(target_family = "wasm"))]
    fn allocation_failure_is_not_counted_as_a_live_allocation() {
        let metrics = Arc::new(ResourceMetrics::default());
        metrics.cache_allocation_failed(4096);
        let stats = metrics.snapshot();
        assert_eq!(stats.cache_allocation_failed_count, 1);
        assert_eq!(stats.cache_allocation_failed_estimated_bytes, 4096);
        assert_eq!(stats.bitmap_cache.created_count, 0);
        assert_eq!(stats.bitmap_cache.live_estimated_bytes, 0);
        assert_eq!(stats.cache_budget_rejected_count, 0);
    }
}
