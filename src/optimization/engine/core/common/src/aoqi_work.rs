//! Independent switches and counters for the third optimization round.
use std::sync::{OnceLock, atomic::{AtomicU64, Ordering}};

#[derive(Clone, Copy)]
pub struct Mode { pub sleep: bool, pub loading: bool, pub dedup: bool, background_zws: bool }

impl Mode {
    pub fn background_for_movie(self, bytes: &[u8]) -> bool {
        self.loading || (self.background_zws && bytes.starts_with(b"ZWS"))
    }
}

static MODE: OnceLock<Mode> = OnceLock::new();

fn read_mode(default: &str) -> Mode {
    let name = std::env::var("AOQI_STAGE3_MODE").unwrap_or_else(|_| default.into());
    Mode { sleep: matches!(name.as_str(), "all" | "sleep"),
        loading: matches!(name.as_str(), "all" | "loading"),
        dedup: matches!(name.as_str(), "all" | "dedup"),
        background_zws: std::env::var("AOLA_BACKGROUND_ZWS").as_deref() == Ok("1") }
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
    blocking_when(mode().background_for_movie(&bytes), move || decode(bytes)).await
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
        let control = Mode { sleep: true, loading: false, dedup: false, background_zws: false };
        let zws = Mode { background_zws: true, ..control };
        let all_loading = Mode { loading: true, ..control };
        assert!(!control.background_for_movie(b"ZWS"));
        assert!(zws.background_for_movie(b"ZWS"));
        for body in [b"CWS".as_slice(), b"FWS", b"ZW", b""] {
            assert!(!zws.background_for_movie(body));
            assert!(all_loading.background_for_movie(body));
        }
    }
}
