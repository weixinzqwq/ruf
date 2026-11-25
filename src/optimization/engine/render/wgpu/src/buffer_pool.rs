use crate::descriptors::Descriptors;
use crate::aoqi::{BitmapUpscale, TexturePoolLimits};
use crate::globals::Globals;
use crate::resource_metrics::{ResourceKind, ResourceLease};
use std::sync::atomic::{AtomicU64, Ordering};
use fnv::FnvHashMap;
use std::collections::BTreeMap;
use std::fmt::{Debug, Formatter};
use std::ops::Deref;
use std::sync::{Arc, Mutex, OnceLock, Weak};

pub(crate) fn battle_texture_pool_enabled() -> bool {
    static ENABLED: OnceLock<bool> = OnceLock::new();
    *ENABLED.get_or_init(|| std::env::var("AOLA_BATTLE_TEXTURE_POOL").as_deref() == Ok("1"))
}

type PoolInner<T, D> = Mutex<PoolStorage<T, D>>;
type Constructor<Type, Description> = Box<dyn Fn(&Descriptors, &Description) -> Type>;

pub type PooledTexture = (wgpu::Texture, wgpu::TextureView, BlurBindings);

/// Texture-only filter bindings live with their own pooled allocation.
/// No foreign texture or mutable parameter buffer is retained here.
#[derive(Debug, Default)]
pub struct BlurBindings([std::sync::OnceLock<wgpu::BindGroup>; 2]);

impl BlurBindings {
    pub fn get(&self, descriptors: &Descriptors, kind: usize,
        create: impl FnOnce() -> wgpu::BindGroup) -> wgpu::BindGroup {
        if let Some(group) = self.0[kind].get() {
            descriptors.resource_metrics.record_work(|s| s.filter_bind_group_reuses += 1);
            return group.clone();
        }
        self.0[kind].get_or_init(create).clone()
    }
}

struct PoolItem<T, D> {
    item: T,
    description: D,
    _resource_lease: Option<ResourceLease>,
    idle_reservation: Option<IdleReservation>,
}

/// Shared by all temporary texture buckets in both render pools. Reservation
/// exists only while an item is idle, and is released on take/drop/eviction.
#[derive(Debug)]
pub(crate) struct IdleBudget {
    used: AtomicU64,
    limit: AtomicU64,
}

impl Default for IdleBudget {
    fn default() -> Self {
        Self { used: AtomicU64::new(0), limit: AtomicU64::new(TexturePoolLimits::default().idle_bytes) }
    }
}

impl IdleBudget {
    fn reserve(self: &Arc<Self>, bytes: u64) -> Option<IdleReservation> {
        self.used.fetch_update(Ordering::Relaxed, Ordering::Relaxed, |used| {
            used.checked_add(bytes).filter(|total| *total <= self.limit.load(Ordering::Relaxed))
        }).ok()?;
        Some(IdleReservation { budget: self.clone(), bytes })
    }
}

struct IdleReservation { budget: Arc<IdleBudget>, bytes: u64 }
impl Drop for IdleReservation {
    fn drop(&mut self) { self.budget.used.fetch_sub(self.bytes, Ordering::Relaxed); }
}

struct PoolStorage<T, D> {
    items: Vec<PoolItem<T, D>>,
    idle_budget: Option<(u64, fn(&D) -> u64)>,
    shared_idle_budget: Option<(Arc<IdleBudget>, u64)>,
    idle_notice: Option<(Arc<Mutex<Vec<TextureKey>>>, TextureKey)>,
}

impl<T, D> PoolStorage<T, D> {
    fn take_compatible(&mut self, description: &D) -> Option<PoolItem<T, D>>
    where
        D: BufferDescription,
    {
        let best = self.items.iter().enumerate().filter_map(|(i, item)| {
            description.cost_to_use(&item.description).map(|cost| (cost, i))
        }).min_by(|(left, _), (right, _)| left.cmp(right));
        let (_, index) = best?;
        let mut item = self.items.swap_remove(index);
        item.idle_reservation.take();
        Some(item)
    }

    fn trim(&mut self, incoming: u64) {
        let Some((budget, size)) = self.idle_budget else { return; };
        let mut bytes = self.items.iter().fold(0u64, |sum, item| sum.saturating_add(size(&item.description)));
        while bytes.saturating_add(incoming) > budget && !self.items.is_empty() {
            let largest = self.items.iter().enumerate().max_by_key(|(_, item)| size(&item.description)).unwrap().0;
            let item = self.items.swap_remove(largest);
            bytes = bytes.saturating_sub(size(&item.description));
        }
    }

    fn put(&mut self, mut item: PoolItem<T, D>) {
        let bytes = self.idle_budget.map_or(0, |(_, size)| size(&item.description));
        if self.idle_budget.is_some_and(|(budget, _)| bytes > budget) { return; }
        self.trim(bytes);
        if let Some((budget, estimated_bytes)) = &self.shared_idle_budget {
            // All sizes share the native-byte cap. Cold owners make room on a miss;
            // current/previous-frame owners remain protected from one-use noise.
            let probation = item._resource_lease.as_ref().is_some_and(|lease| !lease.pool_reusable);
            let bytes = item._resource_lease.as_ref().map_or(*estimated_bytes, |lease| lease.allocation_bytes);
            let Some(reservation) = budget.reserve(bytes) else {
                if let Some(lease) = &item._resource_lease { lease.idle_return_rejected(probation); }
                return;
            };
            item.idle_reservation = Some(reservation);
        }
        if self.items.is_empty() && let Some((returns, key)) = &self.idle_notice {
            returns.lock().unwrap().push(*key);
        }
        self.items.push(item);
    }
}

#[derive(Debug, Default)]
pub struct TexturePool {
    pools: Arc<Mutex<FnvHashMap<TextureKey, TextureBucket>>>,
    idle_index: Arc<Mutex<IdleIndex>>,
    globals_cache: FnvHashMap<GlobalsKey, (Arc<Globals>, u64)>,
    bitmap_upscale: BitmapUpscale,
    frame: Arc<AtomicU64>,
    access: Arc<AtomicU64>,
    limits: TexturePoolLimits,
    idle_budget: Arc<IdleBudget>,
}

#[derive(Debug)]
struct TextureBucket<T = PooledTexture, D: BufferDescription = AlwaysCompatible> {
    pool: BufferPool<T, D>,
    last_used: u64,
    last_access: u64,
}

/// Pressure eviction indexes only nonempty buckets. Age tracking also covers
/// empty buckets. Late returns notify the pressure index without taking the
/// pool-map lock.
#[derive(Debug, Default)]
struct IdleIndex {
    by_access: [BTreeMap<u64, TextureKey>; 2],
    // One entry per bucket, including empty buckets with outstanding textures.
    // Hits update the bucket itself. Recheck only when a trim or expiry is due.
    by_frame: BTreeMap<u64, Vec<TextureKey>>,
    returns: Arc<Mutex<Vec<TextureKey>>>,
    pending: Vec<TextureKey>,
}

impl IdleIndex {
    fn expire<T, D: BufferDescription>(
        &mut self, pools: &mut FnvHashMap<TextureKey, TextureBucket<T, D>>,
        frame: u64, idle_frames: u64, long_run: bool,
    ) {
        // As in pressure eviction, protect the current and previous frame's working set.
        let check_age = if long_run { idle_frames.min(2) } else { idle_frames };
        let expiry_age = if long_run { idle_frames.min(31) } else { idle_frames };
        while let Some((&oldest, _)) = self.by_frame.first_key_value() {
            if frame.saturating_sub(oldest) < check_age {
                break;
            }
            let (_, keys) = self.by_frame.pop_first().unwrap();
            for key in keys {
                let bucket = &pools[&key];
                let age = frame.saturating_sub(bucket.last_used);
                let next = if age < check_age {
                    bucket.last_used
                } else if age < expiry_age {
                    // Blur alternates two renderable single-sample RGBA targets of
                    // the same size. Keep the pair across short animation gaps.
                    let keep = if battle_texture_pool_enabled() && key.is_filter_target() { 2 } else { 1 };
                    bucket.pool.available.lock().unwrap().items.truncate(keep);
                    // The next check is due at last_used + expiry_age.
                    bucket.last_used + (expiry_age - check_age)
                } else {
                    self.remove(key, bucket.last_access);
                    // Each checked-out PoolEntry owns a Weak to this storage. Retain its
                    // return path while expiring idle siblings. A zero age disables retention.
                    if long_run && check_age != 0 && Arc::weak_count(&bucket.pool.available) > 0 {
                        bucket.pool.clear_available();
                        frame + (expiry_age - check_age)
                    } else {
                        pools.remove(&key);
                        continue;
                    }
                };
                self.by_frame.entry(next).or_default().push(key);
            }
        }
    }

    fn remove(&mut self, key: TextureKey, access: u64) {
        self.by_access[usize::from(key.sample_count == 1)].remove(&access);
    }

    fn sync<T, D: BufferDescription>(&mut self, key: TextureKey, bucket: &TextureBucket<T, D>) {
        self.remove(key, bucket.last_access);
        if !bucket.pool.available.lock().unwrap().items.is_empty() {
            self.by_access[usize::from(key.sample_count == 1)].insert(bucket.last_access, key);
        }
    }

    fn drain_returns<T, D: BufferDescription>(&mut self, pools: &FnvHashMap<TextureKey, TextureBucket<T, D>>) {
        // Never hold the return-queue lock while locking a bucket's storage.
        std::mem::swap(&mut self.pending, &mut *self.returns.lock().unwrap());
        while let Some(key) = self.pending.pop() {
            if let Some(bucket) = pools.get(&key) { self.sync(key, bucket); }
        }
    }
}

fn evict_to_fit<T, D: BufferDescription>(
    pools: &FnvHashMap<TextureKey, TextureBucket<T, D>>, index: &mut IdleIndex,
    budget: &IdleBudget, bytes: u64, request: Option<(TextureKey, u64)>,
) -> (u64, u64) {
    let limit = budget.limit.load(Ordering::Relaxed);
    let mut evicted = 0;
    let mut visited = 0;
    if bytes > limit { return (0, 0); }
    // Preserve the existing cold-MSAA-first policy. Access order also orders
    // last-used frames, so a recent oldest entry ends this class immediately.
    // A new MSAA dimension can displace a cold MSAA owner, but do not trade
    // many small single-sample targets for it until its key is requested again.
    let new_msaa = battle_texture_pool_enabled() && request.is_some_and(|(key, _)| {
        key.sample_count > 1 && !pools.contains_key(&key)
    });
    let classes = if new_msaa { 1 } else { 2 };
    for kind in 0..classes {
        while budget.used.load(Ordering::Relaxed).saturating_add(bytes) > limit {
            let Some((&access, &key)) = index.by_access[kind].first_key_value() else { break; };
            visited += 1;
            let bucket = &pools[&key];
            if let Some((requested, frame)) = request {
                if key == requested {
                    index.by_access[kind].remove(&access);
                    continue;
                }
                if frame.saturating_sub(bucket.last_used) <= 1 { break; }
            }
            let mut available = bucket.pool.available.lock().unwrap();
            while budget.used.load(Ordering::Relaxed).saturating_add(bytes) > limit
                && !available.items.is_empty()
            {
                available.items.pop();
                evicted += 1;
            }
            if available.items.is_empty() { index.by_access[kind].remove(&access); }
        }
    }
    (evicted, visited)
}

impl TexturePool {
    pub(crate) fn idle_limit(&self) -> u64 { self.idle_budget.limit.load(Ordering::Relaxed) }
    pub(crate) fn idle_stats(&self) -> (u64, u64, u64) {
        let pools = self.pools.lock().unwrap();
        (self.idle_budget.used.load(Ordering::Relaxed),
         pools.values().map(|bucket| bucket.pool.available.lock().unwrap().items.len() as u64).sum(), pools.len() as u64)
    }
    pub fn new() -> Self {
        Default::default()
    }

    pub(crate) fn sharing_with(other: &Self) -> Self {
        Self { pools: other.pools.clone(), idle_index: other.idle_index.clone(), frame: other.frame.clone(), access: other.access.clone(),
            idle_budget: other.idle_budget.clone(), ..Self::default() }
    }

    pub fn clear(&mut self) {
        self.pools.lock().unwrap().clear();
        let mut index = self.idle_index.lock().unwrap();
        index.by_access.iter_mut().for_each(BTreeMap::clear);
        index.by_frame.clear();
        index.returns.lock().unwrap().clear();
        self.globals_cache.clear();
    }

    pub fn set_bitmap_upscale(&mut self, mode: BitmapUpscale) {
        if self.bitmap_upscale != mode {
            self.bitmap_upscale = mode;
            self.globals_cache.clear();
        }
    }

    pub fn set_limits(&mut self, limits: TexturePoolLimits) {
        let age_changed = self.limits.idle_frames != limits.idle_frames
            && ruffle_render::backend::aola_long_run_maintenance();
        self.limits = limits;
        self.idle_budget.limit.store(limits.idle_bytes, Ordering::Relaxed);
        if age_changed || self.idle_budget.used.load(Ordering::Relaxed) > limits.idle_bytes {
            let pools = self.pools.lock().unwrap();
            let mut index = self.idle_index.lock().unwrap();
            if age_changed {
                // Existing second-stage checks were scheduled for the previous limits.
                index.by_frame.clear();
                for (&key, bucket) in pools.iter() {
                    index.by_frame.entry(bucket.last_used).or_default().push(key);
                }
            }
            index.drain_returns(&pools);
            evict_to_fit(&pools, &mut index, &self.idle_budget, 0, None);
        }
    }

    /// Called after submission. Eviction drops only the pool's idle ownership;
    /// live PoolEntry values keep their textures and are not destroyed.
    pub fn end_frame(&mut self) {
        let frame = self.frame.load(Ordering::Relaxed);
        let mut pools = self.pools.lock().unwrap();
        self.idle_index.lock().unwrap().expire(&mut pools, frame, self.limits.idle_frames,
            ruffle_render::backend::aola_long_run_maintenance());
        drop(pools);
        self.prune_globals();
        self.frame.fetch_add(1, Ordering::Relaxed);
    }

    pub(crate) fn prune_globals(&mut self) {
        let frame = self.frame.load(Ordering::Relaxed);
        // Keep recently used viewport buffers so a stable viewport does not
        // allocate a uniform buffer every frame. In-use targets keep their Arc.
        self.globals_cache.retain(|_, (globals, last_used)| {
            Arc::strong_count(globals) > 1
                || frame.saturating_sub(*last_used) < self.limits.idle_frames
        });
    }

    pub fn get_texture(
        &mut self,
        descriptors: &Descriptors,
        size: wgpu::Extent3d,
        usage: wgpu::TextureUsages,
        format: wgpu::TextureFormat,
        sample_count: u32,
    ) -> PoolEntry<PooledTexture, AlwaysCompatible> {
        let lookup_started = web_time::Instant::now();
        let key = TextureKey {
            size,
            usage,
            format,
            sample_count,
        };
        let frame = self.frame.load(Ordering::Relaxed);
        let access = self.access.fetch_add(1, Ordering::Relaxed);
        let shared_budget = self.idle_budget.clone();
        let metrics = descriptors.resource_metrics.clone();
        let mut pools = self.pools.lock().unwrap();
        let mut index = self.idle_index.lock().unwrap();
        index.drain_returns(&pools);
        if let Some(bucket) = pools.get(&key) { index.remove(key, bucket.last_access); }
        let reused = pools.get(&key).is_some_and(|bucket| !bucket.pool.available.lock().unwrap().items.is_empty());
        let seen = pools.contains_key(&key);
        // Retire cold idle owners across both render paths; live owners survive.
        let alignment = if descriptors.backend == wgpu::Backend::Dx12 { if sample_count > 1 { 4 * 1024 * 1024 } else { 65536 } } else { 1 };
        let charge = pools.get(&key).and_then(|bucket| bucket.pool.available.lock().unwrap().shared_idle_budget.as_ref().map(|(_, bytes)| *bytes))
            .unwrap_or_else(|| key.estimated_bytes().div_ceil(alignment) * alignment);
        // New and recurring sizes can displace cold owners, never recent ones.
        // This avoids starving a changing animation behind long-idle sizes.
        let (evicted, visited) = if reused { (0, 0) } else {
            evict_to_fit(&pools, &mut index, &shared_budget, charge, Some((key, frame)))
        };
        descriptors.resource_metrics.pool_request(reused, evicted);
        if seen && !reused { descriptors.resource_metrics.pool_recurrent_miss(); }
        let bucket = pools.entry(key).or_insert_with(|| {
            let label = if cfg!(feature = "render_debug_labels") {
                use std::sync::atomic::{AtomicU32, Ordering};
                static ID_COUNT: AtomicU32 = AtomicU32::new(0);
                let id = ID_COUNT.fetch_add(1, Ordering::Relaxed);
                create_debug_label!("Pooled texture {}", id)
            } else {
                None
            };
            let pool = BufferPool::new(Box::new(move |descriptors, _description| {
                let texture = descriptors.device.create_texture(&wgpu::TextureDescriptor {
                    label: label.as_deref(),
                    size,
                    mip_level_count: 1,
                    sample_count,
                    dimension: wgpu::TextureDimension::D2,
                    format,
                    view_formats: &[format],
                    usage,
                });
                let view = texture.create_view(&Default::default());
                (texture, view, BlurBindings::default())
            }));
            let device = descriptors.device.clone();
            let pool = pool.with_tracking(Box::new(move |texture: &PooledTexture, _, elapsed| {
                let kind = key.metric_kind();
                metrics.record_work(|s| {
                    s.temporary_created[kind] += 1;
                    s.temporary_created_texels[kind] += u64::from(key.size.width) * u64::from(key.size.height);
                    s.temporary_created_bytes[kind] += key.estimated_bytes();
                    s.temporary_creation_ns[kind] += elapsed.as_nanos() as u64;
                });
                let allocated = crate::resource_metrics::allocation_bytes(&device, &texture.0, key.estimated_bytes());
                metrics.texture_lease(ResourceKind::TemporaryTexture, key.estimated_bytes(), allocated, elapsed)
            }));
            {
                let mut available = pool.available.lock().unwrap();
                available.shared_idle_budget = Some((shared_budget, charge));
                available.idle_notice = Some((index.returns.clone(), key));
            }
            index.by_frame.entry(frame).or_default().push(key);
            TextureBucket { pool, last_used: frame, last_access: access }
        });
        bucket.last_used = frame;
        bucket.last_access = access;
        descriptors.resource_metrics.record_work(|s| {
            s.temporary_pool_lookup_ns += lookup_started.elapsed().as_nanos() as u64;
            s.pool_pressure_candidates += visited;
            let texels = u64::from(size.width) * u64::from(size.height);
            let bin = match texels { 0..=4096 => 0, 4097..=65536 => 1, 65537..=262144 => 2, 262145..=839999 => 3, _ => 4 };
            s.temporary_size_requests[bin] += 1;
            if !reused {
                let misses = if seen { &mut s.temporary_pool_empty_bucket } else { &mut s.temporary_pool_missing_bucket };
                misses[key.metric_kind()][bin] += 1;
            }
        });
        let mut entry = bucket.pool.take(descriptors, AlwaysCompatible);
        index.sync(key, bucket);
        if let Some(lease) = entry.item.as_mut().and_then(|item| item._resource_lease.as_mut()) {
            lease.pool_reusable |= seen;
        }
        entry
    }

    pub fn get_globals(
        &mut self,
        descriptors: &Descriptors,
        viewport_width: u32,
        viewport_height: u32,
    ) -> Arc<Globals> {
        let frame = self.frame.load(Ordering::Relaxed);
        let bitmap_upscale = self.bitmap_upscale;
        let entry = self.globals_cache
            .entry(GlobalsKey {
                viewport_width,
                viewport_height,
            })
            .or_insert_with(|| {
                (Arc::new(Globals::new(
                    &descriptors.device,
                    &descriptors.bind_layouts.globals,
                    viewport_width,
                    viewport_height,
                    bitmap_upscale,
                )), frame)
            });
        entry.1 = frame;
        entry.0.clone()
    }
}

#[derive(Copy, Clone, Debug, Hash, Eq, PartialEq)]
struct TextureKey {
    size: wgpu::Extent3d,
    usage: wgpu::TextureUsages,
    format: wgpu::TextureFormat,
    sample_count: u32,
}

impl TextureKey {
    fn is_filter_target(&self) -> bool {
        self.sample_count == 1 && self.format == wgpu::TextureFormat::Rgba8Unorm
            && self.usage == wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING
                | wgpu::TextureUsages::COPY_SRC | wgpu::TextureUsages::COPY_DST
    }

    fn metric_kind(&self) -> usize {
        match self.format {
            wgpu::TextureFormat::Stencil8 | wgpu::TextureFormat::Depth24PlusStencil8 => 2,
            wgpu::TextureFormat::Rgba8Unorm | wgpu::TextureFormat::Rgba8UnormSrgb
            | wgpu::TextureFormat::Bgra8Unorm | wgpu::TextureFormat::Bgra8UnormSrgb
            | wgpu::TextureFormat::Rgba16Float => usize::from(self.sample_count == 1),
            _ => 3,
        }
    }

    fn estimated_bytes(&self) -> u64 {
        // All currently pooled targets use these color/stencil formats.
        // Treat unknown formats conservatively as 16 bytes per texel.
        let bytes_per_texel = match self.format {
            wgpu::TextureFormat::R8Unorm | wgpu::TextureFormat::Stencil8 => 1,
            wgpu::TextureFormat::Rgba8Unorm | wgpu::TextureFormat::Rgba8UnormSrgb
            | wgpu::TextureFormat::Bgra8Unorm | wgpu::TextureFormat::Bgra8UnormSrgb
            | wgpu::TextureFormat::Depth24PlusStencil8 => 4,
            wgpu::TextureFormat::Rgba16Float => 8,
            _ => 16,
        };
        u64::from(self.size.width)
            .saturating_mul(u64::from(self.size.height))
            .saturating_mul(u64::from(self.size.depth_or_array_layers))
            .saturating_mul(u64::from(self.sample_count))
            .saturating_mul(bytes_per_texel)
    }
}

#[derive(Copy, Clone, Debug, Hash, Eq, PartialEq)]
struct GlobalsKey {
    viewport_width: u32,
    viewport_height: u32,
}

pub trait BufferDescription: Clone + Debug {
    type Cost: Ord;

    /// If the potential buffer represented by this description (`self`)
    /// fits another existing buffer and its description (`other`),
    /// return the cost to use that buffer instead of making a new one.
    ///
    /// Cost is an arbitrary unit, but lower is better.
    /// None means that the other buffer cannot be used in place of this one.
    fn cost_to_use(&self, other: &Self) -> Option<Self::Cost>;
}

#[derive(Clone, Debug)]
pub struct AlwaysCompatible;

impl BufferDescription for AlwaysCompatible {
    type Cost = ();

    fn cost_to_use(&self, _other: &Self) -> Option<()> {
        Some(())
    }
}

pub struct BufferPool<Type, Description: BufferDescription> {
    available: Arc<PoolInner<Type, Description>>,
    constructor: Constructor<Type, Description>,
    lease_factory: Option<Box<dyn Fn(&Type, &Description, std::time::Duration) -> ResourceLease>>,
}

impl<Type, Description: BufferDescription> Debug for BufferPool<Type, Description> {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("BufferPool").finish()
    }
}

impl<Type, Description: BufferDescription> BufferPool<Type, Description> {
    pub fn new(constructor: Constructor<Type, Description>) -> Self {
        Self {
            available: Arc::new(Mutex::new(PoolStorage { items: Vec::new(), idle_budget: None, shared_idle_budget: None, idle_notice: None })),
            constructor,
            lease_factory: None,
        }
    }

    /// Bound resources as soon as they are returned, including returns made
    /// through a SyncHandle outside submit_frame. Live entries are untouched.
    pub fn with_idle_budget(
        constructor: Constructor<Type, Description>,
        budget: u64,
        size: fn(&Description) -> u64,
    ) -> Self {
        Self {
            available: Arc::new(Mutex::new(PoolStorage {
                items: Vec::new(),
                idle_budget: Some((budget, size)),
                shared_idle_budget: None, idle_notice: None,
            })),
            constructor,
            lease_factory: None,
        }
    }

    pub(crate) fn with_tracking(mut self, factory: Box<dyn Fn(&Type, &Description, std::time::Duration) -> ResourceLease>) -> Self {
        self.lease_factory = Some(factory);
        self
    }

    pub fn clear_available(&self) {
        self.available.lock().expect("Should not be able to lock recursively").items.clear();
    }

    pub fn take(
        &self,
        descriptors: &Descriptors,
        description: Description,
    ) -> PoolEntry<Type, Description> {
        let mut guard = self
            .available
            .lock()
            .expect("Should not be able to lock recursively");
        let item = if let Some(item) = guard.take_compatible(&description) {
            item
        } else {
            let started = web_time::Instant::now();
            let item = (self.constructor)(descriptors, &description);
            let resource_lease = self.lease_factory.as_ref().map(|factory| factory(&item, &description, started.elapsed()));
            if let (Some((_, bytes)), Some(lease)) = (&mut guard.shared_idle_budget, &resource_lease) { *bytes = lease.allocation_bytes; }
            PoolItem { item, description, _resource_lease: resource_lease, idle_reservation: None }
        };
        PoolEntry {
            item: Some(item),
            pool: Arc::downgrade(&self.available),
        }
    }
}

pub struct PoolEntry<Type, Description: BufferDescription> {
    item: Option<PoolItem<Type, Description>>,
    pool: Weak<PoolInner<Type, Description>>,
}

impl<Type, Description: BufferDescription> Debug for PoolEntry<Type, Description>
where
    Type: Debug,
{
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.debug_tuple("PoolEntry").field(&self.item.as_ref().map(|item| &item.item)).finish()
    }
}

impl<Type, Description: BufferDescription> Drop for PoolEntry<Type, Description> {
    fn drop(&mut self) {
        if let Some(item) = self.item.take()
            && let Some(pool) = self.pool.upgrade()
        {
            pool.lock()
                .expect("Should not be able to lock recursively")
                .put(item)
        }
    }
}

impl<Type, Description: BufferDescription> Deref for PoolEntry<Type, Description> {
    type Target = Type;

    fn deref(&self) -> &Self::Target {
        &self.item.as_ref().expect("Item should exist until dropped").item
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::resource_metrics::ResourceMetrics;

    #[derive(Clone, Debug)]
    struct Size(u64);

    impl BufferDescription for Size {
        type Cost = u64;
        fn cost_to_use(&self, other: &Self) -> Option<u64> {
            other.0.checked_sub(self.0)
        }
    }

    struct DropProbe(Arc<AtomicU64>);
    impl Drop for DropProbe {
        fn drop(&mut self) { self.0.fetch_add(1, Ordering::Relaxed); }
    }

    fn item(metrics: &Arc<ResourceMetrics>, drops: &Arc<AtomicU64>, bytes: u64) -> PoolItem<DropProbe, Size> {
        PoolItem {
            item: DropProbe(drops.clone()),
            description: Size(bytes),
            _resource_lease: Some(metrics.lease(ResourceKind::ReadbackBuffer, bytes)),
            idle_reservation: None,
        }
    }

    #[test]
    fn returned_buffers_are_bounded_and_reuse_keeps_allocation_size() {
        let metrics = Arc::new(ResourceMetrics::default());
        let drops = Arc::new(AtomicU64::new(0));
        let mut pool = PoolStorage { items: Vec::new(), idle_budget: Some((100, (|size: &Size| size.0) as fn(&Size) -> u64)), shared_idle_budget: None, idle_notice: None };
        pool.put(item(&metrics, &drops, 60));
        pool.put(item(&metrics, &drops, 60)); // evicts previous idle allocation
        assert_eq!(pool.items.len(), 1);
        assert_eq!(drops.load(Ordering::Relaxed), 1);
        assert_eq!(metrics.snapshot().readback_buffer.live_estimated_bytes, 60);
        let reused = pool.take_compatible(&Size(20)).unwrap();
        assert_eq!(reused.description.0, 60); // actual allocation, not request size
        assert_eq!(metrics.snapshot().readback_buffer.created_count, 2);
        pool.put(reused);
        pool.put(item(&metrics, &drops, 101)); // oversized return is not retained
        assert_eq!(pool.items.len(), 1);
        assert_eq!(metrics.snapshot().readback_buffer.released_count, 2);
        drop(pool);
        assert_eq!(drops.load(Ordering::Relaxed), 3);
        let stats = metrics.snapshot().readback_buffer;
        assert_eq!((stats.live_count, stats.live_estimated_bytes), (0, 0));
        assert_eq!(stats.created_estimated_bytes, stats.released_estimated_bytes);
    }

    #[test]
    fn shared_budget_bounds_distinct_texture_buckets_and_releases_reservations() {
        let budget = Arc::new(IdleBudget { used: AtomicU64::new(0), limit: AtomicU64::new(100) });
        let metrics = Arc::new(ResourceMetrics::default());
        let drops = Arc::new(AtomicU64::new(0));
        // Admission must charge the actual returned allocation even when the
        // bucket's last size estimate would have let both items through.
        let mut first = PoolStorage { items: Vec::new(), idle_budget: None, shared_idle_budget: Some((budget.clone(), 10)), idle_notice: None };
        let mut second = PoolStorage { items: Vec::new(), idle_budget: None, shared_idle_budget: Some((budget.clone(), 60)), idle_notice: None };
        first.put(item(&metrics, &drops, 60));
        second.put(item(&metrics, &drops, 60));
        assert!(second.items.is_empty());
        assert_eq!(budget.used.load(Ordering::Relaxed), 60);
        assert_eq!(drops.load(Ordering::Relaxed), 1);
        let live = first.take_compatible(&Size(60)).unwrap();
        assert_eq!(budget.used.load(Ordering::Relaxed), 0);
        second.put(item(&metrics, &drops, 60));
        drop(first);
        assert_eq!(metrics.snapshot().readback_buffer.live_count, 2);
        drop(second);
        assert_eq!(budget.used.load(Ordering::Relaxed), 0);
        assert_eq!(metrics.snapshot().readback_buffer.live_count, 1);
        drop(live);
        assert_eq!(metrics.snapshot().readback_buffer.live_count, 0);
        assert_eq!(drops.load(Ordering::Relaxed), 3);
    }

    #[test]
    fn clearing_idle_items_preserves_checked_out_owner_then_return_is_counted() {
        let metrics = Arc::new(ResourceMetrics::default());
        let drops = Arc::new(AtomicU64::new(0));
        let pool: BufferPool<DropProbe, Size> = BufferPool::with_idle_budget(
            Box::new(|_, _| panic!("This CPU-only test never constructs a GPU resource")),
            100, |size| size.0,
        );
        pool.available.lock().unwrap().put(item(&metrics, &drops, 40));
        let live = PoolEntry {
            item: Some(item(&metrics, &drops, 80)),
            pool: Arc::downgrade(&pool.available),
        };
        pool.clear_available();
        assert_eq!(metrics.snapshot().readback_buffer.live_estimated_bytes, 80);
        assert_eq!(drops.load(Ordering::Relaxed), 1);
        drop(live); // return retains allocation, not a release/recreate
        assert_eq!(metrics.snapshot().readback_buffer.live_count, 1);
        assert_eq!(metrics.snapshot().readback_buffer.created_count, 2);
        drop(pool);
        assert_eq!(metrics.snapshot().readback_buffer.released_count, 2);
        assert_eq!(drops.load(Ordering::Relaxed), 2);
    }

    fn key(width: u32) -> TextureKey {
        TextureKey {
            size: wgpu::Extent3d { width, height: 2, depth_or_array_layers: 1 },
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            format: wgpu::TextureFormat::Rgba8Unorm,
            sample_count: 4,
        }
    }

    fn test_index<T, D: BufferDescription>(pools: &FnvHashMap<TextureKey, TextureBucket<T, D>>) -> IdleIndex {
        let mut index = IdleIndex::default();
        for (&key, bucket) in pools { index.sync(key, bucket); }
        index
    }

    #[test]
    fn pressure_lookup_ignores_empty_metadata_and_reindexes_late_returns() {
        let budget = Arc::new(IdleBudget { used: AtomicU64::new(0), limit: AtomicU64::new(32) });
        let metrics = Arc::new(ResourceMetrics::default());
        let drops = Arc::new(AtomicU64::new(0));
        let mut pools = FnvHashMap::default();
        let mut index = IdleIndex::default();
        for width in 1..=3000 {
            let pool = BufferPool::<DropProbe, Size>::new(Box::new(|_, _| panic!("CPU-only")));
            {
                let mut storage = pool.available.lock().unwrap();
                storage.shared_idle_budget = Some((budget.clone(), 32));
                storage.idle_notice = Some((index.returns.clone(), key(width)));
            }
            pools.insert(key(width), TextureBucket { pool, last_used: 0, last_access: u64::from(width) });
        }
        let pool = &pools[&key(2999)].pool;
        pool.available.lock().unwrap().put(item(&metrics, &drops, 32));
        index.drain_returns(&pools);
        assert_eq!(evict_to_fit(&pools, &mut index, &budget, 32, Some((key(3001), 2))), (1, 1));
        assert!(index.by_access.iter().all(BTreeMap::is_empty));
        // An entry held during the eviction returns later and must be reusable
        // or evictable again, without rescanning the other 2999 empty buckets.
        let live = PoolEntry { item: Some(item(&metrics, &drops, 32)), pool: Arc::downgrade(&pool.available) };
        drop(live);
        index.drain_returns(&pools);
        assert_eq!(evict_to_fit(&pools, &mut index, &budget, 32, Some((key(3001), 2))), (1, 1));
        assert_eq!(budget.used.load(Ordering::Relaxed), 0);
    }

    #[test]
    fn a_new_size_replaces_cold_idle_resources_and_keeps_hot_and_live_owners() {
        let budget = Arc::new(IdleBudget { used: AtomicU64::new(0), limit: AtomicU64::new(96) });
        let metrics = Arc::new(ResourceMetrics::default());
        let drops = Arc::new(AtomicU64::new(0));
        let cold_key = key(1);
        let mut hot_key = key(1);
        hot_key.format = wgpu::TextureFormat::Bgra8Unorm;
        let mut pools = FnvHashMap::default();
        for (key, last_access) in [(cold_key, 1), (hot_key, 9)] {
            let pool = BufferPool::<DropProbe, Size>::new(Box::new(|_, _| panic!("CPU-only test")));
            let mut available = pool.available.lock().unwrap();
            available.shared_idle_budget = Some((budget.clone(), 32));
            available.put(item(&metrics, &drops, 32));
            drop(available);
            pools.insert(key, TextureBucket { pool, last_used: 0, last_access });
        }
        let live = PoolEntry { item: Some(item(&metrics, &drops, 32)), pool: Arc::downgrade(&pools[&cold_key].pool.available) };
        assert_eq!(evict_to_fit(&pools, &mut test_index(&pools), &budget, key(2).estimated_bytes(), Some((key(2), 2))).0, 1);
        assert!(pools[&cold_key].pool.available.lock().unwrap().items.is_empty());
        assert_eq!(pools[&hot_key].pool.available.lock().unwrap().items.len(), 1);
        assert_eq!(drops.load(Ordering::Relaxed), 1);
        let mut new_size = PoolStorage { items: Vec::new(), idle_budget: None, shared_idle_budget: Some((budget.clone(), 64)), idle_notice: None };
        new_size.put(item(&metrics, &drops, 64));
        assert_eq!(budget.used.load(Ordering::Relaxed), 96);
        drop(live); // Full idle budget drops this return, not the hot allocation.
        assert_eq!(pools[&hot_key].pool.available.lock().unwrap().items.len(), 1);
        drop(new_size);
        drop(pools);
        assert_eq!(budget.used.load(Ordering::Relaxed), 0);
        assert_eq!(metrics.snapshot().readback_buffer.live_count, 0);
    }

    #[test]
    fn main_and_offscreen_share_bucket_ownership_and_one_frame_clock() {
        let mut main = TexturePool::new();
        let offscreen = TexturePool::sharing_with(&main);
        assert!(Arc::ptr_eq(&main.pools, &offscreen.pools));
        main.end_frame();
        assert_eq!(offscreen.frame.load(Ordering::Relaxed), 1);
        assert!(Arc::ptr_eq(&main.idle_budget, &offscreen.idle_budget));
    }

    #[test]
    fn one_off_sizes_do_not_displace_the_recent_working_set() {
        let budget = Arc::new(IdleBudget { used: AtomicU64::new(0), limit: AtomicU64::new(96) });
        let metrics = Arc::new(ResourceMetrics::default());
        let drops = Arc::new(AtomicU64::new(0));
        let pool = BufferPool::<DropProbe, Size>::new(Box::new(|_, _| panic!("CPU-only")));
        {
            let mut idle = pool.available.lock().unwrap();
            idle.shared_idle_budget = Some((budget.clone(), 32));
            for _ in 0..3 { idle.put(item(&metrics, &drops, 32)); }
        }
        let mut pools = FnvHashMap::default();
        pools.insert(key(1), TextureBucket { pool, last_used: 7, last_access: 1 });
        assert_eq!(evict_to_fit(&pools, &mut test_index(&pools), &budget, 64, Some((key(2), 8))).0, 0);
        assert_eq!(drops.load(Ordering::Relaxed), 0);
        let mut transient = PoolStorage { items: Vec::new(), idle_budget: None, shared_idle_budget: Some((budget.clone(), 64)), idle_notice: None };
        transient.put(item(&metrics, &drops, 64));
        assert!(transient.items.is_empty());
        assert_eq!(budget.used.load(Ordering::Relaxed), 96);
        assert_eq!(evict_to_fit(&pools, &mut test_index(&pools), &budget, 64, Some((key(2), 9))).0, 2);
        assert_eq!(pools[&key(1)].pool.available.lock().unwrap().items.len(), 1);
    }

    #[test]
    fn small_request_does_not_discard_an_entire_same_size_idle_bucket() {
        let budget=Arc::new(IdleBudget {used:AtomicU64::new(0),limit:AtomicU64::new(128)});
        let metrics=Arc::new(ResourceMetrics::default());
        let drops=Arc::new(AtomicU64::new(0));
        let pool=BufferPool::<DropProbe,Size>::new(Box::new(|_,_|panic!("CPU-only test")));
        {
            let mut available=pool.available.lock().unwrap();
            available.shared_idle_budget=Some((budget.clone(),32));
            for _ in 0..4 {available.put(item(&metrics,&drops,32));}
        }
        let mut pools=FnvHashMap::default();
        pools.insert(key(1),TextureBucket {pool,last_used:0,last_access:0});
        let mut requested=key(1);requested.format=wgpu::TextureFormat::Bgra8Unorm;
        assert_eq!(evict_to_fit(&pools,&mut test_index(&pools),&budget,requested.estimated_bytes(),Some((requested,2))).0,1);
        assert_eq!(pools[&key(1)].pool.available.lock().unwrap().items.len(),3);
        assert_eq!(budget.used.load(Ordering::Relaxed),96);
        assert_eq!(drops.load(Ordering::Relaxed),1);
    }

    #[test]
    fn estimated_budget_includes_samples_and_lowering_the_limit_reclaims_only_the_excess() {
        assert_eq!(key(3).estimated_bytes(), 3 * 2 * 4 * 4);
        let budget = Arc::new(IdleBudget { used: AtomicU64::new(0), limit: AtomicU64::new(96) });
        let metrics = Arc::new(ResourceMetrics::default());
        let drops = Arc::new(AtomicU64::new(0));
        let mut pools = FnvHashMap::default();
        let pool = BufferPool::<DropProbe, Size>::new(Box::new(|_, _| panic!("CPU-only")));
        {
            let mut idle = pool.available.lock().unwrap();
            idle.shared_idle_budget = Some((budget.clone(), 32));
            for _ in 0..3 { idle.put(item(&metrics, &drops, 32)); }
        }
        pools.insert(key(1), TextureBucket { pool, last_used: 3, last_access: 0 });
        let mut index = test_index(&pools);
        budget.limit.store(64, Ordering::Relaxed);
        assert_eq!(evict_to_fit(&pools, &mut index, &budget, 0, None), (1, 1));
        assert_eq!(budget.used.load(Ordering::Relaxed), 64);
        budget.limit.store(0, Ordering::Relaxed);
        assert_eq!(evict_to_fit(&pools, &mut index, &budget, 0, None), (2, 1));
        assert_eq!(budget.used.load(Ordering::Relaxed), 0);
    }

    #[test]
    fn expiry_reschedules_reused_buckets_and_expires_empty_metadata_at_the_exact_age() {
        let mut pools = FnvHashMap::default();
        let mut index = IdleIndex::default();
        for (width, first_used, last_used) in [(1, 0, 0), (2, 0, 1), (3, 1, 2)] {
            let pool = BufferPool::<DropProbe, Size>::new(Box::new(|_, _| panic!("CPU-only")));
            pools.insert(key(width), TextureBucket { pool, last_used, last_access: width.into() });
            index.by_frame.entry(first_used).or_default().push(key(width));
        }
        index.expire(&mut pools, 1, 2, false);
        assert_eq!(pools.len(), 3);
        index.expire(&mut pools, 2, 2, false);
        assert_eq!(pools.len(), 2);
        assert!(!pools.contains_key(&key(1)));
        index.expire(&mut pools, 3, 2, false);
        assert_eq!(pools.len(), 1);
        assert!(pools.contains_key(&key(3)));
        index.expire(&mut pools, 3, 0, false);
        assert!(pools.is_empty());
        assert!(index.by_frame.is_empty());
    }

    #[test]
    fn long_run_expiry_preserves_borrowed_return_path_and_trims_only_cold_siblings() {
        let budget = Arc::new(IdleBudget::default());
        let drops = Arc::new(AtomicU64::new(0));
        let metrics = Arc::new(ResourceMetrics::default());
        let pool = BufferPool::<DropProbe, Size>::new(Box::new(|_, _| panic!("CPU-only")));
        let mut index = IdleIndex::default();
        let key = key(1);
        {
            let mut idle = pool.available.lock().unwrap();
            idle.shared_idle_budget = Some((budget.clone(), 32));
            idle.idle_notice = Some((index.returns.clone(), key));
            for _ in 0..4 { idle.put(item(&metrics, &drops, 32)); }
        }
        let borrowed = PoolEntry {
            item: pool.available.lock().unwrap().take_compatible(&Size(32)),
            pool: Arc::downgrade(&pool.available),
        };
        let mut pools = FnvHashMap::default();
        pools.insert(key, TextureBucket { pool, last_used: 0, last_access: 1 });
        index.by_frame.entry(0).or_default().push(key);
        index.drain_returns(&pools);
        index.expire(&mut pools, 1, 120, true);
        assert_eq!(budget.used.load(Ordering::Relaxed), 96);
        index.expire(&mut pools, 2, 120, true);
        assert_eq!(budget.used.load(Ordering::Relaxed), 32);
        index.expire(&mut pools, 31, 120, true);
        assert_eq!(budget.used.load(Ordering::Relaxed), 0);
        assert!(index.by_access.iter().all(BTreeMap::is_empty));
        assert_eq!(drops.load(Ordering::Relaxed), 3);
        drop(borrowed);
        assert_eq!(budget.used.load(Ordering::Relaxed), 32);
        index.drain_returns(&pools);
        assert_eq!(index.by_access[0].len(), 1);
        index.expire(&mut pools, 62, 120, true);
        assert!(pools.is_empty());
        assert!(index.by_frame.is_empty());
        assert_eq!(drops.load(Ordering::Relaxed), 4);
    }

    #[test]
    fn dropping_pool_does_not_invalidate_an_outstanding_entry() {
        let available = Arc::new(Mutex::new(PoolStorage::<String, AlwaysCompatible> {
            items: Vec::new(), idle_budget: None, shared_idle_budget: None, idle_notice: None,
        }));
        let live = PoolEntry {
            item: Some(PoolItem { item: String::from("live texture stand-in"), description: AlwaysCompatible, _resource_lease: None, idle_reservation: None }),
            pool: Arc::downgrade(&available),
        };
        drop(available);
        assert_eq!(&**live, "live texture stand-in");
        drop(live);
    }
}
