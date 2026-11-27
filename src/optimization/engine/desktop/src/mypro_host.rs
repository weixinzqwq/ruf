//! The player and its modeless tools are separate top-level windows. Only the game owns a surface.
use crate::custom_event::RuffleEvent;
use crate::mypro_features::Features;
use crate::mypro_services::ToolsService;
use crate::player::PlayerController;
use ruffle_core::events::KeyDescriptor;
use ruffle_core::PlayerEvent;
use serde_json::{json, Value};
use std::borrow::Cow;
use std::cell::Cell;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::Arc;
use std::time::{Duration, Instant};
use winit::dpi::{LogicalSize, PhysicalPosition, PhysicalSize};
use winit::event::{ElementState, MouseButton, WindowEvent};
use winit::event_loop::{ActiveEventLoop, EventLoopProxy};
use winit::platform::windows::WindowAttributesExtWindows;
use winit::raw_window_handle::{HasWindowHandle, RawWindowHandle};
use winit::window::{Window, WindowAttributes};
use wry::{NewWindowResponse, PageLoadEvent, Rect, WebContext, WebView, WebViewBuilder};
use windows_sys::core::w;
use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
use windows_sys::Win32::Graphics::Gdi::{CreateFontIndirectW, DeleteObject, HFONT};
use windows_sys::Win32::UI::HiDpi::SystemParametersInfoForDpi;
use windows_sys::Win32::UI::Input::KeyboardAndMouse::SetFocus;
use windows_sys::Win32::UI::Shell::{DefSubclassProc, RemoveWindowSubclass, SetWindowSubclass};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    BN_CLICKED, CreateWindowExW, DestroyWindow, GetWindowLongPtrW, MoveWindow, SendMessageW, SetPropW,
    GWLP_HINSTANCE, NONCLIENTMETRICSW, SPI_GETNONCLIENTMETRICS, WM_COMMAND, WM_KEYDOWN, WM_KILLFOCUS,
    WM_SETFOCUS, WM_SETFONT, WS_CHILD, WS_CLIPCHILDREN, WS_TABSTOP, WS_VISIBLE,
};

const PANEL_HTML: &str = include_str!("mypro_panel/index.html");
const BATTLE_ICONS: &[(&str, &[u8])] = include!(concat!(env!("OUT_DIR"), "/battle_icons.rs"));
const PANEL_URL: &str = "mypro://localhost/index.html";
// The locked Wry Windows backend maps the custom scheme to this local origin.
const PANEL_ORIGIN_URL: &str = "http://mypro.localhost/index.html";
const TOOLBAR_HEIGHT: f64 = 36.0;
const MAX_REQUEST_BYTES: usize = 512 * 1024;
const MAX_QUEUED_REQUESTS: usize = 16;

fn panel_response(method: &wry::http::Method, path: &str) -> wry::http::Response<Cow<'static, [u8]>> {
    let resource = if method == wry::http::Method::GET {
        match path {
            "/index.html" => Some(("text/html; charset=utf-8", PANEL_HTML.as_bytes())),
            "/battle-assets/names.json" => Some(("application/json", &include_bytes!("../assets/battle-assistant/names.json")[..])),
            _ => BATTLE_ICONS.binary_search_by(|(url, _)| url.cmp(&path)).ok()
                .map(|i| ("image/png", BATTLE_ICONS[i].1)),
        }
    } else { None };
    let (mime, bytes) = resource.unwrap_or(("text/plain", &[]));
    wry::http::Response::builder()
        .status(if resource.is_some() { 200 } else { 404 })
        .header("Content-Type", mime)
        .header("Content-Security-Policy", "default-src 'none'; script-src 'unsafe-inline'; style-src 'unsafe-inline'; img-src 'self' data:; connect-src 'self'; base-uri 'none'; form-action 'none'; frame-src 'none'")
        .body(Cow::Borrowed(bytes)).unwrap()
}

pub fn enabled() -> bool {
    std::env::var_os("AOLA_MYPRO_TOOLS").as_deref() == Some(std::ffi::OsStr::new("1"))
}

pub fn is_embedded_game(window: &Window) -> bool {
    use windows_sys::Win32::UI::WindowsAndMessaging::{GetWindowLongW, GWL_STYLE, WS_CHILD};
    enabled() && unsafe { GetWindowLongW(hwnd(window), GWL_STYLE) as u32 } & WS_CHILD != 0
}

fn toolbar_height(scale: f64) -> u32 {
    (TOOLBAR_HEIGHT * scale).round() as u32
}

struct Request<'a> {
    protocol_version: u64,
    bridge_generation: u64,
    request_id: &'a str,
    session_id: Option<&'a str>,
    command: &'a str,
    payload: &'a Value,
}

impl<'a> Request<'a> {
    fn parse(input: &'a Value) -> Option<Self> {
        let obj = input.as_object()?;
        if obj.len() != 6 { return None; }
        let session = obj.get("sessionId")?;
        Some(Self {
            protocol_version: obj.get("protocolVersion")?.as_u64()?,
            bridge_generation: obj.get("bridgeGeneration")?.as_u64()?,
            request_id: obj.get("requestId")?.as_str()?,
            session_id: if session.is_null() { None } else { Some(session.as_str()?) },
            command: obj.get("command")?.as_str()?,
            payload: obj.get("payload")?,
        })
    }
}

pub struct ToolsHost {
    input: Option<GameInput>,
    // WebView and WebContext must die before their native parent window.
    web: Option<WebView>,
    context: WebContext,
    toolbar: Toolbar,
    pub panel_move: crate::window_move::WindowMoveLoop,
    pub panel: Arc<Window>,
    pub parent: Arc<Window>,
    proxy: EventLoopProxy<RuffleEvent>,
    generation: Rc<Cell<u64>>,
    queued: Rc<Cell<usize>>,
    ready: u64,
    last_request: u64,
    visible: bool,
    shown: bool,
    session: Option<Option<i64>>,
    event_sequence: u64,
    background_revision: u32,
    visual_morph_revision: u64,
    next_native_auto_sample: Instant,
    services: ToolsService,
    lineups: crate::mypro_features::lineups::Lineups,
    features: Features,
}

// winit combines WM_ACTIVATE and WM_SETFOCUS for top-level focus notifications.
// WS_CHILD receives WM_SETFOCUS directly, including transitions to the tool WebView.
struct GameInput {
    window: Arc<Window>,
    proxy: Box<EventLoopProxy<RuffleEvent>>,
    keys: Vec<KeyDescriptor>,
    buttons: Vec<MouseButton>,
}

fn hwnd(window: &Window) -> HWND {
    let RawWindowHandle::Win32(handle) = window.window_handle().expect("live game child").as_raw() else { unreachable!() };
    handle.hwnd.get() as HWND
}

struct Toolbar {
    bar: HWND,
    button: HWND,
    font: HFONT,
    dpi: u32,
    proxy: Box<EventLoopProxy<RuffleEvent>>,
}

unsafe extern "system" fn toolbar_event(window: HWND, msg: u32, w: WPARAM, l: LPARAM, _: usize, data: usize) -> LRESULT {
    let event = if msg == WM_COMMAND && (w & 0xffff) == 1 && (w >> 16) == BN_CLICKED as usize {
        Some(RuffleEvent::MyProOpenPanel)
    } else if msg == WM_KEYDOWN && w == windows_sys::Win32::UI::Input::KeyboardAndMouse::VK_F8 as usize {
        if l & (1 << 30) != 0 { return 0; }
        Some(RuffleEvent::MyProTogglePanel)
    } else {
        None
    };
    if let Some(event) = event {
        let proxy = unsafe { &*(data as *const EventLoopProxy<RuffleEvent>) };
        let _ = proxy.send_event(event);
        return 0;
    }
    unsafe { DefSubclassProc(window, msg, w, l) }
}

impl Toolbar {
    fn new(parent: &Window, proxy: EventLoopProxy<RuffleEvent>) -> anyhow::Result<Self> {
        use std::ptr::null_mut;
        let owner = hwnd(parent);
        let instance = unsafe { GetWindowLongPtrW(owner, GWLP_HINSTANCE) } as _;
        let bar = unsafe { CreateWindowExW(0, w!("STATIC"), w!(""), WS_CHILD | WS_VISIBLE | WS_CLIPCHILDREN,
            0, 0, 1, 1, owner, null_mut(), instance, null_mut()) };
        anyhow::ensure!(!bar.is_null(), "Unable to create player toolbar");
        let button = unsafe { CreateWindowExW(0, w!("BUTTON"), w!("功能面板"), WS_CHILD | WS_VISIBLE | WS_TABSTOP,
            0, 0, 1, 1, bar, 1usize as _, instance, null_mut()) };
        let toolbar = Self { bar, button, font: null_mut(), dpi: 0, proxy: Box::new(proxy) };
        anyhow::ensure!(!button.is_null(), "Unable to create tools button");
        let id = &*toolbar.proxy as *const _ as usize;
        for window in [bar, button] {
            anyhow::ensure!(unsafe { SetWindowSubclass(window, Some(toolbar_event), id, id) } != 0,
                "Unable to handle tools button");
        }
        Ok(toolbar)
    }

    fn layout(&mut self, width: u32, height: u32, scale: f64) {
        let dpi = (96.0 * scale).round() as u32;
        let px = |value: f64| (value * scale).round() as i32;
        unsafe {
            if self.dpi != dpi {
                self.dpi = dpi;
                let mut metrics = NONCLIENTMETRICSW::default();
                metrics.cbSize = std::mem::size_of_val(&metrics) as u32;
                if SystemParametersInfoForDpi(SPI_GETNONCLIENTMETRICS, metrics.cbSize, (&mut metrics as *mut NONCLIENTMETRICSW).cast(), 0, dpi) != 0 {
                    let font = CreateFontIndirectW(&metrics.lfMessageFont);
                    if !font.is_null() {
                        SendMessageW(self.button, WM_SETFONT, font as usize, 1);
                        if !self.font.is_null() { DeleteObject(self.font); }
                        self.font = font;
                    }
                }
            }
            MoveWindow(self.bar, 0, 0, width as i32, height as i32, 1);
            MoveWindow(self.button, px(6.0), px(4.0), px(100.0), px(28.0), 1);
        }
    }
}

impl Drop for Toolbar {
    fn drop(&mut self) {
        let id = &*self.proxy as *const _ as usize;
        unsafe {
            for window in [self.button, self.bar] {
                if !window.is_null() { RemoveWindowSubclass(window, Some(toolbar_event), id); }
            }
            DestroyWindow(self.bar);
            if !self.font.is_null() { DeleteObject(self.font); }
        }
    }
}

unsafe extern "system" fn child_focus(hwnd: HWND, msg: u32, w: WPARAM, l: LPARAM, _: usize, data: usize) -> LRESULT {
    if msg == WM_SETFOCUS || msg == WM_KILLFOCUS {
        // GameInput owns the boxed proxy until the subclass is removed.
        let proxy = unsafe { &*(data as *const EventLoopProxy<RuffleEvent>) };
        let _ = proxy.send_event(RuffleEvent::MyProGameFocus(msg == WM_SETFOCUS));
    }
    unsafe { DefSubclassProc(hwnd, msg, w, l) }
}

impl Drop for GameInput {
    fn drop(&mut self) {
        let id = &*self.proxy as *const _ as usize;
        unsafe { RemoveWindowSubclass(hwnd(&self.window), Some(child_focus), id); }
    }
}

impl ToolsHost {
    pub fn new(event_loop: &ActiveEventLoop, proxy: EventLoopProxy<RuffleEvent>, root: PathBuf,
               attributes: WindowAttributes) -> anyhow::Result<Self> {
        let parent = Arc::new(event_loop.create_window(attributes.clone()
            .with_clip_children(true)
            .with_title("奥拉星 · MyPro 播放器")
            .with_min_inner_size(LogicalSize::new(480.0, 280.0)))?);
        let _ = parent.request_inner_size(PhysicalSize::new(1200, 700 + toolbar_height(parent.scale_factor())));
        let panel = Arc::new(event_loop.create_window(attributes.with_visible(false)
            .with_clip_children(true).with_title("功能面板")
            .with_min_inner_size(LogicalSize::new(440.0, 360.0))
            .with_inner_size(LogicalSize::new(680.0, 580.0)))?);
        for (window, role) in [(&parent, 1usize), (&panel, 2usize)] {
            anyhow::ensure!(unsafe { SetPropW(hwnd(window), w!("AolaRuffleWindowRole"), role as _) } != 0,
                "Unable to mark player window role");
        }
        let toolbar = Toolbar::new(&parent, proxy.clone())?;
        let panel_move = crate::window_move::WindowMoveLoop::new(panel.clone(), proxy.clone())?;
        Ok(Self { input: None, web: None, context: WebContext::new(Some(root.join("mypro-webview"))),
            toolbar, panel_move, panel, parent, proxy, generation: Rc::new(Cell::new(0)), queued: Rc::new(Cell::new(0)),
            ready: 0, last_request: 0, visible: false, shown: false, session: None, event_sequence: 0,
            background_revision: 0, visual_morph_revision: 0, services: ToolsService::new(root.clone()),
            next_native_auto_sample: Instant::now(),
            lineups: crate::mypro_features::lineups::Lineups::new(root.join("mypro")), features: Features::new(root) })
    }

    fn ensure_web(&mut self) -> anyhow::Result<()> {
        if self.web.is_some() { return Ok(()); }
        let navigating = self.generation.clone();
        let load_proxy = self.proxy.clone();
        let requests = self.generation.clone();
        let pending = self.queued.clone();
        let proxy = self.proxy.clone();
        let web = WebViewBuilder::new_with_web_context(&mut self.context)
            .with_custom_protocol("mypro".into(), |_, req| panel_response(req.method(), req.uri().path()))
            .with_url(PANEL_URL)
            .with_visible(false)
            .with_focused(false)
            .with_devtools(false)
            .with_navigation_handler(|url| url == PANEL_URL || url == PANEL_ORIGIN_URL)
            .with_new_window_req_handler(|_, _| NewWindowResponse::Deny)
            .with_on_page_load_handler(move |phase, _| match phase {
                PageLoadEvent::Started => navigating.set(navigating.get() + 1),
                PageLoadEvent::Finished => {
                    let _ = load_proxy.send_event(RuffleEvent::MyProPageReady(navigating.get()));
                }
            })
            .with_ipc_handler(move |request| {
                if request.uri().to_string() == PANEL_ORIGIN_URL && request.body().len() <= MAX_REQUEST_BYTES
                    && pending.get() < MAX_QUEUED_REQUESTS {
                    pending.set(pending.get() + 1);
                    if proxy.send_event(RuffleEvent::MyProRequest(requests.get(), request.into_body())).is_err() {
                        pending.set(pending.get() - 1);
                    }
                }
            })
            .build_as_child(&*self.panel)?;
        self.web = Some(web);
        Ok(())
    }

    pub fn child_attributes(&self, attributes: WindowAttributes) -> anyhow::Result<WindowAttributes> {
        let handle = self.parent.window_handle()?.as_raw();
        // App keeps this parent alive until after the child GUI, surface and PlayerController are dropped.
        Ok(unsafe { attributes.with_parent_window(Some(handle)) }
            .with_position(PhysicalPosition::new(0, toolbar_height(self.parent.scale_factor()) as i32))
            .with_decorations(false).with_resizable(false).with_skip_taskbar(true)
            .with_min_inner_size(PhysicalSize::new(1, 1)).with_clip_children(true))
    }

    pub fn attach_game(&mut self, window: Arc<Window>, proxy: EventLoopProxy<RuffleEvent>) -> anyhow::Result<()> {
        let input = GameInput { window, proxy: Box::new(proxy), keys: Vec::new(), buttons: Vec::new() };
        let id = &*input.proxy as *const _ as usize;
        // Registration and drop occur on the winit owner thread, while the HWND is alive.
        anyhow::ensure!(unsafe { SetWindowSubclass(hwnd(&input.window), Some(child_focus), id, id) } != 0,
            "Unable to track native game focus");
        self.input = Some(input);
        Ok(())
    }

    pub fn track_input(&mut self, event: &WindowEvent) {
        let Some(input) = &mut self.input else { return };
        match event {
            WindowEvent::KeyboardInput { event, .. } => {
                let key = crate::util::winit_input_to_ruffle_key_descriptor(event);
                input.keys.retain(|held| held.physical_key != key.physical_key);
                if event.state == ElementState::Pressed { input.keys.push(key); }
            }
            WindowEvent::MouseInput { state, button, .. } => {
                input.buttons.retain(|b| b != button);
                if *state == ElementState::Pressed {
                    unsafe { SetFocus(hwnd(&input.window)); }
                    input.buttons.push(*button);
                }
            }
            _ => {}
        }
    }

    pub fn focus_changed(&mut self, player: &PlayerController, focused: bool, pos: (f64, f64)) {
        let Some(input) = &mut self.input else { return };
        if !focused {
            for key in input.keys.drain(..) { player.handle_event(PlayerEvent::KeyUp { key }); }
            for b in input.buttons.drain(..) {
                use ruffle_core::events::MouseButton as Button;
                let button = match b {
                    MouseButton::Left => Button::Left, MouseButton::Middle => Button::Middle,
                    MouseButton::Right => Button::Right, _ => Button::Unknown,
                };
                player.handle_event(PlayerEvent::MouseUp { x:pos.0, y:pos.1, button });
            }
        }
        player.handle_event(if focused { PlayerEvent::FocusGained } else { PlayerEvent::FocusLost });
    }

    pub fn layout(&mut self, game: &Window) {
        let size = self.parent.inner_size();
        if size.width == 0 || size.height == 0 || self.parent.is_minimized() == Some(true) { return; }
        let scale = self.parent.scale_factor();
        let bar = toolbar_height(scale).min(size.height.saturating_sub(1));
        self.toolbar.layout(size.width, bar, scale);
        unsafe { MoveWindow(hwnd(game), 0, bar as i32, size.width as i32, (size.height - bar) as i32, 1); }
    }

    pub fn layout_panel(&mut self) {
        let Some(web) = &self.web else { return };
        let size = self.panel.inner_size();
        let shown = self.visible && size.width > 0 && size.height > 0 && self.panel.is_minimized() != Some(true);
        if self.shown != shown {
            self.shown = shown;
            let _ = web.set_visible(shown);
            let _ = web.evaluate_script(if shown {
                "window.myproVisible?.(true)"
            } else { "window.myproVisible?.(false)" });
        }
        if shown {
            let _ = web.set_bounds(Rect {
                position: wry::dpi::PhysicalPosition::new(0, 0).into(),
                size: wry::dpi::PhysicalSize::new(size.width, size.height).into(),
            });
        }
    }

    pub fn open(&mut self) {
        if let Err(err) = self.ensure_web() {
            tracing::error!("Unable to open MyPro tools: {err}");
            return;
        }
        self.visible = true;
        if self.panel.is_minimized() == Some(true) { self.panel.set_minimized(false); }
        self.panel.set_visible(true);
        self.layout_panel();
        self.panel.focus_window();
        if let Some(web) = &self.web { let _ = web.focus(); }
    }

    pub fn hide(&mut self, game: &Window) {
        self.visible = false;
        self.layout_panel();
        self.panel.set_visible(false);
        if self.parent.is_minimized() != Some(true) {
            self.parent.focus_window();
            unsafe { SetFocus(hwnd(game)); }
        }
    }

    pub fn toggle(&mut self, game: &Window) {
        if self.shown { self.hide(game); } else { self.open(); }
    }

    pub fn page_ready(&mut self, generation: u64) {
        if generation == 0 || generation != self.generation.get() || generation == self.ready { return; }
        self.ready = generation;
        self.last_request = 0;
        let Some(web) = &self.web else { return };
        let _ = web.evaluate_script(&format!(
            "window.myproGeneration={generation};if(!{})window.myproVisible?.(false);window.dispatchEvent(new Event('mypro-ready'));", self.shown
        ));
    }

    fn session_name(session: Option<i64>) -> String {
        session.map_or_else(|| "idle".into(), |id| format!("game-{id}"))
    }

    pub fn sync_session(&mut self, player: &PlayerController) {
        let current = player.session_id();
        crate::mypro_features::game_ui::sync(player);
        let now = Instant::now();
        if now >= self.next_native_auto_sample {
            self.next_native_auto_sample = now + Duration::from_millis(250);
            if let Some(mut movie) = player.get() && movie.battle_input_waiting() {
                Self::native_auto_result(&mut movie, "getNativeAutoBattle", None);
            }
        }
        let revision = crate::mypro_features::background::revision();
        if revision != self.background_revision {
            if let Some(mut movie) = player.get() {
                movie.try_call_internal_interface("aolaRefreshBattleBackground", []);
                self.background_revision = revision;
            }
        }
        let visual_revision = ruffle_core::mypro_observer::morph_rule_snapshot().revision;
        if visual_revision != self.visual_morph_revision && !crate::mypro_services::in_battle()
            && let Some(mut movie) = player.get()
        {
            movie.try_call_internal_interface("onAolaMyProVisualRefresh", []);
            self.visual_morph_revision = visual_revision;
        }
        if self.session != Some(current) { self.lineups.reset(); }
        self.lineups.tick(player,crate::mypro_services::in_battle);
        if self.session == Some(current) { return; }
        self.session = Some(current);
        if let Some(input) = &mut self.input {
            input.keys.clear();
            input.buttons.clear();
        }
        let id = Self::session_name(current);
        self.services.set_session(&id);
        self.features.set_session(&id);
        if self.ready > 0 && self.ready == self.generation.get() {
            self.event_sequence += 1;
            self.deliver(&json!({"protocolVersion":1,"kind":"event","event":"session.changed","seq":self.event_sequence,
                "bridgeGeneration":self.ready,"sessionId":id,"payload":{"sessionId":id}}));
        }
    }

    pub fn stop(&mut self) {
        self.ready = 0;
        self.lineups.reset();
        self.services.set_session("idle");
        self.features.set_session("idle");
    }

    fn deliver(&self, value: &Value) {
        if let Some(web) = &self.web { let _ = web.evaluate_script(&format!("window.myproReceive?.({value})")); }
    }

    fn snapshot(&self, player: &PlayerController, game: &Window, session: &str) -> Value {
        let state = player.get().map(|p| (p.is_playing(), p.performance_stats().frames, p.base_frame_rate()));
        let playing = state.map(|s| s.0);
        let mut capabilities = json!({});
        for name in ["session.pause","session.resume","panel.hide","panel.reload","battle.auto.read","battle.auto.set",
            "game.panels","game.panel.open","theme.read","theme.apply","workshop.readOwnBag","lineups.read","lineups.refresh","lineups.save","lineups.restore"] {
            let available = matches!(name,"panel.hide"|"panel.reload"|"game.panels") || playing.is_some();
            capabilities[name] = json!({"available":available,"reason":if available {""} else {"游戏尚未载入"}});
        }
        for additional in [self.services.capabilities(), self.features.capabilities()] {
            if let Some(items) = additional.as_object() {
                capabilities.as_object_mut().unwrap().extend(items.clone());
            }
        }
        let size = game.inner_size();
        json!({"sessionId":session,"hasMovie":playing.is_some(),"playing":playing.unwrap_or(false),
            "logicalFrames":state.map(|s| s.1),"targetFps":state.map(|s| s.2),
            "viewport":{"width":size.width,"height":size.height},"capabilities":capabilities,
            "coreBuild":concat!("Ruffle ",env!("CARGO_PKG_VERSION"))})
    }

    fn native_auto_result(
        movie: &mut ruffle_core::Player,
        callback: &str,
        arg: Option<ruffle_core::external::Value>,
    ) -> Option<ruffle_core::external::Value> {
        use ruffle_core::external::Value as Flash;
        let result = movie.try_call_internal_interface(callback, arg);
        let enabled = matches!(&result, Some(Flash::Object(state)) if state.get("enabled") == Some(&Flash::Bool(true)));
        movie.set_battle_native_auto_enabled(enabled);
        result
    }

    fn native_auto(player: &PlayerController, command: &str, payload: &Value) -> Result<Value, String> {
        use ruffle_core::external::Value as Flash;
        let (callback, arg) = if command == "battle.auto.set" {
            let enabled = payload["enabled"].as_bool().ok_or("InvalidArguments: expected enabled")?;
            ("setNativeAutoBattle", Some(Flash::Bool(enabled)))
        } else { ("getNativeAutoBattle", None) };
        if let Some(Flash::Object(result)) = player.get().and_then(|mut p| Self::native_auto_result(&mut p, callback, arg)) {
            if let Some(Flash::String(status)) = result.get("status") {
                let enabled = match result.get("enabled") { Some(Flash::Bool(flag)) => Some(*flag), _ => None };
                let changed = matches!(result.get("changed"), Some(Flash::Bool(true)));
                return Ok(json!({"status":status,"enabled":enabled,"changed":changed}));
            }
        }
        Ok(json!({"status":"unavailable","enabled":null,"changed":false}))
    }

    pub fn request(&mut self, generation: u64, body: &str, player: &PlayerController, game: &Window) {
        self.queued.set(self.queued.get().saturating_sub(1));
        if generation == 0 || generation != self.ready || generation != self.generation.get() { return; }
        let Ok(input) = serde_json::from_str::<Value>(body) else { return };
        let Some(req) = Request::parse(&input) else { return };
        if req.bridge_generation != generation { return; }
        let Some(seq) = req.request_id.strip_prefix("ui-").and_then(|n| n.parse::<u64>().ok()) else { return };
        if seq <= self.last_request { return; }
        self.last_request = seq;
        self.sync_session(player);
        let session = Self::session_name(player.session_id());
        let result = if req.protocol_version != 1 {
            Err("UnsupportedProtocolVersion".into())
        } else if !req.payload.is_object() {
            Err("InvalidArguments".into())
        } else if req.command != "app.snapshot" && req.session_id != Some(session.as_str()) {
            Err("StaleContext".into())
        } else { match req.command {
            "app.snapshot" => Ok(self.snapshot(player, game, &session)),
            "session.pause" | "session.resume" => player.get().map(|mut p| {
                p.set_is_playing(req.command == "session.resume");
                json!({"playing":p.is_playing()})
            }).ok_or_else(|| "NotInGame".to_string()),
            "panel.hide" | "panel.reload" => Ok(json!({})),
            "battle.auto.read" | "battle.auto.set" => Self::native_auto(player, req.command, req.payload),
            "workshop.readOwnBag" => crate::mypro_features::workshop::validate(req.payload).and_then(|script| {
                if script["canReadOwnBag"]!=true { return Err("InvalidArguments: 此文本没有单一的自身背包读取操作".into()); }
                self.lineups.command("lineups.refresh",&json!({}),player,crate::mypro_services::in_battle())
                    .unwrap_or_else(||Err("UnsupportedCapability".into()))
            }),
            name @ ("game.panels" | "game.panel.open" | "theme.read" | "theme.apply") =>
                crate::mypro_features::game_ui::command(player,name,req.payload).unwrap_or_else(|| Err("UnsupportedCapability".into())),
            name @ ("lineups.read" | "lineups.refresh" | "lineups.save" | "lineups.restore") =>
                self.lineups.command(name,req.payload,player,crate::mypro_services::in_battle()).unwrap_or_else(|| Err("UnsupportedCapability".into())),
            name => self.services.command(name, req.payload)
                .or_else(|| self.features.command(name, req.payload))
                .unwrap_or_else(|| Err("UnsupportedCapability".into())),
        }};
        let success = result.is_ok();
        if success && req.command == "settings.patch" && req.payload["values"].get("battleInputNormalFps").is_some()
            && let Some(mut movie) = player.get()
        {
            movie.apply_dynamic_frame_rate();
        }
        let mut response = json!({"protocolVersion":1,"kind":"response","requestId":req.request_id,
            "sessionId":session,"bridgeGeneration":generation,"ok":success});
        match result {
            Ok(value) => response["result"] = value,
            Err(message) => {
                let code = message.split_once(':').map_or(message.as_str(), |(code, _)| code);
                response["error"] = json!({"code":code,"message":message});
            }
        }
        self.deliver(&response);
        if success {
            match req.command {
                "panel.hide" if self.visible => self.hide(game),
                "panel.reload" => { if let Some(web) = &self.web { let _ = web.reload(); } }
                _ => {}
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mypro_panel_serves_only_embedded_battle_assets() {
        use wry::http::{Method, StatusCode};

        let png = panel_response(&Method::GET, "/battle-assets/equipment/10227.png");
        assert_eq!(png.status(), StatusCode::OK);
        assert_eq!(png.headers()["Content-Type"], "image/png");
        assert!(png.body().starts_with(b"\x89PNG\r\n\x1a\n"));
        let names = panel_response(&Method::GET, "/battle-assets/names.json");
        assert_eq!(names.headers()["Content-Type"], "application/json");
        let catalog: Value = serde_json::from_slice(names.body()).unwrap();
        assert_eq!(catalog["equipment:10227"], "鹿鸣仙风王者战剑LV3");
        for path in ["/battle-assets/pet/0.png", "/battle-assets/../index.html", "/unlisted"] {
            assert_eq!(panel_response(&Method::GET, path).status(), StatusCode::NOT_FOUND);
        }
        assert_eq!(panel_response(&Method::POST, "/index.html").status(), StatusCode::NOT_FOUND);
    }

    #[test]
    fn mypro_native_game_has_700_physical_pixels_below_dpi_scaled_toolbar() {
        for (scale, bar) in [(1.0,36),(1.25,45),(1.5,54),(2.0,72)] {
            assert_eq!(toolbar_height(scale), bar);
        }
    }
}
