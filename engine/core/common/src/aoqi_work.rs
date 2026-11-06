//! Independent switches and counters for the third optimization round.
use std::sync::{OnceLock, atomic::{AtomicU64, Ordering}};

#[derive(Clone, Copy)]
pub struct Mode { pub sleep: bool, pub loading: bool, pub dedup: bool }

static MODE: OnceLock<Mode> = OnceLock::new();

fn read_mode(default: &str) -> Mode {
    let name = std::env::var("AOQI_STAGE3_MODE").unwrap_or_else(|_| default.into());
    Mode { sleep: matches!(name.as_str(), "all" | "sleep"),
        loading: matches!(name.as_str(), "all" | "loading"),
        dedup: matches!(name.as_str(), "all" | "dedup") }
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
    #[cfg(not(target_family = "wasm"))]
    if mode().loading {
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
