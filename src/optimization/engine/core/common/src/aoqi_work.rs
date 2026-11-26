//! Independent switches and counters for the third optimization round.
use std::sync::{OnceLock, atomic::{AtomicU64, Ordering}};

#[derive(Clone, Copy)]
pub struct Mode {
    pub sleep: bool,
    pub loading: bool,
    pub dedup: bool,
    background_zws: bool,
    background_cws: bool,
}

fn is_large_cws(bytes: &[u8]) -> bool {
    bytes.len() >= 16 * 1024 && bytes.starts_with(b"CWS")
}

impl Mode {
    // Keep Loader.loadBytes' synchronous error and event behavior in CWS-only mode.
    pub fn background_for_load_bytes(self, bytes: &[u8]) -> bool {
        self.loading || (self.background_zws && bytes.starts_with(b"ZWS"))
    }

    pub fn background_for_movie(self, bytes: &[u8]) -> bool {
        self.background_for_load_bytes(bytes) || (self.background_cws && is_large_cws(bytes))
    }

    pub fn background_for_large_cws(self) -> bool {
        self.loading || self.background_cws
    }
}

static MODE: OnceLock<Mode> = OnceLock::new();

fn read_mode(default: &str) -> Mode {
    let name = std::env::var("AOQI_STAGE3_MODE").unwrap_or_else(|_| default.into());
    Mode { sleep: matches!(name.as_str(), "all" | "sleep"),
        loading: matches!(name.as_str(), "all" | "loading"),
        dedup: matches!(name.as_str(), "all" | "dedup"),
        background_zws: std::env::var("AOLA_BACKGROUND_ZWS").as_deref() == Ok("1"),
        background_cws: std::env::var("AOLA_BACKGROUND_CWS").as_deref() == Ok("1") }
}

/// Set the Aola default before creating a player, without changing process environment.
/// Explicit comparison modes still take precedence; other hosts keep their default.
pub fn initialize_aola_mode() {
    MODE.get_or_init(|| read_mode("sleep"));
}

pub fn mode() -> Mode {
    *MODE.get_or_init(|| read_mode("all"))
}

pub static BACKGROUND_JOBS: AtomicU64 = AtomicU64::new(0);
pub static BACKGROUND_NS: AtomicU64 = AtomicU64::new(0);
/// Completed movie jobs for compressed CWS inputs of at least 16 KiB on desktop workers.
pub static BACKGROUND_LARGE_CWS_JOBS: AtomicU64 = AtomicU64::new(0);
pub static SWF_DECODES: AtomicU64 = AtomicU64::new(0);
pub static SWF_DEDUP_HITS: AtomicU64 = AtomicU64::new(0);
pub static SWF_SHARED_BYTES: AtomicU64 = AtomicU64::new(0);
pub static SLEEP_SCANNED: AtomicU64 = AtomicU64::new(0);
pub static SLEEP_SKIPPED: AtomicU64 = AtomicU64::new(0);
pub static SLEEP_WAKES: AtomicU64 = AtomicU64::new(0);

/// Only owned bytes and immutable data cross this boundary, never VM/GC objects.
pub async fn blocking<T: Send + 'static>(f: impl FnOnce() -> T + Send + 'static) -> Result<T, &'static str> {
    blocking_when(mode().loading, f).await
}

/// Schedule decoding separately from optional background HTTP/cache work.
pub async fn decode_movie<T: Send + 'static>(
    bytes: Vec<u8>,
    decode: impl FnOnce(Vec<u8>) -> T + Send + 'static,
) -> Result<T, &'static str> {
    let background = mode().background_for_movie(&bytes);
    let count_large_cws = cfg!(not(target_family = "wasm")) && background && is_large_cws(&bytes);
    blocking_when(background, move || {
        let result = decode(bytes);
        if count_large_cws {
            BACKGROUND_LARGE_CWS_JOBS.fetch_add(1, Ordering::Relaxed);
        }
        result
    }).await
}

async fn blocking_when<T: Send + 'static>(
    _background: bool,
    f: impl FnOnce() -> T + Send + 'static,
) -> Result<T, &'static str> {
    #[cfg(not(target_family = "wasm"))]
    if _background {
        type Job = Box<dyn FnOnce() + Send>;
        static WORK: OnceLock<async_channel::Sender<Job>> = OnceLock::new();
        let queue = WORK.get_or_init(|| {
            let (sender, receiver) = async_channel::bounded::<Job>(16);
            for index in 0..2 {
                let receiver = receiver.clone();
                std::thread::Builder::new().name(format!("aoqi-resource-{index}")).spawn(move || {
                    while let Ok(job) = receiver.recv_blocking() { job(); }
                }).expect("resource worker thread");
            }
            sender
        });
        let (sender, receiver) = async_channel::bounded(1);
        queue.send(Box::new(move || {
            if sender.is_closed() { return; }
            let start = std::time::Instant::now();
            let value = f();
            BACKGROUND_JOBS.fetch_add(1, Ordering::Relaxed);
            BACKGROUND_NS.fetch_add(start.elapsed().as_nanos() as u64, Ordering::Relaxed);
            let _ = sender.try_send(value);
        })).await.map_err(|_| "Resource worker queue closed")?;
        return receiver.recv().await.map_err(|_| "Resource worker stopped");
    }
    Ok(f())
}

#[cfg(test)]
mod tests {
    use super::Mode;

    #[test]
    fn zws_background_can_be_selected_without_other_resource_work() {
        let control = Mode { sleep: true, loading: false, dedup: false, background_zws: false, background_cws: false };
        let zws = Mode { background_zws: true, ..control };
        let all_loading = Mode { loading: true, ..control };
        assert!(!control.background_for_movie(b"ZWS"));
        assert!(zws.background_for_movie(b"ZWS"));
        for body in [b"CWS".as_slice(), b"FWS", b"ZW", b""] {
            assert!(!zws.background_for_movie(body));
            assert!(all_loading.background_for_movie(body));
        }
    }

    #[test]
    fn large_cws_background_only_changes_fetched_movies() {
        let control = Mode { sleep: true, loading: false, dedup: false, background_zws: false, background_cws: false };
        let cws = Mode { background_cws: true, ..control };
        let loading = Mode { loading: true, ..control };
        let mut body = vec![0; 16 * 1024];
        body[..3].copy_from_slice(b"CWS");

        assert!(!control.background_for_large_cws());
        assert!(cws.background_for_large_cws());
        assert!(loading.background_for_large_cws());
        assert!(!control.background_for_movie(&body));
        assert!(!cws.background_for_movie(&body[..body.len() - 1]));
        assert!(cws.background_for_movie(&body));
        assert!(!cws.background_for_load_bytes(&body));
        assert!(loading.background_for_load_bytes(&body));
        for signature in [b"FWS", b"ZWS", b"PNG"] {
            body[..3].copy_from_slice(signature);
            assert!(!cws.background_for_movie(&body));
        }
    }
}
