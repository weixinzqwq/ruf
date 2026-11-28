use crate::backends::{
    DesktopExternalInterfaceProvider, DesktopFSCommandProvider, DesktopNavigatorInterface,
    DesktopUiBackend, PathAllowList,
};
use crate::cli::FilesystemAccessMode;
use crate::cli::GameModePreference;
use crate::custom_event::RuffleEvent;
use crate::gui::{FilePicker, GuiController, MovieView};
use crate::preferences::GlobalPreferences;
use crate::{CALLSTACK, RENDER_INFO, SWF_INFO};
use anyhow::anyhow;
use ruffle_core::backend::navigator::{OwnedFuture, SocketMode};
use ruffle_core::config::Letterbox;
use ruffle_core::events::{GamepadButton, KeyCode};
use ruffle_core::font::DefaultFont;
use ruffle_core::{LoadBehavior, Player, PlayerBuilder, PlayerEvent};
use ruffle_frontend_utils::backends::audio::CpalAudioBackend;
use ruffle_frontend_utils::backends::navigator::{AolaSwfMode, ExternalNavigatorBackend, FutureSpawner};
use ruffle_frontend_utils::bundle::source::BundleSourceError;
use ruffle_frontend_utils::bundle::{Bundle, BundleError};
use ruffle_frontend_utils::content::{ContentDescriptor, PlayingContent};
use ruffle_frontend_utils::player_options::PlayerOptions;
use ruffle_frontend_utils::recents::Recent;
use ruffle_render::backend::RenderBackend;
use ruffle_render::quality::StageQuality;
use ruffle_render_wgpu::backend::WgpuRenderBackend;
use ruffle_render_wgpu::aoqi::BitmapUpscale;
use ruffle_render_wgpu::clap::PowerPreference;
use ruffle_render_wgpu::descriptors::Descriptors;
use std::borrow::Cow;
use std::collections::{HashMap, HashSet};
use std::fmt::Debug;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;
use url::Url;
use winit::event_loop::EventLoopProxy;
use winit::window::Window;

/// Options used when creating a Player (& passed through to a PlayerBuilder).
/// These may be primed by command line arguments.
#[derive(Debug, Clone)]
pub struct LaunchOptions {
    pub player: PlayerOptions,
    pub proxy: Option<Url>,
    pub socket_allowed: HashSet<String>,
    pub tcp_connections: Option<SocketMode>,
    pub fullscreen: bool,
    pub save_directory: PathBuf,
    pub cache_directory: PathBuf,
    pub filesystem_access_mode: FilesystemAccessMode,
    pub gamepad_button_mapping: HashMap<GamepadButton, KeyCode>,
    pub avm2_optimizer_enabled: bool,
}

impl From<&GlobalPreferences> for LaunchOptions {
    fn from(value: &GlobalPreferences) -> Self {
        Self {
            player: PlayerOptions {
                parameters: value.cli.parameters().collect(),
                max_execution_duration: value.cli.max_execution_duration,
                base: value.cli.base.clone(),
                quality: value.cli.quality,
                align: value.cli.align,
                force_align: if value.cli.force_align {
                    Some(true)
                } else {
                    None
                },
                scale: value.cli.scale,
                force_scale: if value.cli.force_scale {
                    Some(true)
                } else {
                    None
                },
                upgrade_to_https: if value.cli.upgrade_to_https {
                    Some(true)
                } else {
                    None
                },
                load_behavior: value.cli.load_behavior,
                letterbox: value.cli.letterbox,
                spoof_url: value.cli.spoof_url.clone(),
                referer: value.cli.referer.clone(),
                cookie: value.cli.cookie.clone(),
                player_version: value.cli.player_version,
                player_runtime: value.cli.player_runtime,
                frame_rate: value.cli.frame_rate,
                dummy_external_interface: if value.cli.dummy_external_interface {
                    Some(true)
                } else {
                    None
                },
            },
            proxy: value.cli.proxy.clone(),
            fullscreen: value.cli.fullscreen,
            save_directory: value.cli.save_directory.clone(),
            cache_directory: value.cli.cache_directory.clone(),
            filesystem_access_mode: value.cli.filesystem_access_mode,
            socket_allowed: HashSet::from_iter(value.cli.socket_allow.iter().cloned()),
            tcp_connections: value.cli.tcp_connections,
            gamepad_button_mapping: HashMap::from_iter(value.cli.gamepad_button.iter().cloned()),
            avm2_optimizer_enabled: !value.cli.no_avm2_optimizer,
        }
    }
}

/// Represents a current Player and any associated state with that player,
/// which may be lost when this Player is closed (dropped)
struct ActivePlayer {
    id: PlayerId,
    player: Arc<Mutex<Player>>,

    #[cfg(target_os = "linux")]
    _gamemode_session: crate::dbus::GameModeSession,
}

impl ActivePlayer {
    #[expect(clippy::too_many_arguments)]
    pub fn new(
        opt: &LaunchOptions,
        event_loop: EventLoopProxy<RuffleEvent>,
        content_descriptor: &ContentDescriptor,
        window: Arc<Window>,
        descriptors: Arc<Descriptors>,
        movie_view: MovieView,
        font_database: Rc<fontdb::Database>,
        preferences: GlobalPreferences,
        file_picker: FilePicker,
    ) -> Self {
        let player_id = PlayerId::new();
        let aoqi_host_enabled = preferences.cli.aoqi_host;
        let mut builder = PlayerBuilder::new();

        match CpalAudioBackend::new(preferences.output_device_name().as_deref()) {
            Ok(audio) => {
                builder = builder.with_audio(audio);
            }
            Err(e) => {
                tracing::error!("Unable to create audio device: {}", e);
            }
        };

        let mut content = PlayingContent::DirectFile(content_descriptor.clone());
        if content_descriptor.url.scheme() == "file"
            && let Ok(path) = content_descriptor.url.to_file_path()
        {
            match Bundle::from_path(&path) {
                Ok(bundle) => {
                    if bundle.warnings().is_empty() {
                        tracing::info!("Opening bundle at {path:?}");
                    } else {
                        // TODO: Show warnings to user (toast?)
                        tracing::warn!("Opening bundle at {path:?} with warnings");
                        for warning in bundle.warnings() {
                            tracing::warn!("{warning}");
                        }
                    }
                    content = PlayingContent::Bundle(content_descriptor.clone(), Box::new(bundle));
                }
                Err(
                    BundleError::BundleDoesntExist
                    | BundleError::InvalidSource(BundleSourceError::UnknownSource),
                ) => {
                    // Do nothing and carry on opening it as a swf - this likely isn't a bundle at all
                }
                Err(e) => {
                    // TODO: Visible popup when a bundle (or regular file) fails to open
                    tracing::error!("Couldn't open bundle at {path:?}: {e}");
                }
            }
        }

        let recent_limit = preferences.recent_limit();
        if let Err(e) = preferences.write_recents(|writer| {
            writer.push(
                Recent {
                    content_descriptor: content_descriptor.clone(),
                    name: content.name(),
                },
                recent_limit,
            )
        }) {
            tracing::warn!("Couldn't update recents: {e}");
        }

        let opt = match &content {
            PlayingContent::DirectFile(_) => Cow::Borrowed(opt),
            PlayingContent::Bundle(_, bundle) => {
                let player = opt.player.or(&bundle.information().player);

                Cow::Owned(LaunchOptions {
                    player,
                    proxy: opt.proxy.clone(),
                    socket_allowed: opt.socket_allowed.clone(),
                    tcp_connections: opt.tcp_connections,
                    fullscreen: opt.fullscreen,
                    save_directory: opt.save_directory.clone(),
                    cache_directory: opt.cache_directory.clone(),
                    filesystem_access_mode: opt.filesystem_access_mode,
                    gamepad_button_mapping: opt.gamepad_button_mapping.clone(),
                    avm2_optimizer_enabled: opt.avm2_optimizer_enabled,
                })
            }
        };

        let future_spawner = WinitExecutor {
            event_loop: event_loop.clone(),
            player_id,
        };
        let movie_url = content.initial_swf_url().clone();
        let readable_name = content.name();
        let initial_allow_list = PathAllowList::new(content_descriptor);
        let navigator = ExternalNavigatorBackend::new(
            opt.player
                .base
                .to_owned()
                .unwrap_or_else(|| movie_url.clone()),
            opt.player.referer.clone(),
            opt.player.cookie.clone(),
            future_spawner,
            opt.proxy.clone(),
            opt.player.upgrade_to_https.unwrap_or_default(),
            opt.socket_allowed.clone(),
            opt.tcp_connections.unwrap_or(SocketMode::Ask),
            Rc::new(content),
            DesktopNavigatorInterface::new(
                preferences.clone(),
                event_loop.clone(),
                initial_allow_list,
                opt.filesystem_access_mode,
            ),
        );
        let navigator = if let Some(root) = preferences.cli.aoqi_data_dir.as_ref().filter(|_| aoqi_host_enabled) {
            navigator.with_aoqi_cache(root.join("cache"))
                .expect("Cannot initialize the application resource cache")
        } else { navigator };

        let navigator = if let Some(root)=preferences.cli.aola_data_dir.as_ref().filter(|_|preferences.cli.aola_host) {
            navigator.with_aola_cache(root.join("cache")).expect("Cannot initialize Aola resource cache")
                .with_aola_weekly_patches(root)
        } else {navigator};
        let hide_notice = preferences.cli.aola_host
            && std::env::var_os("AOLA_HIDE_NOTICE").as_deref() == Some(std::ffi::OsStr::new("1"));
        let block_skill = preferences.cli.aola_host
            && std::env::var_os("AOLA_BLOCK_SKILL").as_deref() == Some(std::ffi::OsStr::new("1"));
        let hide_transform_portrait = preferences.cli.aola_host
            && std::env::var_os("AOLA_HIDE_TRANSFORM_PORTRAIT").as_deref() == Some(std::ffi::OsStr::new("1"));
        let skip_battle_backdrop = preferences.cli.aola_host
            && std::env::var_os("AOLA_SKIP_BATTLE_BACKDROP").as_deref() == Some(std::ffi::OsStr::new("1"));
        // Tool settings are consulted at the SWF call site, including after startup.
        let live_display = preferences.cli.aola_host && std::env::var("AOLA_MYPRO_TOOLS").as_deref() == Ok("1");
        let patch_battle = live_display || block_skill || hide_transform_portrait || skip_battle_backdrop;
        let mut navigator = navigator;
        if hide_notice || patch_battle {
            let exe = std::env::current_exe().ok();
            let root = exe.as_deref().and_then(std::path::Path::parent);
            if live_display && let Some(root) = root {
                navigator = navigator.with_aola_game_ui(root.join("swf-game-ui"));
            }
            if live_display || hide_notice {
                navigator = navigator.with_aola_notice(root.map(|r| r.join("swf-notice")).as_deref());
            }
            if patch_battle {
                navigator = navigator.with_aola_battle(root.map(|r| r.join("swf-battle")).as_deref());
            }
        }
        let navigator = if preferences.cli.aola_host {
            let legacy = std::env::var("AOLA_PET_PATCH").unwrap_or_default();
            let lifecycle = std::env::var("AOLA_SWF_LIFECYCLE_MODE").unwrap_or_default();
            match AolaSwfMode::select(&lifecycle, &legacy) {
                AolaSwfMode::Official => {
                    if lifecycle == "official" {
                        tracing::info!(target: "aola_swf_lifecycle", mode="official", source="config", result="official");
                    }
                    if std::env::var("AOLA_OPTIMIZE_BACKPACK").as_deref() == Ok("1") {
                        let bundle = std::env::current_exe().ok()
                            .and_then(|exe| exe.parent().map(|root| root.join("swf-backpack")));
                        navigator.with_aola_backpack(bundle.as_deref())
                    } else { navigator }
                }
                AolaSwfMode::Legacy { optimized } => {
                    let root = std::env::current_exe().expect("Executable path").parent().expect("Executable directory").join("swf");
                    navigator.with_aola_pet_patch(&root, optimized).expect("Validated Aola pet SWF files required")
                }
                AolaSwfMode::Lifecycle { mode, conflict } => {
                    if let Some(data) = preferences.cli.aola_data_dir.as_ref() {
                        let root = std::env::current_exe().expect("Executable path").parent().expect("Executable directory").join("swf-lifecycle");
                        navigator.with_aola_lifecycle(mode, &root, &data.join("swf-capture"), conflict)
                    } else {
                        tracing::info!(target: "aola_swf_lifecycle", mode="unavailable", source="config", result="missing-data-directory");
                        navigator
                    }
                }
            }
        } else { navigator };
        if cfg!(feature = "external_video") && preferences.openh264_enabled() {
            #[cfg(feature = "external_video")]
            {
                use ruffle_video_external::{
                    backend::ExternalVideoBackend, decoder::openh264::OpenH264Codec,
                };
                let openh264 = tokio::task::block_in_place(|| {
                    OpenH264Codec::load(&opt.cache_directory.join("video"))
                });
                let backend = match openh264 {
                    Ok(codec) => ExternalVideoBackend::new_with_openh264(codec),
                    Err(e) => {
                        tracing::error!("Failed to load OpenH264: {}", e);
                        ExternalVideoBackend::new()
                    }
                };
                builder = builder.with_video(backend);
            }
        } else {
            #[cfg(feature = "software_video")]
            {
                builder =
                    builder.with_video(ruffle_video_software::backend::SoftwareVideoBackend::new());
            }
        }

        #[cfg_attr(not(target_os = "linux"), allow(unused))]
        let gamemode_enable = match preferences.gamemode_preference() {
            GameModePreference::Default => {
                preferences.graphics_power_preference() == PowerPreference::High
            }
            GameModePreference::On => {
                if cfg!(not(target_os = "linux")) {
                    tracing::warn!("Cannot enable GameMode, as it is supported only on Linux");
                }
                true
            }
            GameModePreference::Off => false,
        };

        let quality = opt.player.quality.unwrap_or(StageQuality::High);
        let mut renderer = WgpuRenderBackend::new_with_quality(descriptors, movie_view, quality)
            .map_err(|e| anyhow!(e.to_string()))
            .expect("Couldn't create wgpu rendering backend");
        if aoqi_host_enabled {
            renderer.set_bitmap_upscale(BitmapUpscale::Off);
        }
        RENDER_INFO.with(|i| *i.borrow_mut() = Some(renderer.debug_info().to_string()));

        if opt.player.dummy_external_interface.unwrap_or_default() {
            builder = builder.with_external_interface(Box::new(DesktopExternalInterfaceProvider {
                spoof_url: opt.player.spoof_url.clone(),
            }));
        }
        #[cfg(windows)]
        if let Some(provider) = crate::aoqi_host::provider() {
            builder = builder.with_external_interface(provider);
        }

        #[cfg(windows)]
        if crate::aola_host::enabled() {
            builder = builder.with_external_interface(Box::new(crate::aola_host::AolaInterface::new(
                hide_notice, block_skill, hide_transform_portrait, skip_battle_backdrop,
            )));
        }
        if !opt.gamepad_button_mapping.is_empty() {
            builder = builder.with_gamepad_button_mapping(opt.gamepad_button_mapping.clone());
        }

        let (notification_sender, notification_recv) = async_channel::unbounded();

        let event_loop2 = event_loop.clone();
        tokio::spawn(async move {
            while let Ok(notification) = notification_recv.recv().await {
                let event = RuffleEvent::PlayerNotification(notification);
                match event_loop2.send_event(event) {
                    Ok(_) => continue,
                    Err(_) => break,
                }
            }
        });

        builder = builder
            .with_navigator(navigator)
            .with_renderer(renderer)
            .with_storage(preferences.storage_backend().create_backend(&opt))
            .with_notification_sender(notification_sender)
            .with_fs_commands(Box::new(DesktopFSCommandProvider {
                event_loop: event_loop.clone(),
            }))
            .with_ui(
                DesktopUiBackend::new(
                    window.clone(),
                    event_loop.clone(),
                    font_database,
                    preferences,
                    file_picker,
                )
                .expect("Couldn't create ui backend"),
            )
            .with_autoplay(true)
            .with_letterbox(opt.player.letterbox.unwrap_or(Letterbox::On))
            .with_max_execution_duration(opt.player.max_execution_duration.unwrap_or(Duration::MAX))
            .with_quality(quality)
            .with_align(
                opt.player.align.unwrap_or_default(),
                opt.player.force_align.unwrap_or_default(),
            )
            .with_scale_mode(
                opt.player.scale.unwrap_or_default(),
                opt.player.force_scale.unwrap_or_default(),
            )
            .with_fullscreen(opt.fullscreen)
            .with_load_behavior(opt.player.load_behavior.unwrap_or(LoadBehavior::Streaming))
            .with_spoofed_url(opt.player.spoof_url.clone().map(|url| url.to_string()))
            .with_page_url(opt.player.spoof_url.clone().map(|url| url.to_string()))
            .with_player_version(opt.player.player_version)
            .with_player_runtime(opt.player.player_runtime.unwrap_or_default())
            .with_frame_rate(if aoqi_host_enabled { None } else { opt.player.frame_rate })
            .with_avm2_optimizer_enabled(opt.avm2_optimizer_enabled);
        if aoqi_host_enabled {
            let (width, height) = GuiController::AOQI_RENDER_SIZE;
            builder = builder.with_viewport_dimensions(width, height, 1.0);
        }
        let player = builder.build();
        #[cfg(windows)]
        if crate::aola_host::enabled() {
            let mut player = player.lock().unwrap();
            player.enable_tick_profiling();
            let mode = std::env::var("AOLA_DETAIL_PROFILE").ok();
            player.set_pet_detail_mode(ruffle_core::pet_detail_profile::DetailMode::from_setting(mode.as_deref()));
        }
        #[cfg(windows)]
        crate::aoqi_host::attach(&player);

        if aoqi_host_enabled {
            window.set_title("Air奥奇传说 · 纹理池与占用测试版");
        } else {
            window.set_title(&format!("Ruffle - {readable_name}"));
        }

        #[cfg(windows)]
        if crate::aola_host::enabled() { window.set_title("奥拉星 · 原生迁移测试版"); }
        SWF_INFO.with(|i| *i.borrow_mut() = Some(readable_name));

        let on_metadata = move |swf_header: &ruffle_core::swf::HeaderExt| {
            let _ = event_loop.send_event(RuffleEvent::OnMetadata(swf_header.clone()));
        };

        {
            let mut player_lock = player.lock().expect("Player lock must be available");
            CALLSTACK.with(|callstack| {
                *callstack.borrow_mut() = Some(player_lock.callstack());
            });
            player_lock.fetch_root_movie(
                movie_url.to_string(),
                opt.player.parameters.to_owned(),
                Box::new(on_metadata),
            );

            player_lock.set_default_font(
                DefaultFont::Serif,
                vec![
                    "Times New Roman".into(),
                    "Tinos".into(),
                    "Liberation Serif".into(),
                    "DejaVu Serif".into(),
                ],
            );
            player_lock.set_default_font(
                DefaultFont::Sans,
                vec![
                    "Arial".into(),
                    "Arimo".into(),
                    "Liberation Sans".into(),
                    "DejaVu Sans".into(),
                ],
            );
            player_lock.set_default_font(
                DefaultFont::Typewriter,
                vec![
                    "Courier New".into(),
                    "Cousine".into(),
                    "Liberation Mono".into(),
                    "DejaVu Sans Mono".into(),
                ],
            );
            player_lock.set_default_font(
                DefaultFont::JapaneseGothic,
                vec![
                    "ヒラギノ角ゴ Pro W3".into(), // Mac with Japanese environment
                    "MS UI Gothic".into(),        // Windows
                    "Noto Sans CJK JP".into(),    // Linux
                    "Arial Unicode MS".into(),    // Mac fallback
                ],
            );
            player_lock.set_default_font(
                DefaultFont::JapaneseGothicMono,
                vec![
                    "Osaka－等幅".into(),      // Mac with Japanese environment
                    "MS Gothic".into(),        // Windows
                    "Noto Sans CJK JP".into(), // Linux
                    "Arial Unicode MS".into(), // Mac fallback
                ],
            );
            player_lock.set_default_font(
                DefaultFont::JapaneseMincho,
                vec![
                    "ヒラギノ明朝 Pro W3".into(), // Mac with Japanese environment
                    "MS PMincho".into(),          // Windows
                    "Noto Sans CJK JP".into(),    // Linux
                    "Arial Unicode MS".into(),    // Mac fallback
                ],
            );
        }

        Self {
            id: player_id,
            player,
            #[cfg(target_os = "linux")]
            _gamemode_session: crate::dbus::GameModeSession::new(gamemode_enable),
        }
    }
}

/// Owner of a Ruffle Player (via ActivePlayer),
/// responsible for either creating, destroying or communicating with that player.
pub struct PlayerController {
    player: Option<ActivePlayer>,
    event_loop: EventLoopProxy<RuffleEvent>,
    window: Arc<Window>,
    descriptors: Arc<Descriptors>,
    font_database: Rc<fontdb::Database>,
    preferences: GlobalPreferences,
    file_picker: FilePicker,
}

impl PlayerController {
    pub fn new(
        event_loop: EventLoopProxy<RuffleEvent>,
        window: Arc<Window>,
        descriptors: Arc<Descriptors>,
        font_database: fontdb::Database,
        preferences: GlobalPreferences,
        file_picker: FilePicker,
    ) -> Self {
        Self {
            player: None,
            event_loop,
            window,
            descriptors,
            font_database: Rc::new(font_database),
            preferences,
            file_picker,
        }
    }

    pub fn create(
        &mut self,
        opt: &LaunchOptions,
        content_descriptor: &ContentDescriptor,
        movie_view: MovieView,
    ) {
        self.player = Some(ActivePlayer::new(
            opt,
            self.event_loop.clone(),
            content_descriptor,
            self.window.clone(),
            self.descriptors.clone(),
            movie_view,
            self.font_database.clone(),
            self.preferences.clone(),
            self.file_picker.clone(),
        ));
    }

    pub fn destroy(&mut self) {
        self.player = None;
    }

    #[cfg(windows)]
    pub(crate) fn drop_with_close_timings(self) {
        {
            let _phase = crate::aola_close_phases::phase("player-controller-player-drop");
            drop(self.player);
        }
        drop(self.event_loop);
        {
            let _phase = crate::aola_close_phases::phase("player-controller-window-drop");
            drop(self.window);
        }
        {
            let _phase = crate::aola_close_phases::phase("player-controller-descriptors-drop");
            drop(self.descriptors);
        }
        {
            let _phase = crate::aola_close_phases::phase("player-controller-font-database-drop");
            drop(self.font_database);
        }
    }

    #[cfg(windows)]
    pub fn session_id(&self) -> Option<i64> {
        self.player.as_ref().map(|player| player.id.0)
    }

    pub fn get(&self) -> Option<MutexGuard<'_, Player>> {
        match &self.player {
            None => None,
            // We don't want to return None when the lock fails to grab as that's a fatal error, not a lack of player
            Some(player) => Some(
                player
                    .player
                    .try_lock()
                    .expect("Player lock must be available"),
            ),
        }
    }

    pub fn handle_event(&self, event: PlayerEvent) -> bool {
        if let Some(mut player) = self.get()
            && player.is_playing()
        {
            return player.handle_event(event);
        }

        false
    }

    pub fn poll(&self, task: PlayerRunnable) {
        // Only run the task if it matches our current player;
        // otherwise it is stale, and should be cancelled (which
        // happens implicitly on drop).
        if let Some(player) = &self.player
            && *task.0.metadata() == player.id
        {
            task.0.run();
        }
    }
}

/// A unique identifier for a given `Player` instance.
/// Used to track which player any currently executing future is bound to.
#[derive(Copy, Clone, Eq, PartialEq)]
struct PlayerId(i64);

impl PlayerId {
    fn new() -> Self {
        use std::sync::atomic::{AtomicI64, Ordering};

        static NEXT: AtomicI64 = AtomicI64::new(0);
        let id = NEXT.fetch_add(1, Ordering::Relaxed);
        assert!(id >= 0, "PlayerId overflowed!");
        Self(id)
    }
}

/// A `Player`-bound future that is currently running.
pub struct PlayerRunnable(async_task::Runnable<PlayerId>);

/// A bare-bones executor that schedules tasks on the winit event loop.
struct WinitExecutor {
    event_loop: EventLoopProxy<RuffleEvent>,
    player_id: PlayerId,
}

impl<E: std::error::Error + 'static> FutureSpawner<E> for WinitExecutor {
    fn spawn(&self, future: OwnedFuture<(), E>) {
        // Discard any errors.
        let future = async {
            if let Err(e) = future.await {
                tracing::error!("Async error: {}", e);
            }
        };

        let event_loop = self.event_loop.clone();
        let scheduler = move |task| {
            let event = RuffleEvent::TaskPoll(PlayerRunnable(task));
            if event_loop.send_event(event).is_err() {
                tracing::error!("Couldn't schedule task - event loop is closed");
            }
        };

        let (runnable, task) = async_task::Builder::new()
            .metadata(self.player_id)
            .spawn_local(|_| future, scheduler);

        // The future should run in the background.
        task.detach();
        // Immediately schedule the future to be polled for the first time.
        runnable.schedule();
    }
}
