use crate::custom_event::{OpenType, RuffleEvent};
use crate::gui::{GuiController, MENU_HEIGHT};
use crate::player::{LaunchOptions, PlayerController};
use crate::preferences::GlobalPreferences;
use crate::util::{
    get_screen_size, gilrs_button_to_gamepad_button, winit_input_to_ruffle_key_descriptor,
    winit_to_ruffle_text_control,
};
use anyhow::Error;
use gilrs::{Event, EventType, Gilrs};
use ruffle_core::FloatDuration;
use ruffle_core::PlayerEvent;
use ruffle_core::events::{ImeEvent, ImeNotification, PlayerNotification};
use ruffle_core::swf::HeaderExt;
use ruffle_frontend_utils::content::ContentDescriptor;
use ruffle_render::backend::ViewportDimensions;
use std::sync::Arc;
use std::time::Instant;
use winit::application::ApplicationHandler;
use winit::dpi::{LogicalSize, PhysicalPosition, PhysicalSize, Size};
use winit::event::{ElementState, Ime, KeyEvent, Modifiers, StartCause, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop, EventLoopProxy};
use winit::keyboard::{Key, NamedKey};
use winit::window::{Fullscreen, Icon, WindowAttributes, WindowId};

struct MainWindow {
    #[cfg(windows)]
    window_move: crate::window_move::WindowMoveLoop,
    #[cfg(windows)]
    aoqi_host: Option<crate::aoqi_host::HostUi>,
    preferences: GlobalPreferences,
    gui: GuiController,
    player: PlayerController,
    minimized: bool,
    /// Set from `WindowEvent::Occluded`, which winit only reports on macOS and X11
    /// (and on X11 only without a compositor). Always false elsewhere.
    occluded: bool,
    mouse_pos: PhysicalPosition<f64>,

    /// Whether `mouse_pos` changed since the last `MouseMove` sent to the player.
    /// Moves are coalesced, so that a movie with slow mouse handlers sees only
    /// the latest position instead of every move queued while it was busy.
    mouse_move_pending: bool,

    modifiers: Modifiers,
    min_window_size: LogicalSize<u32>,
    max_window_size: PhysicalSize<u32>,
    no_gui: bool,
    is_tools_child: bool,
    preferred_width: Option<f64>,
    preferred_height: Option<f64>,
    start_fullscreen: bool,
    loaded: LoadingState,
    time: Instant,
    next_frame_time: Option<Instant>,
    event_loop_proxy: EventLoopProxy<RuffleEvent>,
}

impl MainWindow {
    #[cfg(windows)]
    fn drop_with_close_timings(self) {
        // Match the declaration order. Release the GUI surface before the PlayerController.
        drop(self.window_move);
        drop(self.aoqi_host);
        drop(self.preferences);
        {
            let _phase = crate::aola_close_phases::phase("gui-drop");
            drop(self.gui);
        }
        {
            let _phase = crate::aola_close_phases::phase("player-controller-drop");
            drop(self.player);
        }
    }

    pub fn window_event(&mut self, event_loop: &ActiveEventLoop, event: WindowEvent) {
        if matches!(event, WindowEvent::RedrawRequested) {
            // Don't render when nobody can see the result: while minimized, the
            // surface also keeps the size it had before, as `GuiController::resize`
            // refuses to reconfigure it to a zero size.
            if !self.minimized && !self.occluded {
                self.gui.render(self.player.get());
            }

            // Important that we return here, or we'll get a feedback loop with egui
            // (winit says redraw, egui hears redraw and says redraw, we hear redraw and tell winit to redraw...)
            return;
        }

        if self.gui.handle_event(&event) {
            // Event consumed by GUI.
            return;
        }
        match event {
            WindowEvent::CloseRequested => {
                #[cfg(windows)]
                if self.aoqi_host.as_ref().is_some_and(|host| !host.close_requested()) {
                    return;
                }
                event_loop.exit();
            }
            WindowEvent::Resized(size) => {
                // How minimizing shows up on Windows; winit still has no
                // `WindowEvent::Minimize`. (`Window::is_minimized` does exist by
                // now, but it has to be polled, and it wouldn't tell us anything
                // new: macOS reports minimized windows as occluded instead, and
                // on Wayland it always returns `None`.)
                self.minimized = size.width == 0 && size.height == 0;

                if let Some(mut player) = self.player.get() {
                    player.set_viewport_dimensions(self.gui.viewport_dimensions(size));
                }
                // AS resize handlers calculate WebView frames from the new viewport.
                #[cfg(windows)]
                if let Some(host) = &mut self.aoqi_host { host.resized(); }
                self.gui.window().request_redraw();
                if matches!(self.loaded, LoadingState::WaitingForResize) {
                    self.loaded = LoadingState::Loaded;
                }
            }
            WindowEvent::ScaleFactorChanged { .. } if self.preferences.cli.aoqi_host || self.is_tools_child => {
                if let Some(mut player) = self.player.get() {
                    player.set_viewport_dimensions(self.gui.viewport_dimensions(self.gui.window().inner_size()));
                }
                #[cfg(windows)]
                if let Some(host) = &mut self.aoqi_host { host.resized(); }
                self.gui.window().request_redraw();
            }
            WindowEvent::Occluded(occluded) => {
                self.occluded = occluded;
                if !occluded {
                    // Nothing repaints us on its own if the movie is paused.
                    self.gui.window().request_redraw();
                }
            }
            WindowEvent::CursorMoved { position, .. } => {
                if self.gui.is_context_menu_visible() {
                    return;
                }

                // This is needed because some platforms (like macOS)
                // will fire this event at the same time as WindowEvent::MouseInput,
                // which may cause inaccurate behavior in the movie
                if position == self.mouse_pos {
                    return;
                }

                self.mouse_pos = position;

                // We can't handle mouse move here, doing so would put Ruffle in
                // a busy loop processing every queued CursorMoved event. Flash
                // Player does not queue mouse moves and instead skips them
                // if they happen when the movie is busy.
                self.mouse_move_pending = true;
            }
            WindowEvent::DroppedFile(file) => {
                if let Some(content_descriptor) = ContentDescriptor::new_local(&file, None) {
                    self.gui.create_movie(
                        &mut self.player,
                        LaunchOptions::from(&self.preferences),
                        content_descriptor,
                    );
                }
            }
            WindowEvent::Focused(true) => {
                self.handle_player_event(PlayerEvent::FocusGained);
            }
            WindowEvent::Focused(false) => {
                self.handle_player_event(PlayerEvent::FocusLost);
            }
            WindowEvent::MouseInput { button, state, .. } => {
                if self.gui.is_context_menu_visible() {
                    return;
                }

                use ruffle_core::events::MouseButton as RuffleMouseButton;
                use winit::event::MouseButton;
                let (x, y) = self.gui.window_to_movie_position(self.mouse_pos);
                let button = match button {
                    MouseButton::Left => RuffleMouseButton::Left,
                    MouseButton::Right => RuffleMouseButton::Right,
                    MouseButton::Middle => RuffleMouseButton::Middle,
                    _ => RuffleMouseButton::Unknown,
                };
                let event = match state {
                    // TODO We should get information about click index from the OS,
                    //   but winit does not support that yet.
                    ElementState::Pressed => PlayerEvent::MouseDown {
                        x,
                        y,
                        button,
                        index: None,
                    },
                    ElementState::Released => PlayerEvent::MouseUp { x, y, button },
                };
                let handled = self.handle_player_event(event);
                if !handled && state == ElementState::Pressed && button == RuffleMouseButton::Right
                {
                    // Show context menu.
                    if let Some(mut player) = self.player.get() {
                        let context_menu = player.prepare_context_menu();

                        // MouseUp event will be ignored when the context menu is shown,
                        // but it has to be dispatched when the menu closes.
                        let close_event = PlayerEvent::MouseUp {
                            x,
                            y,
                            button: RuffleMouseButton::Right,
                        };
                        self.gui.show_context_menu(context_menu, close_event);
                    }
                }
            }
            WindowEvent::MouseWheel { delta, .. } => {
                if self.gui.is_context_menu_visible() {
                    return;
                }

                use ruffle_core::events::MouseWheelDelta;
                use winit::event::MouseScrollDelta;
                let delta = match delta {
                    MouseScrollDelta::LineDelta(_, dy) => MouseWheelDelta::Lines(dy.into()),
                    MouseScrollDelta::PixelDelta(pos) if self.preferences.cli.aoqi_host => {
                        let (_, y) = self.gui.window_to_movie_position(PhysicalPosition::new(0.0, pos.y));
                        let (_, origin) = self.gui.window_to_movie_position(PhysicalPosition::new(0.0, 0.0));
                        MouseWheelDelta::Pixels(y - origin)
                    }
                    MouseScrollDelta::PixelDelta(pos) => MouseWheelDelta::Pixels(pos.y),
                };
                let event = PlayerEvent::MouseWheel { delta };
                self.handle_player_event(event);
            }
            WindowEvent::CursorEntered { .. } => {
                if let Some(mut player) = self.player.get() {
                    player.set_mouse_in_stage(true);
                }
            }
            WindowEvent::CursorLeft { .. } => {
                // The movie has to see the latest position while the mouse
                // is still in the stage.
                self.flush_mouse_move();
                if let Some(mut player) = self.player.get() {
                    player.set_mouse_in_stage(false);
                }
                self.handle_player_event(PlayerEvent::MouseLeave);
            }
            WindowEvent::ModifiersChanged(new_modifiers) => {
                self.modifiers = new_modifiers;
            }
            WindowEvent::KeyboardInput { event, .. } => {
                if self.gui.is_context_menu_visible() {
                    return;
                }

                // Handle escaping from fullscreen.
                if let KeyEvent {
                    state: ElementState::Pressed,
                    logical_key: Key::Named(NamedKey::Escape),
                    ..
                } = event
                {
                    let _ = self
                        .event_loop_proxy
                        .send_event(RuffleEvent::ExitFullScreen);
                }

                let key = winit_input_to_ruffle_key_descriptor(&event);
                match event.state {
                    ElementState::Pressed => {
                        self.handle_player_event(PlayerEvent::KeyDown { key });
                        if let Some(control_code) =
                            winit_to_ruffle_text_control(&event, self.modifiers)
                        {
                            self.handle_player_event(PlayerEvent::TextControl {
                                code: control_code,
                            });
                        } else if let Some(text) = event.text {
                            for codepoint in text.chars() {
                                self.handle_player_event(PlayerEvent::TextInput { codepoint });
                            }
                        }
                    }
                    ElementState::Released => {
                        self.handle_player_event(PlayerEvent::KeyUp { key });
                    }
                };
            }
            WindowEvent::Ime(ime) => match ime {
                Ime::Enabled => {}
                Ime::Preedit(text, cursor) => {
                    self.handle_player_event(PlayerEvent::Ime(ImeEvent::Preedit(text, cursor)));
                }
                Ime::Commit(text) => {
                    self.handle_player_event(PlayerEvent::Ime(ImeEvent::Commit(text)));
                }
                Ime::Disabled => {}
            },
            _ => (),
        }
    }

    fn on_metadata(&mut self, swf_header: HeaderExt) {
        #[cfg(windows)]
        if self.is_tools_child {
            // Only the top-level tools window determines its child rectangle.
            self.loaded = LoadingState::Loaded;
            self.gui.window().set_visible(true);
            if let Some(mut player) = self.player.get() {
                player.set_viewport_dimensions(self.gui.viewport_dimensions(self.gui.window().inner_size()));
            }
            return;
        }
        let height_offset = if self.gui.window().fullscreen().is_some() || self.no_gui {
            0.0
        } else {
            MENU_HEIGHT as f64
        };

        // To prevent issues like waiting on resize indefinitely (#11364) or desyncing the window state on Windows,
        // do not resize while window is maximized.
        let should_resize = !self.gui.window().is_maximized();

        let (viewport_size, state) = if should_resize {
            let movie_width = swf_header.stage_size().width().to_pixels();
            let movie_height = swf_header.stage_size().height().to_pixels();

            let window_size: Size = match (self.preferred_width, self.preferred_height) {
                (None, None) => LogicalSize::new(movie_width, movie_height + height_offset).into(),
                (Some(width), None) => {
                    let scale = width / movie_width;
                    let height = movie_height * scale;
                    PhysicalSize::new(
                        width.max(1.0),
                        height.max(1.0) + height_offset * self.gui.window().scale_factor(),
                    )
                    .into()
                }
                (None, Some(height)) => {
                    let scale = height / movie_height;
                    let width = movie_width * scale;
                    PhysicalSize::new(
                        width.max(1.0),
                        height.max(1.0) + height_offset * self.gui.window().scale_factor(),
                    )
                    .into()
                }
                (Some(width), Some(height)) => PhysicalSize::new(
                    width.max(1.0),
                    height.max(1.0) + height_offset * self.gui.window().scale_factor(),
                )
                .into(),
            };

            let window_size = Size::clamp(
                window_size,
                self.min_window_size.into(),
                self.max_window_size.into(),
                self.gui.window().scale_factor(),
            );

            let viewport_size = self.gui.window().inner_size();
            let mut window_resize_denied = false;

            if let Some(new_viewport_size) = self.gui.window().request_inner_size(window_size) {
                if new_viewport_size != viewport_size {
                    self.gui.resize(new_viewport_size);
                } else {
                    tracing::warn!("Unable to resize window");
                    window_resize_denied = true;
                }
            }

            let viewport_size = self.gui.window().inner_size();

            // On X11 (and possibly other platforms), the window size is not updated immediately.
            // On a successful resize request, wait for the window to be resized to the requested size
            // before we start running the SWF (which can observe the viewport size in "noScale" mode)
            let state = if !window_resize_denied && window_size != viewport_size.into() {
                LoadingState::WaitingForResize
            } else {
                LoadingState::Loaded
            };

            (viewport_size, state)
        } else {
            (self.gui.window().inner_size(), LoadingState::Loaded)
        };

        self.loaded = state;

        self.gui.window().set_fullscreen(if self.start_fullscreen {
            Some(Fullscreen::Borderless(None))
        } else {
            None
        });
        self.gui.window().set_visible(true);

        if let Some(mut player) = self.player.get() {
            let dimensions = if self.preferences.cli.aoqi_host {
                self.gui.viewport_dimensions(viewport_size)
            } else {
                let viewport_scale_factor = self.gui.window().scale_factor();
                ViewportDimensions {
                    width: viewport_size.width,
                    height: viewport_size.height - (height_offset * viewport_scale_factor) as u32,
                    scale_factor: viewport_scale_factor,
                }
            };
            player.set_viewport_dimensions(dimensions);
        }
    }

    fn about_to_wait(&mut self, gilrs: Option<&mut Gilrs>) {
        // When we don't have any more pending events, flush any pending mouse
        // moves, so that they can get coalesced properly.
        self.flush_mouse_move();

        if let Some(Event { event, .. }) = gilrs.and_then(|gilrs| gilrs.next_event()) {
            match event {
                EventType::ButtonPressed(button, _) => {
                    if let Some(button) = gilrs_button_to_gamepad_button(button) {
                        self.handle_player_event(PlayerEvent::GamepadButtonDown { button });
                    }
                }
                EventType::ButtonReleased(button, _) => {
                    if let Some(button) = gilrs_button_to_gamepad_button(button) {
                        self.handle_player_event(PlayerEvent::GamepadButtonUp { button });
                    }
                }
                _ => {}
            }
        }

        // The ordinary event loop and native Windows move/size wake-ups share
        // this clock, so neither path replays time already consumed by the other.
        if matches!(self.loaded, LoadingState::Loaded) {
            let new_time = Instant::now();
            let dt = FloatDuration::from_std(new_time.duration_since(self.time));
            if dt.as_millis() > 0.0 {
                self.time = new_time;
                self.next_frame_time = self.player.get().map(|mut player| {
                    #[cfg(windows)]
                    let dt = FloatDuration::from_millis(dt.as_millis() * crate::aoqi_host::time_scale());
                    player.tick(dt);
                    new_time + player.time_til_next_frame()
                });
            }
        }

        self.render_movie_if_needed();
    }

    fn handle_player_event(&mut self, event: PlayerEvent) -> bool {
        // Apply any pending mouse moves, so that the movie sees the latest
        // position before e.g. a click.
        self.flush_mouse_move();
        let handled = self.player.handle_event(event);

        // Handling an event may have made the movie need rendering (on demand).
        self.render_movie_if_needed();
        handled
    }

    fn flush_mouse_move(&mut self) {
        if !std::mem::take(&mut self.mouse_move_pending) {
            return;
        }

        let (x, y) = self.gui.window_to_movie_position(self.mouse_pos);
        self.player.handle_event(PlayerEvent::MouseMove { x, y });
        self.render_movie_if_needed();
    }

    fn render_movie_if_needed(&self) {
        let mut player = self.player.get();
        let player_needs_render = player.as_ref().is_some_and(|p| p.needs_render());
        if player_needs_render
            && !self.minimized
            && !self.occluded
            && let Some(player) = player.as_deref_mut()
        {
            player.render();
        }
        if player_needs_render || self.gui.needs_render() {
            self.gui.window().request_redraw();
        }
    }
}

pub struct App {
    #[cfg(windows)]
    aola_browser: Option<crate::aola_host::LoginBrowser>,
    #[cfg(windows)]
    aola_performance: Option<crate::aola_diagnostics::PerformanceLog>,
    main_window: Option<MainWindow>,
    #[cfg(windows)]
    mypro_host: Option<crate::mypro_host::ToolsHost>,
    runtime: Option<tokio::runtime::Runtime>,
    gilrs: Option<Gilrs>,
    event_loop_proxy: EventLoopProxy<RuffleEvent>,
    preferences: GlobalPreferences,
    font_database: fontdb::Database,
}

/// Enters the tokio runtime context.
/// This cannot be a method, as the borrow-checker would complain.
macro_rules! enter_runtime {
    ($this:expr) => {
        let _guard = $this.runtime.as_ref().map(|runtime| runtime.enter());
    };
}

impl App {
    pub fn new(preferences: GlobalPreferences) -> Result<(Self, EventLoop<RuffleEvent>), Error> {
        let event_loop = EventLoop::with_user_event().build()?;

        let mut font_database = fontdb::Database::default();
        font_database.load_system_fonts();

        let gilrs = Gilrs::new()
            .inspect_err(|err| {
                tracing::warn!("Gamepad support could not be initialized: {err}");
            })
            .ok();
        let event_loop_proxy = event_loop.create_proxy();
        let runtime = tokio::runtime::Runtime::new()?;
        #[cfg(windows)]
        crate::aola_host::set_proxy(event_loop_proxy.clone());
        #[cfg(windows)]
        let aola_performance = if preferences.cli.aola_host || crate::aola_close_phases::configured() {
            Some(crate::aola_diagnostics::PerformanceLog::new(
                preferences.cli.aola_data_dir.as_ref().expect("Aola data directory initialized"))?)
        } else { None };

        Ok((
            Self {
                #[cfg(windows)]
                aola_browser: None,
                #[cfg(windows)]
                aola_performance,
                main_window: None,
                #[cfg(windows)]
                mypro_host: None,
                runtime: Some(runtime),
                gilrs,
                event_loop_proxy,
                font_database,
                preferences,
            },
            event_loop,
        ))
    }
}

impl ApplicationHandler<RuffleEvent> for App {
    fn new_events(&mut self, event_loop: &ActiveEventLoop, cause: StartCause) {
        enter_runtime!(self);

        if cause == StartCause::Init {
            let movie_url = self.preferences.cli.movie_url.clone();
            let icon_bytes = include_bytes!("../assets/favicon-32.rgba");
            let icon =
                Icon::from_rgba(icon_bytes.to_vec(), 32, 32).expect("App icon should be correct");
            let icon = if self.preferences.cli.aoqi_host {
                aoqi_assets::get("assets/app-icon.png")
                    .and_then(|asset| image::load_from_memory(asset.bytes).ok())
                    .and_then(|image| {
                        let image=image.into_rgba8();
                        let (width,height)=image.dimensions();
                        Icon::from_rgba(image.into_raw(),width,height).ok()
                    }).unwrap_or(icon)
            } else { icon };

            let no_gui = self.preferences.cli.no_gui;
            let min_window_size = if no_gui {
                (16, 16)
            } else {
                (350, MENU_HEIGHT + 16)
            }
            .into();
            let start_fullscreen = self.preferences.cli.fullscreen;

            #[cfg_attr(not(target_os = "linux"), allow(unused_mut))]
            let mut window_attributes = WindowAttributes::default()
                .with_visible(false)
                .with_title("Ruffle")
                .with_window_icon(Some(icon))
                .with_min_inner_size(min_window_size);

            let (preferred_width, preferred_height) = if self.preferences.cli.aola_host {
                let size = PhysicalSize::new(1200.0, 700.0);
                window_attributes = window_attributes
                    .with_title("奥拉星 · 原生迁移测试版")
                    .with_inner_size(size);
                (Some(size.width), Some(size.height))
            } else {
                (self.preferences.cli.width, self.preferences.cli.height)
            };

            #[cfg(target_os = "linux")]
            {
                use winit::platform::startup_notify::{
                    self, EventLoopExtStartupNotify, WindowAttributesExtStartupNotify,
                };
                use winit::platform::wayland::WindowAttributesExtWayland;
                window_attributes = window_attributes.with_name("rs.ruffle.Ruffle", "main");
                if let Some(token) = event_loop.read_token_from_env() {
                    startup_notify::reset_activation_token_env();
                    window_attributes = window_attributes.with_activation_token(token);
                }
            }

            let event_loop_proxy = self.event_loop_proxy.clone();
            let preferences = self.preferences.clone();
            #[cfg(windows)]
            if crate::mypro_host::enabled() && !preferences.cli.aoqi_host {
                let root = preferences.cli.aola_data_dir.clone().unwrap_or_else(|| preferences.cli.config.clone());
                match crate::mypro_host::ToolsHost::new(event_loop, event_loop_proxy.clone(), root, window_attributes.clone()) {
                    Ok(host) => {
                        window_attributes = host.child_attributes(window_attributes).expect("Native game child handle");
                        self.mypro_host = Some(host);
                    }
                    Err(err) => tracing::error!("MyPro tools unavailable, opening normal player: {err}"),
                }
            }
            let window = event_loop
                .create_window(window_attributes)
                .expect("Window should be created");
            let max_window_size = get_screen_size(&window);
            window.set_max_inner_size(Some(max_window_size));
            let window = Arc::new(window);
            #[cfg(windows)]
            let window_move = crate::window_move::WindowMoveLoop::new(
                self.mypro_host.as_ref().map_or(&window, |h| &h.parent).clone(), event_loop_proxy.clone(),
            ).expect("Window move timer initialization failed");
            #[cfg(windows)]
            let aoqi_host = if preferences.cli.aoqi_host {
                Some(crate::aoqi_host::HostUi::new(&preferences.cli, window.clone(), event_loop_proxy.clone())
                    .expect("Native host initialization failed"))
            } else { None };
            let font_database = self.font_database.clone();

            let mut gui = GuiController::new(
                window.clone(),
                event_loop_proxy.clone(),
                preferences.clone(),
                &font_database,
                movie_url.clone(),
                no_gui,
            )
            .expect("GUI controller should be created");
            #[cfg(windows)]
            { gui.aola_performance = self.aola_performance.take(); }

            let mut player = PlayerController::new(
                event_loop_proxy.clone(),
                window.clone(),
                gui.descriptors().clone(),
                font_database,
                preferences.clone(),
                gui.file_picker(),
            );

            if let Some(movie_url) = &movie_url {
                gui.create_movie(
                    &mut player,
                    LaunchOptions::from(&preferences),
                    ContentDescriptor {
                        url: movie_url.clone(),
                        root_content_path: None,
                    },
                );
            } else if !self.preferences.cli.aola_host {
                gui.show_open_dialog();
            }

            let mut loaded = LoadingState::Loading;

            if movie_url.is_none() {
                // No SWF provided on command line; show window with dummy movie immediately.
                window.set_visible(true);
                loaded = LoadingState::Loaded;
            }

            self.main_window = Some(MainWindow {
                #[cfg(windows)]
                window_move,
                #[cfg(windows)]
                aoqi_host,
                preferences,
                gui,
                player,
                min_window_size,
                max_window_size,
                no_gui,
                #[cfg(windows)]
                is_tools_child: self.mypro_host.is_some(),
                #[cfg(not(windows))]
                is_tools_child: false,
                preferred_width,
                preferred_height,
                start_fullscreen,
                loaded,
                minimized: false,
                occluded: false,
                mouse_pos: PhysicalPosition::new(0.0, 0.0),
                mouse_move_pending: false,
                modifiers: Modifiers::default(),
                time: Instant::now(),
                next_frame_time: None,
                event_loop_proxy,
            });
            #[cfg(windows)]
            if let Some(host) = &mut self.mypro_host {
                host.attach_game(window.clone(), self.event_loop_proxy.clone()).expect("Native game focus handler");
                host.layout(&window);
                host.parent.set_visible(true);
            }
            #[cfg(windows)]
            if crate::aola_host::enabled() && movie_url.is_none() {
                match crate::aola_host::LoginBrowser::new(window,self.event_loop_proxy.clone()) {
                    Ok(browser)=>self.aola_browser=Some(browser),
                    Err(_)=>{crate::aola_host::record("host-error","webview2-create");event_loop.exit();}
                }
            }
        }
    }

    fn resumed(&mut self, _event_loop: &ActiveEventLoop) {}

    fn user_event(&mut self, event_loop: &ActiveEventLoop, event: RuffleEvent) {
        #[cfg(windows)]
        if let RuffleEvent::WindowMoveTick(window_id) = event {
            let main_pending = self.main_window.as_ref().is_some_and(|main|
                main.window_move.tick_pending()
                && (main.gui.window().id() == window_id || self.mypro_host.as_ref().is_some_and(|h| h.parent.id() == window_id))
            );
            let panel_pending = self.mypro_host.as_ref().is_some_and(|host|
                host.panel.id() == window_id && host.panel_move.tick_pending()
            );
            if main_pending || panel_pending {
                // AboutToWait is not dispatched by Windows' nested move/size
                // loop. Use the same clock, player update and redraw path here.
                // Keep the pending flag set until done to prevent reentrant
                // message pumping from queueing a second player update.
                self.about_to_wait(event_loop);
                if main_pending && let Some(main) = &self.main_window {
                    main.window_move.finish_tick();
                }
                if panel_pending && let Some(host) = &self.mypro_host { host.panel_move.finish_tick(); }
            }
            return;
        }
        enter_runtime!(self);

        #[cfg(windows)]
        let event=match event {
            RuffleEvent::MyProPageReady(generation) => {
                if let Some(host) = &mut self.mypro_host { host.page_ready(generation); }
                return;
            }
            RuffleEvent::MyProOpenPanel => {
                if let Some(host) = &mut self.mypro_host { host.open(); }
                return;
            }
            RuffleEvent::MyProTogglePanel => {
                if let (Some(host), Some(main)) = (&mut self.mypro_host, &self.main_window) {
                    host.toggle(main.gui.window());
                }
                return;
            }
            RuffleEvent::MyProFullscreen(full) => {
                if let Some(host) = &self.mypro_host {
                    host.parent.set_fullscreen(full.then_some(Fullscreen::Borderless(None)));
                }
                return;
            }
            RuffleEvent::MyProRequest(generation, body) => {
                if let (Some(host), Some(main)) = (&mut self.mypro_host, &self.main_window) {
                    host.request(generation, &body, &main.player, main.gui.window());
                }
                return;
            }
            RuffleEvent::MyProGameFocus(focused) => {
                if let (Some(host), Some(main)) = (&mut self.mypro_host, &mut self.main_window) {
                    if !focused { main.modifiers = Modifiers::default(); }
                    host.focus_changed(&main.player, focused, main.gui.window_to_movie_position(main.mouse_pos));
                }
                return;
            }
            RuffleEvent::AolaPageState(payload)=>{
                if let Some(browser)=&self.aola_browser
                    && browser.accept_snapshot(&payload)
                    && let Some(main)=&mut self.main_window {
                    main.gui.create_movie(&mut main.player,LaunchOptions::from(&self.preferences),ContentDescriptor {
                        url:url::Url::parse(crate::aola_host::START_URL).expect("Official start URL"),root_content_path:None,
                    });
                    main.gui.window().focus_window();
                    crate::aola_host::record("host","player-started-with-page-context");
                }
                return;
            }
            RuffleEvent::AolaNavigate(url)=>{
                if let Some(main)=&mut self.main_window {main.gui.close_movie(&mut main.player);}
                if let Some(browser)=&self.aola_browser {browser.navigate(&url);}
                return;
            }
            RuffleEvent::AolaCookie(key,value,expires)=>{
                if let Some(browser)=&self.aola_browser {browser.write_cookie(&key,&value,&expires);}
                return;
            }
            event=>event,
        };
        match (&mut self.main_window, event) {
            (Some(main_window), RuffleEvent::AoqiRestart) => {
                if let Some(url) = main_window.preferences.cli.movie_url.clone() {
                    main_window.gui.create_movie(&mut main_window.player,
                        LaunchOptions::from(&main_window.preferences),
                        ContentDescriptor { url, root_content_path: None });
                }
            }
            (Some(main_window), RuffleEvent::TaskPoll(task)) => main_window.player.poll(task),

            (Some(main_window), RuffleEvent::OnMetadata(swf_header)) => {
                main_window.on_metadata(swf_header)
            }

            (Some(main_window), RuffleEvent::ContextMenuItemClicked(index)) => {
                if let Some(mut player) = main_window.player.get() {
                    player.run_context_menu_callback(index);
                }
            }

            (Some(main_window), RuffleEvent::BrowseAndOpen(options, open_type)) => {
                let event_loop = main_window.event_loop_proxy.clone();
                let picker = main_window.gui.file_picker();
                tokio::spawn(async move {
                    let picked = match open_type {
                        OpenType::File => {
                            picker.pick_ruffle_file(None).await.map(|file| (file, None))
                        }
                        OpenType::Directory => picker
                            .pick_ruffle_directory_and_content(None)
                            .await
                            .map(|(dir, file)| (file, Some(dir))),
                    };

                    if let Some(desc) =
                        picked.and_then(|(file, dir)| ContentDescriptor::new_local(&file, dir))
                    {
                        let _ = event_loop.send_event(RuffleEvent::Open(desc, options));
                    }
                });
            }

            (Some(main_window), RuffleEvent::Open(desc, options)) => {
                main_window
                    .gui
                    .create_movie(&mut main_window.player, *options, desc);
            }

            (Some(main_window), RuffleEvent::OpenDialog(descriptor)) => {
                main_window.gui.open_dialog(descriptor);
            }

            (Some(main_window), RuffleEvent::CloseFile) => {
                main_window.gui.window().set_title("Ruffle"); // Reset title since file has been closed.
                main_window.gui.close_movie(&mut main_window.player);
            }

            (Some(main_window), RuffleEvent::ExportBundle) => {
                main_window.gui.export_bundle();
            }

            (Some(main_window), RuffleEvent::EnterFullScreen) => {
                if let Some(mut player) = main_window.player.get()
                    && player.is_playing()
                {
                    player.set_fullscreen(true);
                }
            }

            (Some(main_window), RuffleEvent::ExitFullScreen) => {
                if let Some(mut player) = main_window.player.get()
                    && player.is_playing()
                {
                    player.set_fullscreen(false);
                }
            }

            (Some(main_window), RuffleEvent::PlayerNotification(notification)) => {
                match notification {
                    PlayerNotification::ImeNotification(ImeNotification::ImeReady {
                        purpose,
                        cursor_area,
                    }) => {
                        let ime_enabled = main_window.preferences.ime_enabled()
                            .unwrap_or(main_window.preferences.cli.aoqi_host || main_window.is_tools_child);
                        main_window.gui.set_ime_allowed(ime_enabled);
                        main_window.gui.set_ime_purpose(purpose);
                        main_window.gui.set_ime_cursor_area(cursor_area);
                    }
                    PlayerNotification::ImeNotification(ImeNotification::ImePurposeUpdated(
                        purpose,
                    )) => {
                        main_window.gui.set_ime_purpose(purpose);
                    }
                    PlayerNotification::ImeNotification(ImeNotification::ImeCursorAreaUpdated(
                        cursor_area,
                    )) => {
                        main_window.gui.set_ime_cursor_area(cursor_area);
                    }
                    PlayerNotification::ImeNotification(ImeNotification::ImeNotReady) => {
                        main_window.gui.set_ime_allowed(false);
                    }
                }
            }

            (_, RuffleEvent::ExitRequested) => {
                event_loop.exit();
            }

            _ => {}
        }
    }

    fn window_event(
        &mut self,
        event_loop: &ActiveEventLoop,
        window_id: WindowId,
        event: WindowEvent,
    ) {
        enter_runtime!(self);
        #[cfg(windows)]
        if let (Some(tools), Some(main)) = (&mut self.mypro_host, &mut self.main_window) {
            if let WindowEvent::KeyboardInput { event: key, .. } = &event
                && key.logical_key == Key::Named(NamedKey::F8)
                && (window_id == tools.parent.id() || window_id == tools.panel.id() || window_id == main.gui.window().id())
            {
                if key.state == ElementState::Pressed && !key.repeat { tools.toggle(main.gui.window()); }
                return;
            }
            if window_id == tools.panel.id() {
                match event {
                    WindowEvent::CloseRequested => tools.hide(main.gui.window()),
                    WindowEvent::Resized(_) | WindowEvent::ScaleFactorChanged { .. } => tools.layout_panel(),
                    _ => {}
                }
                return;
            }
            if window_id == tools.parent.id() {
                match event {
                    WindowEvent::CloseRequested => event_loop.exit(),
                    WindowEvent::Resized(_) | WindowEvent::ScaleFactorChanged { .. } => {
                        let size = tools.parent.inner_size();
                        main.minimized = size.width == 0 || size.height == 0 || tools.parent.is_minimized() == Some(true);
                        tools.layout(main.gui.window());
                        if !main.minimized { main.gui.window().request_redraw(); }
                    }
                    _ => {}
                }
                return;
            }
            if window_id != main.gui.window().id() { return; }
            if matches!(event, WindowEvent::Focused(_)) { return; } // Native child focus is delivered above.
            tools.track_input(&event);
        }
        #[cfg(windows)]
        if let Some(browser)=&self.aola_browser
            && self.main_window.as_ref().is_some_and(|main| main.gui.window().id()==window_id) {
            browser.window_event(&event);
        }

        if let Some(main_window) = &mut self.main_window {
            main_window.window_event(event_loop, event);
        }
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        enter_runtime!(self);

        if let Some(main_window) = &mut self.main_window {
            #[cfg(windows)]
            if let Some(host) = &mut self.mypro_host { host.sync_session(&main_window.player); }
            main_window.about_to_wait(self.gilrs.as_mut());
            #[cfg(windows)]
            if let Some(host) = &mut main_window.aoqi_host {
                host.pump(event_loop, main_window.gui.presentation_stats());
                let wake = Instant::now() + std::time::Duration::from_millis(16);
                main_window.next_frame_time = Some(main_window.next_frame_time.map_or(wake, |next| next.min(wake)));
            }
            #[cfg(windows)]
            main_window.gui.sample_performance(&main_window.player, crate::aola_diagnostics::SampleKind::Interval);

            // The event loop is finished; let's find out how long we need to wait for.
            // We don't need to worry about earlier update requests, as it's the
            // only place where we're setting control flow, and events cancel wait.
            // Note: the control flow might be set to `ControlFlow::WaitUntil` with a
            // timestamp in the past! Take that into consideration when changing this code.
            if let Some(next_frame_time) = main_window.next_frame_time {
                event_loop.set_control_flow(ControlFlow::WaitUntil(next_frame_time));
            }
        }
    }

    fn exiting(&mut self, _event_loop: &ActiveEventLoop) {
        #[cfg(windows)]
        {
            crate::aola_close_phases::start();
            if let Some(browser) = self.aola_browser.take() {
                let _phase = crate::aola_close_phases::phase("webview-drop");
                drop(browser);
            }
            if let Some(host) = &mut self.mypro_host { host.stop(); }
        }
        // `MainWindow` needs a tokio context to properly drop.
        {
            enter_runtime!(self);
            #[cfg(windows)]
            if let Some(window) = &mut self.main_window {
                if let Some(host) = &mut window.aoqi_host { host.finish(window.gui.presentation_stats()); }
                if window.gui.aola_performance.is_some() {
                    let _phase = crate::aola_close_phases::phase("final-performance-sample");
                    window.gui.sample_performance(&window.player, crate::aola_diagnostics::SampleKind::ProcessEnd);
                }
            }
            if let Some(window) = self.main_window.take() {
                #[cfg(windows)]
                let _phase = crate::aola_close_phases::phase("main-window-drop");
                #[cfg(windows)]
                if crate::aola_close_phases::active() { window.drop_with_close_timings(); } else { drop(window); }
                #[cfg(not(windows))]
                drop(window);
            }
            #[cfg(windows)]
            if let Some(host) = self.mypro_host.take() {
                let _phase = crate::aola_close_phases::phase("mypro-tools-drop");
                drop(host);
            }
        }

        // Manually stop the tokio runtime: this makes sure that any pending Player-bound futures
        // are properly cancelled and put back on the winit event loop before it closes, preventing
        // them from being dropped on the wrong thread and causing a panic.
        if let Some(runtime) = self.runtime.take() {
            #[cfg(windows)]
            let _phase = crate::aola_close_phases::phase("tokio-shutdown");
            runtime.shutdown_timeout(std::time::Duration::from_secs(1));
        }
        #[cfg(windows)]
        crate::aola_close_phases::record("event-loop-tail", "begin", None);
    }
}

enum LoadingState {
    Loading,
    WaitingForResize,
    Loaded,
}
