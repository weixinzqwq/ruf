use crate::sandbox::SandboxType;

use gc_arena::{Collect, DynamicRootSet, Gc, GcWeak, Mutation};
use std::fmt::{Debug, Formatter};
use std::hash::{Hash, Hasher};
use std::ops::Deref;
use std::sync::{Arc, Mutex, OnceLock, Weak, atomic::Ordering};
use sha2::{Digest, Sha256};
use swf::{Fixed8, HeaderExt, Rectangle, Twips};
use url::Url;

pub type SwfStream<'a> = swf::read::Reader<'a>;

/// Share only immutable decompressed bytes; URLs, domains and loader state remain independent.
pub struct PreparedSwf {
    header: HeaderExt,
    data: Arc<Vec<u8>>,
    compressed_len: usize,
}

struct SharedSwf { header: HeaderExt, data: Weak<Vec<u8>> }
type SwfCache = std::collections::HashMap<[u8; 32], SharedSwf>;

impl PreparedSwf {
    pub fn decode(bytes: &[u8]) -> Result<Self, swf::error::Error> {
        static CACHE: OnceLock<Mutex<SwfCache>> = OnceLock::new();
        let cache = CACHE.get_or_init(Default::default);
        let key: Option<[u8; 32]> = crate::aoqi_work::mode().dedup.then(|| Sha256::digest(bytes).into());
        let lookup = |cache: &SwfCache, key: &[u8; 32]| {
            cache.get(key).and_then(|entry| entry.data.upgrade().map(|data| {
                crate::aoqi_work::SWF_DEDUP_HITS.fetch_add(1, Ordering::Relaxed);
                crate::aoqi_work::SWF_SHARED_BYTES.fetch_add(data.len() as u64, Ordering::Relaxed);
                Self { header: entry.header.clone(), data, compressed_len: bytes.len() }
            }))
        };
        if let Some(key) = &key
            && let Some(hit) = lookup(&cache.lock().unwrap(), key) { return Ok(hit); }
        let decoded = swf::read::decompress_swf(bytes)?;
        crate::aoqi_work::SWF_DECODES.fetch_add(1, Ordering::Relaxed);
        let prepared = Self { header: decoded.header, data: Arc::new(decoded.data), compressed_len: bytes.len() };
        if let Some(key) = key {
            let mut cache = cache.lock().unwrap();
            // Another worker may have completed the same resource while we decoded.
            if let Some(hit) = lookup(&cache, &key) { return Ok(hit); }
            cache.retain(|_, entry| entry.data.strong_count() > 0);
            // Metadata has a hard cap; the cache never keeps resource bytes alive.
            if cache.len() >= 4096 { cache.clear(); }
            cache.insert(key, SharedSwf { header: prepared.header.clone(), data: Arc::downgrade(&prepared.data) });
        }
        Ok(prepared)
    }
}

/// A shared pointer to a `SwfMovieData`.
#[derive(Clone, Collect, Copy, Debug)]
#[collect(no_drop)]
pub struct SwfMovie<'gc>(Gc<'gc, MovieOwner<'gc>>);

#[derive(Collect)]
#[collect(no_drop)]
struct MovieOwner<'gc> {
    data: SwfMovieData,
    // The core's typed library is anchored here, not in the global movie index.
    // DynamicRootSet avoids a dependency from common back to core.
    library_roots: DynamicRootSet<'gc>,
}

impl Debug for MovieOwner<'_> {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result { self.data.fmt(f) }
}

impl<'gc> SwfMovie<'gc> {
    pub fn new(mc: &Mutation<'gc>, data: SwfMovieData) -> Self {
        SwfMovie(Gc::new(mc, MovieOwner { data, library_roots: DynamicRootSet::new(mc) }))
    }

    pub fn library_roots(self) -> DynamicRootSet<'gc> {
        self.0.library_roots
    }

    pub fn downgrade(self) -> SwfMovieWeak<'gc> {
        SwfMovieWeak(Gc::downgrade(self.0))
    }

    pub fn as_ptr(self) -> *const SwfMovieData {
        &self.0.data
    }

    pub fn ptr_eq(this: &Self, other: &Self) -> bool {
        std::ptr::eq(this.as_ptr(), other.as_ptr())
    }
}

impl<'gc> Deref for SwfMovie<'gc> {
    type Target = SwfMovieData;

    fn deref(&self) -> &Self::Target {
        &self.0.data
    }
}

impl PartialEq for SwfMovie<'_> {
    fn eq(&self, other: &Self) -> bool {
        Gc::ptr_eq(self.0, other.0)
    }
}

impl Eq for SwfMovie<'_> {}

impl Hash for SwfMovie<'_> {
    fn hash<H: Hasher>(&self, state: &mut H) {
        Gc::as_ptr(self.0).hash(state);
    }
}

/// Like `SwfMovie`, but is a weak reference.
#[derive(Clone, Collect, Copy, Debug)]
#[collect(no_drop)]
pub struct SwfMovieWeak<'gc>(GcWeak<'gc, MovieOwner<'gc>>);

impl<'gc> SwfMovieWeak<'gc> {
    pub fn is_dropped(self) -> bool {
        self.0.is_dropped()
    }

    pub fn upgrade(&self, mc: &Mutation<'gc>) -> Option<SwfMovie<'gc>> {
        self.0.upgrade(mc).map(SwfMovie)
    }
}

/// An open, fully parsed SWF movie ready to play back, either in a Player or a
/// MovieClip.
#[derive(Clone, Collect)]
#[collect(require_static)]
pub struct SwfMovieData {
    /// The SWF header parsed from the data stream.
    header: HeaderExt,

    /// Uncompressed SWF data.
    data: Arc<Vec<u8>>,

    /// The URL the SWF was downloaded from.
    url: String,

    /// The URL that triggered the SWF load.
    loader_url: Option<String>,

    /// Any parameters provided when loading this movie (also known as 'flashvars'),
    /// as a list of key-value pairs.
    parameters: Vec<(String, String)>,

    /// The suggest encoding for this SWF.
    encoding: &'static swf::Encoding,

    /// The compressed length of the entire datastream
    compressed_len: usize,

    /// Whether this SwfMovieData actually represents a loaded movie or fills in for
    /// something else, like an loaded image, filler movie, or error state.
    is_movie: bool,

    /// Whether this movie should be interpreted as AVM1, regardless of what the
    /// header declares.
    force_avm1: bool,

    /// Whether this movie was loaded using `Loader.loadBytes`.
    ///
    /// If this is `true`, the `url` field is likely meaningless, as the movie
    /// was loaded from bytes, not a URL. If this is `true`, an HTTP status
    /// event will not be fired on the `loaderInfo` of this movie.
    is_from_bytes: bool,

    /// Security sandbox type enforced for this movie.
    ///
    /// It absolutely cannot be changed after constructing
    /// the object in order to ensure proper sandboxing.
    sandbox_type: SandboxType,
}

impl SwfMovieData {
    /// Construct an empty movie.
    pub fn empty(swf_version: u8, loader_url: Option<String>) -> Self {
        let url = "file:///".to_string();
        let header = HeaderExt::default_with_swf_version(swf_version);

        // TODO What sandbox type should we use here?
        let sandbox_type = SandboxType::infer(url.as_str(), &header);
        Self {
            header,
            data: Arc::new(Vec::new()),
            url,
            loader_url,
            parameters: Vec::new(),
            encoding: swf::UTF_8,
            compressed_len: 0,
            is_movie: false,
            force_avm1: false,
            is_from_bytes: false,
            sandbox_type,
        }
    }

    /// Construct an empty movie with a fake `compressed_len`.
    /// This is used by `Loader` when firing an initial `progress` event:
    /// `LoaderInfo.bytesTotal` is set to the actual value, but no data is available,
    /// and `LoaderInfo.parameters` is empty.
    pub fn fake_with_compressed_len(
        swf_version: u8,
        loader_url: Option<String>,
        compressed_len: usize,
    ) -> Self {
        let url = "file:///".to_string();
        let header = HeaderExt::default_with_swf_version(swf_version);

        // TODO What sandbox type should we use here?
        let sandbox_type = SandboxType::infer(url.as_str(), &header);
        Self {
            header,
            compressed_len,
            data: Arc::new(Vec::new()),
            url,
            loader_url,
            parameters: Vec::new(),
            encoding: swf::UTF_8,
            is_movie: false,
            force_avm1: false,
            is_from_bytes: false,
            sandbox_type,
        }
    }

    /// Like `fake_with_compressed_len`, but uses actual data.
    /// This is used when loading a Bitmap to expose the underlying content
    pub fn fake_with_compressed_data(
        swf_version: u8,
        loader_url: Option<String>,
        compressed_data: Vec<u8>,
    ) -> Self {
        let url = "file:///".to_string();
        let header = HeaderExt::default_with_swf_version(swf_version);

        // TODO What sandbox type should we use here?
        let sandbox_type = SandboxType::infer(url.as_str(), &header);
        Self {
            header,
            compressed_len: compressed_data.len(),
            data: Arc::new(compressed_data),
            url,
            loader_url,
            parameters: Vec::new(),
            encoding: swf::UTF_8,
            is_movie: false,
            force_avm1: false,
            is_from_bytes: false,
            sandbox_type,
        }
    }

    /// Constructs the error state movie stub in which some attributes have certain
    /// error values to signal that no valid file could be loaded.
    ///
    /// This happens if no file could be loaded or if the loaded content is no valid
    /// supported content.
    pub fn error_movie(movie_url: String) -> Self {
        let header = HeaderExt::default_error_header();

        // TODO What sandbox type should we use here?
        let sandbox_type = SandboxType::infer(movie_url.as_str(), &header);
        Self {
            header,
            data: Arc::new(Vec::new()),
            url: movie_url,
            loader_url: None,
            parameters: Vec::new(),
            encoding: swf::UTF_8,
            compressed_len: 0,
            is_movie: false,
            force_avm1: false,
            is_from_bytes: false,
            sandbox_type,
        }
    }

    /// Construct a movie based on the contents of the SWF datastream. If the
    /// SWF was loaded specifically from the AVM2 method `Loader.loadBytes`,
    /// this method accepts extra information as a `LoadBytesInfo`.
    pub fn from_data(
        swf_data: &[u8],
        url: String,
        loader_url: Option<String>,
        load_bytes_info: Option<LoadBytesInfo>,
    ) -> Result<Self, swf::error::Error> {
        Ok(Self::from_prepared(PreparedSwf::decode(swf_data)?, url, loader_url, load_bytes_info))
    }

    pub fn from_prepared(
        swf_buf: PreparedSwf, url: String, loader_url: Option<String>,
        load_bytes_info: Option<LoadBytesInfo>,
    ) -> Self {
        let encoding = swf::SwfStr::encoding_for_version(swf_buf.header.version());

        // The loader SWF has full control over the tags of a SWF loaded using
        // `Loader.loadBytes`, so if we were to use the sandbox type declared in
        // that SWF's header, the SWF could break sandboxing. Instead, always
        // use the sandbox type of the loader SWF, to ensure that it can't load
        // a child SWF with different sandboxing.
        let sandbox_type = load_bytes_info
            .map(|i| i.loader_sandbox_type)
            .unwrap_or_else(|| SandboxType::infer(url.as_str(), &swf_buf.header));

        let mut movie = Self {
            header: swf_buf.header,
            data: swf_buf.data,
            url,
            loader_url,
            parameters: Vec::new(),
            encoding,
            compressed_len: swf_buf.compressed_len,
            is_movie: true,
            force_avm1: false,
            is_from_bytes: load_bytes_info.is_some(),
            sandbox_type,
        };
        movie.append_parameters_from_url();
        movie
    }

    /// Construct a movie based on a loaded image (JPEG, GIF or PNG).
    pub fn from_loaded_image(
        url: String,
        is_from_bytes: bool,
        length: usize,
        width: u32,
        height: u32,
    ) -> Self {
        let stage_size = Rectangle::ZERO
            .with_width(Twips::from_pixels_i32(width as i32))
            .with_height(Twips::from_pixels_i32(height as i32));
        let header = HeaderExt::default_with_uncompressed_len(length as i32, stage_size);
        let sandbox_type = SandboxType::infer(url.as_str(), &header);
        let mut movie = Self {
            header,
            data: Arc::new(Vec::new()),
            url,
            loader_url: None,
            parameters: Vec::new(),
            encoding: swf::UTF_8,
            compressed_len: length,
            is_movie: false,
            force_avm1: false,
            is_from_bytes,
            sandbox_type,
        };
        movie.append_parameters_from_url();
        movie
    }

    fn append_parameters_from_url(&mut self) {
        match Url::parse(&self.url) {
            Ok(url) => {
                for (key, value) in url.query_pairs() {
                    self.parameters.push((key.into_owned(), value.into_owned()));
                }
            }
            Err(e) => {
                tracing::error!(
                    "Failed to parse loader URL when extracting query parameters: {}",
                    e
                );
            }
        }
    }

    pub fn header(&self) -> &HeaderExt {
        &self.header
    }

    /// Get the version of the SWF.
    pub fn version(&self) -> u8 {
        self.header.version()
    }

    pub fn data(&self) -> &[u8] {
        &self.data
    }

    /// Returns the suggested string encoding for the given SWF version.
    /// For SWF version 6 and higher, this is always UTF-8.
    /// For SWF version 5 and lower, this is locale-dependent,
    /// and we default to WINDOWS-1252.
    pub fn encoding(&self) -> &'static swf::Encoding {
        self.encoding
    }

    /// The width of the movie in twips.
    pub fn width(&self) -> Twips {
        self.header.stage_size().width()
    }

    /// The height of the movie in twips.
    pub fn height(&self) -> Twips {
        self.header.stage_size().height()
    }

    /// Get the URL this SWF was fetched from.
    pub fn url(&self) -> &str {
        &self.url
    }

    pub fn set_url(&mut self, url: String) {
        self.url = url;
    }

    pub fn set_force_avm1(&mut self) {
        self.force_avm1 = true;
    }

    pub fn is_from_bytes(&self) -> bool {
        self.is_from_bytes
    }

    /// Get the URL that triggered the fetch of this SWF.
    pub fn loader_url(&self) -> Option<&str> {
        self.loader_url.as_deref()
    }

    pub fn parameters(&self) -> &[(String, String)] {
        &self.parameters
    }

    pub fn append_parameters(&mut self, params: impl IntoIterator<Item = (String, String)>) {
        self.parameters.extend(params);
    }

    pub fn compressed_len(&self) -> usize {
        self.compressed_len
    }

    pub fn uncompressed_len(&self) -> i32 {
        self.header.uncompressed_len()
    }

    /// Whether the SWF's FileAttributes tag declares the SWF to be AVM2.
    pub fn is_declared_action_script_3(&self) -> bool {
        self.header.is_action_script_3()
    }

    /// Whether this `SwfMovieData` should be interpreted as AVM2.
    ///
    /// This usually is the same as `is_declared_action_script_3`, but will
    /// return false if this is an AVM2 movie loaded by AVM1 (which we mark by
    /// setting `force_avm1` to true).
    pub fn is_action_script_3(&self) -> bool {
        self.header.is_action_script_3() && !self.force_avm1
    }

    pub fn stage_size(&self) -> &Rectangle<Twips> {
        self.header.stage_size()
    }

    pub fn num_frames(&self) -> u16 {
        self.header.num_frames()
    }

    pub fn frame_rate(&self) -> Fixed8 {
        self.header.frame_rate()
    }

    pub fn is_movie(&self) -> bool {
        self.is_movie
    }

    pub fn sandbox_type(&self) -> SandboxType {
        self.sandbox_type
    }
}

impl Debug for SwfMovieData {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SwfMovieData")
            .field("header", &self.header)
            .field("data", &self.data.len())
            .field("url", &self.url)
            .field("loader_url", &self.loader_url)
            .field("parameters", &self.parameters)
            .field("encoding", &self.encoding)
            .field("compressed_len", &self.compressed_len)
            .field("is_movie", &self.is_movie)
            .field("sandbox_type", &self.sandbox_type)
            .finish()
    }
}

/// A shared-ownership reference to some portion of an SWF datastream.
#[derive(Debug, Clone, Collect)]
#[collect(no_drop)]
pub struct SwfSlice {
    entire_data: Arc<Vec<u8>>,
    swf_version: u8,

    pub start: usize,
    pub end: usize,
}

impl<'gc> From<SwfMovie<'gc>> for SwfSlice {
    fn from(movie: SwfMovie<'gc>) -> Self {
        let end = movie.data().len();

        Self {
            entire_data: movie.data.clone(),
            swf_version: movie.version(),
            start: 0,
            end,
        }
    }
}

impl AsRef<[u8]> for SwfSlice {
    #[inline]
    fn as_ref(&self) -> &[u8] {
        self.data()
    }
}

impl SwfSlice {
    /// Creates an empty SwfSlice.
    #[inline]
    pub fn empty(movie: SwfMovie<'_>) -> Self {
        Self {
            entire_data: movie.data.clone(),
            swf_version: movie.version(),
            start: 0,
            end: 0,
        }
    }

    /// Creates an empty SwfSlice of the same movie.
    #[inline]
    pub fn copy_empty(&self) -> Self {
        Self {
            entire_data: self.entire_data.clone(),
            swf_version: self.swf_version,
            start: 0,
            end: 0,
        }
    }

    /// Construct a new SwfSlice from a regular slice.
    ///
    /// This function panics if the given slice is not a subslice of the current
    /// slice.
    pub fn to_subslice(&self, slice: &[u8]) -> Self {
        let self_pval = self.entire_data.as_ptr() as usize;
        let slice_pval = slice.as_ptr() as usize;

        if (self_pval + self.start) <= slice_pval && slice_pval < (self_pval + self.end) {
            Self {
                entire_data: self.entire_data.clone(),
                swf_version: self.swf_version,
                start: slice_pval - self_pval,
                end: (slice_pval - self_pval) + slice.len(),
            }
        } else {
            self.copy_empty()
        }
    }

    /// Construct a new SwfSlice from a movie subslice.
    ///
    /// This function allows subslices outside the current slice to be formed,
    /// as long as they are valid subslices of the movie itself.
    pub fn to_unbounded_subslice(&self, slice: &[u8]) -> Self {
        let self_pval = self.entire_data.as_ptr() as usize;
        let self_len = self.entire_data.len();
        let slice_pval = slice.as_ptr() as usize;

        if self_pval <= slice_pval && slice_pval < (self_pval + self_len) {
            Self {
                entire_data: self.entire_data.clone(),
                swf_version: self.swf_version,
                start: slice_pval - self_pval,
                end: (slice_pval - self_pval) + slice.len(),
            }
        } else {
            self.copy_empty()
        }
    }

    /// Construct a new SwfSlice from a SwfStream.
    ///
    /// This is intended to allow constructing references to the contents of a
    /// given SWF tag.
    ///
    /// If the reader references a slice outside the bounds of the current
    /// slice, or the given reader refers to a different underlying movie, this
    /// function returns an empty slice.
    pub fn resize_to_reader(&self, reader: &SwfStream<'_>) -> Self {
        self.to_subslice(reader.get_ref())
    }

    /// Convert the SwfSlice into a standard data slice.
    pub fn data(&self) -> &[u8] {
        &self.entire_data[self.start..self.end]
    }

    /// Access the entire data of the movie backing this SwfSlice.
    pub fn entire_data(&self) -> &[u8] {
        &self.entire_data
    }

    /// Checks if this slice is empty
    pub fn is_empty(&self) -> bool {
        self.end == self.start
    }

    /// Construct a reader for this slice.
    ///
    /// The `from` parameter is the offset to start reading the slice from.
    pub fn read_from(&self, from: u64) -> swf::read::Reader<'_> {
        swf::read::Reader::new(&self.data()[from as usize..], self.swf_version)
    }

    /// Get the length of the SwfSlice.
    pub fn len(&self) -> usize {
        self.end - self.start
    }
}

/// Extra information provided when a SWF is loaded using `Loader.loadBytes`
/// (as opposed to when it is loaded from a URL).
#[derive(Clone, Copy)]
pub struct LoadBytesInfo {
    pub loader_sandbox_type: SandboxType,
}

#[cfg(test)]
mod stage3_tests {
    use super::*;
    const MOVIE: &[u8] = include_bytes!("../../../tests/tests/swfs/avm2/goto_on_orphan/test.swf");

    #[test]
    fn stage3_shared_bytes_keep_loader_metadata_separate_and_are_weakly_held() {
        let mut first = SwfMovieData::from_data(MOVIE, "https://one.invalid/a.swf?v=1".into(), None, None).unwrap();
        let second = SwfMovieData::from_data(MOVIE, "file:///b.swf?v=2".into(), Some("file:///loader.swf".into()),
            Some(LoadBytesInfo { loader_sandbox_type: SandboxType::LocalTrusted })).unwrap();
        assert!(Arc::ptr_eq(&first.data, &second.data));
        assert_ne!(first.url, second.url);
        assert_ne!(first.parameters, second.parameters);
        assert_eq!(second.sandbox_type, SandboxType::LocalTrusted);
        assert_eq!(first.sandbox_type, SandboxType::Remote);
        assert!(second.is_from_bytes && !first.is_from_bytes);
        first.set_force_avm1();
        assert!(!second.force_avm1);
        let weak = Arc::downgrade(&first.data);
        drop(first); drop(second);
        assert!(weak.upgrade().is_none());
        assert!(PreparedSwf::decode(b"not a swf").is_err());
    }
}
