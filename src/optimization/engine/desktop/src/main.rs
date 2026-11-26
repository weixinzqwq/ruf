#![deny(clippy::unwrap_used)]
// By default, Windows creates an additional console window for our program.
//
//
// This is silently ignored on non-windows systems.
// See https://docs.microsoft.com/en-us/cpp/build/reference/subsystem?view=msvc-160 for details.
#![windows_subsystem = "windows"]

mod app;
#[cfg(windows)]
mod aola_host;
#[cfg(windows)]
mod mypro_host;
#[cfg(windows)]
mod mypro_services;
#[cfg(windows)]
mod mypro_features;
#[cfg(windows)]
mod aola_diagnostics;
#[cfg(windows)]
mod aola_close_phases;
#[cfg(windows)]
mod aoqi_host;
#[cfg(windows)]
mod aoqi_diagnostics;
#[cfg(windows)]
mod aoqi_web;
mod aoqi_storage;
mod aoqi_recovery;
mod aoqi_helper;
mod aoqi_shell;
mod backends;
mod cli;
mod custom_event;
mod dbus;
mod gui;
mod log;
mod player;
mod preferences;
#[cfg(feature = "tracy")]
mod tracy;
mod util;
#[cfg(windows)]
mod windows;
#[cfg(windows)]
mod window_move;

use crate::preferences::GlobalPreferences;
use anyhow::{Context, Error};
use app::App;
#[cfg(feature = "shell-completions")]
use clap::CommandFactory;
use clap::Parser;
use cli::Opt;
use rfd::MessageDialogResult;
use ruffle_core::StaticCallstack;
use std::cell::RefCell;
use std::env;
use std::fs::File;
use std::panic::PanicHookInfo;
use tracing_subscriber::fmt::Layer;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;
use url::Url;

thread_local! {
    static CALLSTACK: RefCell<Option<StaticCallstack>> = RefCell::default();
    static RENDER_INFO: RefCell<Option<String>> = RefCell::default();
    static SWF_INFO: RefCell<Option<String>> = RefCell::default();
}

#[cfg(feature = "tracy")]
#[global_allocator]
static GLOBAL: tracing_tracy::client::ProfiledAllocator<std::alloc::System> =
    tracing_tracy::client::ProfiledAllocator::new(std::alloc::System, 0);

static RUFFLE_VERSION: &str = concat!(
    env!("CARGO_PKG_VERSION"),
    "-",
    env!("CFG_RELEASE_CHANNEL"),
    " (",
    env!("VERGEN_GIT_SHA"),
    " ",
    env!("VERGEN_GIT_COMMIT_DATE"),
    ")"
);

fn truncate_to_line_boundary(text: String, max_len: usize) -> String {
    if text.len() <= max_len {
        return text;
    }
    let mut truncated = String::with_capacity(max_len + 10);
    for line in text.lines() {
        if truncated.len() + line.len() > max_len {
            break;
        }
        truncated.push_str(line);
        truncated.push('\n');
    }
    format!("{truncated}[...]")
}

fn panic_hook(info: &PanicHookInfo) {
    CALLSTACK.with(|callstack| {
        if let Some(callstack) = &*callstack.borrow() {
            callstack.avm2(|callstack| println!("AVM2 stack trace: {callstack}"))
        }
    });

    let message = info.payload_as_str().unwrap_or("panic occurred");

    if rfd::MessageDialog::new()
        .set_level(rfd::MessageLevel::Error)
        .set_title("Ruffle")
        .set_description(format!(
            "Ruffle has encountered a fatal error, this is a bug.\n\n\
            {message}\n\n\
            Please report this to us so that we can fix it. Thank you!\n\
            Pressing Yes will open a browser window."
        ))
        .set_buttons(rfd::MessageButtons::YesNo)
        .show()
        == MessageDialogResult::Yes
    {
        let backtrace = std::backtrace::Backtrace::force_capture();
        let panic_text = format!("```\n{info}\n\nStack trace:\n{backtrace}\n```");

        // The URL cannot be too long.
        let panic_text = truncate_to_line_boundary(panic_text, 2048);

        let mut params = vec![
            ("panic_text", panic_text),
            ("platform", "Desktop app".to_string()),
            ("operating_system", os_info::get().to_string()),
            ("ruffle_version", RUFFLE_VERSION.to_string()),
        ];
        let mut extra_info = vec![];
        SWF_INFO.with(|i| {
            if let Some(swf_name) = i.take() {
                extra_info.push(format!("Filename: {swf_name}\n"));
                params.push(("title", format!("Crash on {swf_name}")));
            }
        });
        CALLSTACK.with(|callstack| {
            if let Some(callstack) = &*callstack.borrow() {
                callstack.avm2(|callstack| {
                    extra_info.push(format!("### AVM2 Callstack\n```{callstack}\n```\n"));
                });
            }
        });
        RENDER_INFO.with(|i| {
            if let Some(render_info) = i.take() {
                extra_info.push(format!("### Render Info\n{render_info}\n"));
            }
        });
        if !extra_info.is_empty() {
            params.push(("extra_info", extra_info.join("\n")));
        }
        if let Ok(url) = Url::parse_with_params(
            "https://github.com/ruffle-rs/ruffle/issues/new?assignees=&labels=bug&template=crash_report.yml",
            &params,
        ) {
            let _ = webbrowser::open(url.as_str());
        }
    }
}

fn main() -> Result<(), Error> {
    if std::env::current_exe().ok().and_then(|p| p.file_name().map(|v| v.to_string_lossy().into_owned()))
        .is_some_and(|name| name.eq_ignore_ascii_case("aoqi-allocation-pool-test.exe") || name.eq_ignore_ascii_case("aoqi-pool-reuse-test.exe") || name.eq_ignore_ascii_case("aoqi-gpu-cache-test.exe") || name.eq_ignore_ascii_case("aoqi-command-submit-test.exe") || name.eq_ignore_ascii_case("aoqi-command-maintenance-test.exe") || name.eq_ignore_ascii_case("aoqi-command-encode-test.exe")
                || name.eq_ignore_ascii_case("aoqi-command-batch-test.exe")) {
        // No threads have been started at this entry point.
        unsafe {
            std::env::set_var("AOQI_STAGE3_MODE", "baseline");
            std::env::set_var("AOQI_POOL_IDLE_BUDGET_MB", "256");
            std::env::set_var("AOQI_POOL_RECENT_FRAMES", "120");
        }
    }

    let prev_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        prev_hook(info);
        panic_hook(info);
    }));

    #[cfg(windows)]
    let _console = windows::Console::attach();

    let mut opt = Opt::parse();
    #[cfg(windows)]
    if std::env::current_exe().ok().and_then(|p|p.file_name().map(|n|n.to_string_lossy().into_owned()))
        .is_some_and(|name|name.eq_ignore_ascii_case("aola-native-test.exe")) {
        opt.aola_host = true;
    }
    #[cfg(windows)]
    if opt.aola_host {
        anyhow::ensure!(!opt.aoqi_host,"Choose one game host");
        ruffle_core::aoqi_work::initialize_aola_mode();
        let root=opt.aola_data_dir.clone().unwrap_or_else(|| std::env::current_exe().unwrap().parent().unwrap().join("aola-data"));
        std::fs::create_dir_all(&root)?;
        let root=std::fs::canonicalize(root)?;
        opt.aola_data_dir=Some(root.clone());
        opt.config=root.join("config");
        opt.cache_directory=root.join("cache");
        opt.save_directory=root.join("shared-objects");
        opt.storage=Some(crate::preferences::storage::StorageBackend::Memory);
        opt.graphics=Some(ruffle_render_wgpu::clap::GraphicsBackend::Dx12);
        opt.quality=Some(ruffle_render::quality::StageQuality::High);
        opt.frame_rate.get_or_insert(60.0);
        let local_tools_movie = mypro_host::enabled()
            && opt.movie_url.as_ref().is_some_and(|url| url.scheme() == "file");
        if !local_tools_movie {
            opt.tcp_connections=Some(ruffle_core::backend::navigator::SocketMode::Allow);
        }
        opt.referer=Some(Url::parse(aola_host::PAGE_URL)?);
        if !local_tools_movie { opt.movie_url=None; }
        opt.no_gui=true;
        aola_host::initialize(&root)?;
    }
    if std::env::current_exe().ok().and_then(|path|path.file_name().map(|name|name.to_string_lossy().into_owned()))
        .is_some_and(|name|name.eq_ignore_ascii_case("aoqi-memory-test.exe") || name.eq_ignore_ascii_case("aoqi-performance-test.exe") || name.eq_ignore_ascii_case("aoqi-cache-lifetime-test.exe") || name.eq_ignore_ascii_case("aoqi-render-timing-test.exe") || name.eq_ignore_ascii_case("aoqi-backend-submit-test.exe") || name.eq_ignore_ascii_case("aoqi-submit-rollback-test.exe") || name.eq_ignore_ascii_case("aoqi-glow-reuse-test.exe") || name.eq_ignore_ascii_case("aoqi-bounded-blend-test.exe") || name.eq_ignore_ascii_case("aoqi-filter-submit-test.exe") || name.eq_ignore_ascii_case("aoqi-stage3-test.exe") || (name.eq_ignore_ascii_case("aoqi-allocation-pool-test.exe") || name.eq_ignore_ascii_case("aoqi-pool-reuse-test.exe") || name.eq_ignore_ascii_case("aoqi-gpu-cache-test.exe") || name.eq_ignore_ascii_case("aoqi-command-submit-test.exe") || name.eq_ignore_ascii_case("aoqi-command-maintenance-test.exe") || name.eq_ignore_ascii_case("aoqi-command-encode-test.exe")
                || name.eq_ignore_ascii_case("aoqi-command-batch-test.exe"))) {
        opt.aoqi_host = true;
    }
    if opt.aoqi_host {
        // Fixed comparison profile for the memory-regression build.
        opt.quality = Some(ruffle_render::quality::StageQuality::High);
        // The recovered FrameRateControl applies the selected target in AS3.
        // A native override would silently ignore game Stage.frameRate writes.
        opt.frame_rate = None;
        #[cfg(windows)]
        { opt.graphics = Some(ruffle_render_wgpu::clap::GraphicsBackend::Dx12); }
        let root = opt.aoqi_data_dir.clone().unwrap_or_else(|| {
            std::env::current_exe().expect("Executable path").parent().expect("Executable directory").join("aoqi-data")
        });
        std::fs::create_dir_all(&root)?;
        let root = std::fs::canonicalize(root)?;
        opt.aoqi_data_dir = Some(root.clone());
        opt.config = root.join("config");
        opt.save_directory = root.join("shared-objects");
        opt.cache_directory = root.join("runtime-cache");
        opt.no_gui = false;
        if opt.movie_url.is_none() {
            opt.movie_url = Some(Url::parse("aoqi:///host.swf")?);
        }
        if let Some(source) = &opt.aoqi_migrate_cache_from {
            let report = aoqi_storage::migrate_cache(source, &root.join("cache")).map_err(anyhow::Error::msg)?;
            let storage = aoqi_storage::Storage::new(root).map_err(anyhow::Error::msg)?;
            storage.write_json(".host-state/cache-migration.json", &report).map_err(anyhow::Error::msg)?;
        }
    }

    #[cfg(feature = "shell-completions")]
    if let Some(shell) = opt.completions {
        let mut cmd = Opt::command();
        clap_complete::generate(shell, &mut cmd, "ruffle", &mut std::io::stdout());
        return Ok(());
    }

    let preferences = GlobalPreferences::load(opt)?;
    #[cfg(windows)]
    aola_close_phases::initialize(preferences.cli.aola_data_dir.as_deref())?;
    #[cfg(windows)]
    if aola_host::enabled() || aola_close_phases::configured() {
        use tracing_subscriber::Layer as _;
        if aola_host::enabled() { std::panic::set_hook(Box::new(aola_host::record_panic)); }
        tracing_subscriber::registry().with(aola_host::DiagnosticLayer.with_filter(
            tracing_subscriber::filter::filter_fn(aola_host::diagnostic_enabled))).init();
        aola_host::record("host","starting");
        return App::new(preferences).and_then(|(mut app,event_loop)| {
            let result = event_loop.run_app(&mut app).context("Event loop failure");
            mypro_features::network::finish();
            aola_close_phases::record("event-loop-tail", "end", None);
            {
                let _phase = aola_close_phases::phase("app-drop");
                drop(app);
            }
            aola_close_phases::finish();
            result
        });
    }

    let logs_path = &preferences.cli.cache_directory.join("log");
    let log_path = preferences.log_filename_pattern().create_path(logs_path);
    if let Some(parent) = log_path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    if let Err(err) = migrate_logs(&preferences, logs_path) {
        tracing::warn!("Failed to migrate logs: {}", err);
    }

    // [NA] `_guard` cannot be `_` or it'll immediately drop
    // https://docs.rs/tracing-appender/latest/tracing_appender/non_blocking/index.html
    let (non_blocking_file, _file_guard) = tracing_appender::non_blocking(File::create(log_path)?);
    let (non_blocking_stdout, _stdout_guard) = tracing_appender::non_blocking(std::io::stdout());

    let env_filter = tracing_subscriber::EnvFilter::builder().parse_lossy(
        env::var("RUST_LOG")
            .as_deref()
            .unwrap_or("warn,ruffle=info,avm_trace=info"),
    );

    let subscriber = tracing_subscriber::registry()
        .with(env_filter)
        .with(Layer::new().with_writer(non_blocking_stdout))
        .with(Layer::new().with_writer(non_blocking_file).with_ansi(false));

    #[cfg(feature = "tracy")]
    let subscriber = {
        let tracy_subscriber = tracing_tracy::TracyLayer::new(tracy::RuffleTracyConfig::default());
        subscriber.with(tracy_subscriber)
    };

    subscriber.init();

    let result = App::new(preferences).and_then(|(mut app, event_loop)| {
        event_loop.run_app(&mut app).context("Event loop failure")
    });
    #[cfg(windows)]
    mypro_features::network::finish();

    #[cfg(windows)]
    if let Err(error) = &result {
        eprintln!("{:?}", error)
    }

    result
}

/// Move logs from config directory into proper log directory.
///
/// This exists because in older versions Ruffle created log files in the config directory.
///
/// TODO Remove this after some time.
fn migrate_logs(
    preferences: &GlobalPreferences,
    log_path: &std::path::Path,
) -> std::io::Result<()> {
    tracing::debug!("Migrating logs from config directory to log directory");
    let config_path = &preferences.cli.config;
    let paths = std::fs::read_dir(config_path)?;

    for dir in paths.flatten() {
        if dir.file_name().as_encoded_bytes().ends_with(b".log") {
            let src = dir.path();
            let dest = log_path.join(dir.file_name());
            tracing::info!(
                "Moving log file {} -> {}",
                src.to_string_lossy(),
                dest.to_string_lossy()
            );
            if let Err(err) = std::fs::rename(&src, &dest) {
                tracing::warn!(
                    "Failed to move log file {} -> {}: {err}",
                    src.to_string_lossy(),
                    dest.to_string_lossy()
                )
            }
        }
    }

    Ok(())
}
