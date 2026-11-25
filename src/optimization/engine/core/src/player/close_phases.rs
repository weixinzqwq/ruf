use std::sync::OnceLock;

pub(super) fn enabled() -> bool {
    static ENABLED: OnceLock<bool> = OnceLock::new();
    *ENABLED.get_or_init(|| std::env::var("AOLA_CLOSE_PHASES").as_deref() == Ok("1"))
}

pub(super) fn record(phase: &'static str, edge: &'static str) {
    if enabled() {
        tracing::info!(target: "aola_close_phase", phase, edge);
    }
}

pub(super) fn gc_drop_edge(starting: bool, remaining: usize) {
    let edge = if starting { "begin" } else { "end" };
    tracing::info!(target: "aola_close_phase", phase = "gc-drop-all", edge, nodes = remaining as u64);
}

// These markers observe Rust's field drop order without moving owners, taking
// extra references or releasing a resource earlier than the normal teardown.
pub(super) enum DropBoundary {
    AfterRenderer,
    AfterAudio,
    BeforeArena,
    AfterArena,
    AfterPlayerFields,
    BeforeRootLibrary,
    AfterRootLibrary,
}

impl Drop for DropBoundary {
    fn drop(&mut self) {
        match self {
            Self::AfterRenderer => {
                record("player-renderer-drop", "end");
                record("player-audio-drop", "begin");
            }
            Self::AfterAudio => record("player-audio-drop", "end"),
            Self::BeforeArena => record("player-arena-release", "begin"),
            Self::AfterArena => record("player-arena-release", "end"),
            Self::AfterPlayerFields => record("player-drop", "end"),
            Self::BeforeRootLibrary => record("gc-root-library-drop", "begin"),
            Self::AfterRootLibrary => record("gc-root-library-drop", "end"),
        }
    }
}
