use std::cell::RefCell;
use web_time::Instant;

use crate::backend::audio::RegisteredSound;
use crate::binary_data::BinaryData;
use crate::display_object::{
    Avm1Button, Avm2Button, BitmapClass, EditText, Graphic, MorphShape, MovieClip, Text, Video,
};
use crate::font::Font;
use gc_arena::barrier::unlock;
use gc_arena::lock::Lock;
use gc_arena::{Collect, Gc, Mutation};
use ruffle_render::backend::RenderBackend;
use ruffle_render::bitmap::{Bitmap as RenderBitmap, BitmapHandle, BitmapSize};
use ruffle_render::error::Error as RenderError;
use swf::DefineBitsLossless;

#[derive(Copy, Clone, Collect, Debug)]
#[collect(no_drop)]
pub enum Character<'gc> {
    EditText(EditText<'gc>),
    Graphic(Graphic<'gc>),
    MovieClip(MovieClip<'gc>),
    Bitmap(Gc<'gc, BitmapCharacter<'gc>>),
    Avm1Button(Avm1Button<'gc>),
    Avm2Button(Avm2Button<'gc>),
    Font(Font<'gc>),
    MorphShape(MorphShape<'gc>),
    Text(Text<'gc>),
    Sound(RegisteredSound<'gc>),
    Video(Video<'gc>),
    BinaryData(Gc<'gc, BinaryData>),
}

#[derive(Collect, Debug)]
#[collect(no_drop)]
pub struct BitmapCharacter<'gc> {
    #[collect(require_static)]
    compressed: CompressedBitmap,
    /// A lazily constructed GPU handle, used when performing fills with this bitmap
    #[collect(require_static)]
    handle: RefCell<Option<(Instant, BitmapHandle)>>,
    /// The bitmap class set by `SymbolClass` - this is used when we instantaite
    /// a `Bitmap` displayobject.
    avm2_class: Lock<BitmapClass<'gc>>,
}

impl<'gc> BitmapCharacter<'gc> {
    pub(crate) fn has_gpu_handle(&self) -> bool { self.handle.borrow().is_some() }

    pub fn new(compressed: CompressedBitmap) -> Self {
        Self {
            compressed,
            handle: RefCell::default(),
            avm2_class: Lock::new(BitmapClass::NoSubclass),
        }
    }

    pub(crate) fn expire_gpu_cache(&self, now: Instant) -> bool {
        let mut cached = self.handle.borrow_mut();
        if cached.as_ref().is_some_and(|(used, handle)|
            now.saturating_duration_since(*used) >= crate::tessellation_cache::GPU_CACHE_IDLE
            && std::sync::Arc::strong_count(&handle.0) == 1
            && !handle.0.has_external_bindings()) {
            cached.take();
            return true;
        }
        false
    }

    pub fn compressed(&self) -> &CompressedBitmap {
        &self.compressed
    }

    pub fn avm2_class(&self) -> BitmapClass<'gc> {
        self.avm2_class.get()
    }

    pub fn set_avm2_class(this: Gc<'gc, Self>, bitmap_class: BitmapClass<'gc>, mc: &Mutation<'gc>) {
        unlock!(Gc::write(mc, this), Self, avm2_class).set(bitmap_class);
    }

    pub fn bitmap_handle(
        &self,
        backend: &mut dyn RenderBackend,
    ) -> Result<BitmapHandle, RenderError> {
        if let Some((used, handle)) = self.handle.borrow_mut().as_mut() {
            *used = Instant::now();
            return Ok(handle.clone());
        }
        let decoded = self.compressed.decode()?;
        let new_handle = backend.register_bitmap(decoded)?;
        *self.handle.borrow_mut() = Some((Instant::now(), new_handle.clone()));
        Ok(new_handle)
    }
}

/// Holds a bitmap from an SWF tag, plus the decoded width/height.
/// We avoid decompressing the image until it's actually needed - some pathological SWFS
/// like 'House' have thousands of highly-compressed (mostly empty) bitmaps, which can
/// take over 10GB of ram if we decompress them all during preloading.
#[derive(Clone, Debug)]
pub enum CompressedBitmap {
    Jpeg {
        data: Vec<u8>,
        alpha: Option<Vec<u8>>,
        width: u32,
        height: u32,
    },
    Lossless(DefineBitsLossless<'static>),
}

impl CompressedBitmap {
    pub fn size(&self) -> BitmapSize {
        match self {
            CompressedBitmap::Jpeg { width, height, .. } => BitmapSize {
                width: *width,
                height: *height,
            },
            CompressedBitmap::Lossless(define_bits_lossless) => BitmapSize {
                width: define_bits_lossless.width.into(),
                height: define_bits_lossless.height.into(),
            },
        }
    }
    pub fn decode(&self) -> Result<RenderBitmap<'static>, RenderError> {
        match self {
            CompressedBitmap::Jpeg {
                data,
                alpha,
                width: _,
                height: _,
            } => ruffle_render::utils::decode_define_bits_jpeg(data, alpha.as_deref()),
            CompressedBitmap::Lossless(define_bits_lossless) => {
                ruffle_render::utils::decode_define_bits_lossless(define_bits_lossless)
            }
        }
    }
}

#[cfg(test)]
mod gpu_cache_tests {
    use super::*;
    use std::sync::{Arc, atomic::{AtomicBool, Ordering}};
    #[derive(Debug)]
    struct Handle(Arc<AtomicBool>);
    impl ruffle_render::bitmap::BitmapHandleImpl for Handle {
        fn has_external_bindings(&self) -> bool { self.0.load(Ordering::Relaxed) }
    }

    #[test]
    fn idle_bitmap_releases_only_unshared_gpu_owner_and_decodes_again() {
        let bitmap = BitmapCharacter::new(CompressedBitmap::Lossless(swf::DefineBitsLossless {
            version:2,id:1,format:swf::BitmapFormat::Rgb32,width:1,height:1,
            data:std::borrow::Cow::Borrowed(&[120, 156, 251, 47, 98, 147, 2, 0, 5, 24, 1, 180]),
        }));
        let expected = bitmap.compressed().decode().unwrap().data().to_vec();
        let external = Arc::new(AtomicBool::new(true));
        let owner = BitmapHandle(Arc::new(Handle(external.clone())));
        let weak = Arc::downgrade(&owner.0);
        *bitmap.handle.borrow_mut() = Some((Instant::now() - crate::tessellation_cache::GPU_CACHE_IDLE,owner.clone()));
        assert!(!bitmap.expire_gpu_cache(Instant::now()));
        drop(owner);
        assert!(!bitmap.expire_gpu_cache(Instant::now()), "mesh binding still uses this bitmap");
        external.store(false,Ordering::Relaxed);
        assert!(bitmap.expire_gpu_cache(Instant::now()));
        assert!(weak.upgrade().is_none());
        assert!(!bitmap.has_gpu_handle());
        let mut renderer=ruffle_render::backend::null::NullRenderer::new(ruffle_render::backend::ViewportDimensions {
            width:64,height:64,scale_factor:1.0,
        });
        let rebuilt=bitmap.bitmap_handle(&mut renderer).unwrap();
        assert!(bitmap.has_gpu_handle());
        assert_eq!(bitmap.compressed().decode().unwrap().data(), expected);
        assert_eq!(bitmap.bitmap_handle(&mut renderer).unwrap(),rebuilt);
    }
}
