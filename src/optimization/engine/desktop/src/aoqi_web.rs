//! Windows WebView2 implementation of the recovered aoqi.platform.WebView bridge.
//!
//! All methods and callbacks run on the window event-loop thread. In particular,
//! the navigation policy must call ActionScript synchronously to preserve
//! WebViewEvent.NAVIGATING.preventDefault().

use serde_json::Value;
use std::cell::{Cell, RefCell};
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::mpsc::Sender;
use webview2_com::Microsoft::Web::WebView2::Win32::{
    ICoreWebView2, COREWEBVIEW2_WEB_ERROR_STATUS_UNKNOWN,
};
use webview2_com::{
    take_pwstr, ExecuteScriptCompletedHandler, NavigationCompletedEventHandler,
    NavigationStartingEventHandler, SourceChangedEventHandler, WebMessageReceivedEventHandler,
};
use windows::core::{BOOL, HSTRING, PWSTR};
use winit::window::Window;
use wry::dpi::{LogicalPosition, LogicalSize};
use wry::{Rect, WebContext, WebView, WebViewBuilder, WebViewExtWindows};

#[derive(Debug)]
pub enum WebCommand {
    Create { id: String },
    Load { id: String, url: String },
    Run { id: String, script: String },
    Post { id: String, message: String },
    PostJson { id: String, message: Value },
    Frame { id: String, x: f64, y: f64, width: f64, height: f64 },
    Dispose { id: String },
}

impl WebCommand {
    fn id(&self) -> &str {
        match self {
            Self::Create { id }
            | Self::Load { id, .. }
            | Self::Run { id, .. }
            | Self::Post { id, .. }
            | Self::PostJson { id, .. }
            | Self::Frame { id, .. }
            | Self::Dispose { id } => id,
        }
    }
}

#[derive(Debug)]
pub struct WebEvent {
    pub id: String,
    pub kind: &'static str,
    pub payload: Value,
}

/// Return true to allow navigation. The recovered AS callback returns the
/// opposite: hostDispatch("web:" + id, "navigating", url) == true cancels it.
pub type NavigationHandler = Rc<dyn Fn(&str, &str) -> bool>;

struct View {
    webview: WebView,
    alive: Rc<Cell<bool>>,
    frame: Option<[f64; 4]>,
}

impl Drop for View {
    fn drop(&mut self) {
        self.alive.set(false);
        // Wry closes the WebView2 controller and destroys its child HWND.
    }
}

pub struct WebViews {
    // Drop controllers before their shared context and parent window.
    views: HashMap<String, View>,
    context: WebContext,
    profile_root: PathBuf,
    sender: Sender<WebEvent>,
    navigation_handler: Rc<RefCell<Option<NavigationHandler>>>,
}

impl WebViews {
    pub fn new(profile_root: PathBuf, sender: Sender<WebEvent>) -> Self {
        Self {
            views: HashMap::new(),
            context: WebContext::new(Some(profile_root.clone())),
            profile_root,
            sender,
            navigation_handler: Rc::new(RefCell::new(None)),
        }
    }

    pub fn set_navigation_handler(&mut self, handler: NavigationHandler) {
        *self.navigation_handler.borrow_mut() = Some(handler);
    }

    /// Frame coordinates are parent-client logical pixels (DIPs), not physical
    /// screen coordinates. Convert the SWF viewport transform before calling.
    pub fn execute(&mut self, parent: &Window, command: WebCommand) -> Result<(), String> {
        let id = command.id().to_owned();
        let result = self.execute_inner(parent, command);
        if result.is_err() {
            // Synchronous setup/command failures have no WebErrorStatus. -1 is
            // the reconstruction's explicit native-command error code.
            emit(&self.sender, &id, "failed", Value::from(-1));
        }
        result
    }

    fn execute_inner(&mut self, parent: &Window, command: WebCommand) -> Result<(), String> {
        match command {
            WebCommand::Create { id } => self.create(parent, id),
            WebCommand::Dispose { id } => {
                self.views.remove(&id);
                Ok(())
            }
            WebCommand::Load { id, url } => self.view(&id)?.webview.load_url(&url).map_err(display_error),
            WebCommand::Run { id, script } => {
                let view = self.view(&id)?;
                let alive = view.alive.clone();
                let sender = self.sender.clone();
                // Unlike Wry's convenience evaluate_script method, retain a
                // failing ExecuteScript HRESULT for the bridge's failed event.
                unsafe {
                    view.webview.webview().ExecuteScript(
                        &HSTRING::from(script),
                        &ExecuteScriptCompletedHandler::create(Box::new(move |result, _| {
                            if alive.get() {
                                if let Err(error) = result {
                                    emit(&sender, &id, "failed", Value::from(error.code().0));
                                }
                            }
                            Ok(())
                        })),
                    )
                }
                .map_err(display_error)
            }
            WebCommand::Post { id, message } => {
                // Native WebView2 message delivery, preserving the original
                // string in chrome.webview's `message` event.data.
                unsafe {
                    self.view(&id)?
                        .webview
                        .webview()
                        .PostWebMessageAsString(&HSTRING::from(message))
                }
                .map_err(display_error)
            }
            WebCommand::PostJson { id, message } => unsafe {
                self.view(&id)?
                    .webview
                    .webview()
                    .PostWebMessageAsJson(&HSTRING::from(message.to_string()))
            }
            .map_err(display_error),
            WebCommand::Frame { id, x, y, width, height } => {
                let view = self.views.get_mut(&id).ok_or_else(|| missing_view(&id))?;
                if ![x, y, width, height].iter().all(|value| value.is_finite()) {
                    return Err("WebView frame contains a non-finite coordinate".to_owned());
                }
                if width <= 0.0 || height <= 0.0 {
                    view.webview.set_visible(false).map_err(display_error)?;
                    view.frame = None;
                } else {
                    let frame = [x, y, width, height];
                    view.webview.set_bounds(bounds(frame)).map_err(display_error)?;
                    view.webview.set_visible(true).map_err(display_error)?;
                    view.frame = Some(frame);
                }
                Ok(())
            }
        }
    }

    fn view(&self, id: &str) -> Result<&View, String> {
        self.views.get(id).ok_or_else(|| missing_view(id))
    }

    fn create(&mut self, parent: &Window, id: String) -> Result<(), String> {
        if self.views.contains_key(&id) {
            return Err(format!("WebView {id} already exists"));
        }
        if !self.profile_root.is_absolute() {
            return Err("WebView profile directory must be an absolute isolated path".to_owned());
        }
        std::fs::create_dir_all(&self.profile_root).map_err(display_error)?;

        let alive = Rc::new(Cell::new(true));
        let title_alive = alive.clone();
        let title_id = id.clone();
        let title_sender = self.sender.clone();
        let webview = WebViewBuilder::new_with_web_context(&mut self.context)
            .with_visible(false)
            .with_focused(false)
            .with_bounds(bounds([0.0, 0.0, 1.0, 1.0]))
            .with_document_title_changed_handler(move |title| {
                if title_alive.get() {
                    emit(&title_sender, &title_id, "title", Value::String(title));
                }
            })
            .build_as_child(parent)
            .map_err(display_error)?;

        let core = webview.webview();
        self.attach_events(&core, &id, &alive).map_err(display_error)?;
        self.views.insert(id, View { webview, alive, frame: None });
        Ok(())
    }

    fn attach_events(
        &self,
        core: &ICoreWebView2,
        id: &str,
        alive: &Rc<Cell<bool>>,
    ) -> windows::core::Result<()> {
        let cancelled = Rc::new(RefCell::new(HashSet::<u64>::new()));
        let starting_cancelled = cancelled.clone();
        let starting_handler = self.navigation_handler.clone();
        let starting_alive = alive.clone();
        let starting_id = id.to_owned();
        let starting_sender = self.sender.clone();
        let changed_alive = alive.clone();
        let changed_id = id.to_owned();
        let changed_sender = self.sender.clone();
        let completed_alive = alive.clone();
        let completed_id = id.to_owned();
        let completed_sender = self.sender.clone();
        let message_alive = alive.clone();
        let message_id = id.to_owned();
        let message_sender = self.sender.clone();
        let mut token = 0;
        // WebView2 invokes these on the creating STA/UI thread. No player or
        // host mutex may remain locked while execute() pumps WebView2 messages.
        unsafe {
            core.add_WebMessageReceived(
                &WebMessageReceivedEventHandler::create(Box::new(move |_, args| {
                    let Some(args) = args else { return Ok(()) };
                    if !message_alive.get() {
                        return Ok(());
                    }
                    // AS WebView.messageScript posts a string; the native
                    // HTML panel posts an object. Preserve both wire types.
                    let mut raw = PWSTR::null();
                    let payload = if args.TryGetWebMessageAsString(&mut raw).is_ok() {
                        Value::String(take_pwstr(raw))
                    } else {
                        args.WebMessageAsJson(&mut raw)?;
                        let json = take_pwstr(raw);
                        serde_json::from_str(&json).unwrap_or(Value::String(json))
                    };
                    emit(&message_sender, &message_id, "message", payload);
                    Ok(())
                })),
                &mut token,
            )?;
            core.add_NavigationStarting(
                &NavigationStartingEventHandler::create(Box::new(move |_, args| {
                    let Some(args) = args else { return Ok(()) };
                    if !starting_alive.get() {
                        return args.SetCancel(true);
                    }
                    let mut uri = PWSTR::null();
                    args.Uri(&mut uri)?;
                    let url = take_pwstr(uri);
                    // Release RefCell borrow before reentering ActionScript.
                    let handler = starting_handler.borrow().clone();
                    let allow = if let Some(handler) = handler {
                        handler(&starting_id, &url)
                    } else {
                        emit(&starting_sender, &starting_id, "navigating", Value::String(url));
                        true
                    };
                    if !allow {
                        let mut navigation_id = 0;
                        args.NavigationId(&mut navigation_id)?;
                        starting_cancelled.borrow_mut().insert(navigation_id);
                        args.SetCancel(true)?;
                    }
                    Ok(())
                })),
                &mut token,
            )?;
            core.add_SourceChanged(
                &SourceChangedEventHandler::create(Box::new(move |webview, _| {
                    if changed_alive.get() {
                        if let Some(webview) = webview {
                            emit(&changed_sender, &changed_id, "navigated", Value::String(source(&webview)?));
                        }
                    }
                    Ok(())
                })),
                &mut token,
            )?;
            core.add_NavigationCompleted(
                &NavigationCompletedEventHandler::create(Box::new(move |webview, args| {
                    let (Some(webview), Some(args)) = (webview, args) else { return Ok(()) };
                    if !completed_alive.get() {
                        return Ok(());
                    }
                    let mut navigation_id = 0;
                    args.NavigationId(&mut navigation_id)?;
                    if cancelled.borrow_mut().remove(&navigation_id) {
                        // preventDefault is an intentional cancellation, not a
                        // page-load error (notably LoginOAuth.changing).
                        return Ok(());
                    }
                    let mut success = BOOL::default();
                    args.IsSuccess(&mut success)?;
                    if success.as_bool() {
                        emit(&completed_sender, &completed_id, "loaded", Value::String(source(&webview)?));
                    } else {
                        let mut status = COREWEBVIEW2_WEB_ERROR_STATUS_UNKNOWN;
                        args.WebErrorStatus(&mut status)?;
                        emit(&completed_sender, &completed_id, "failed", Value::from(status.0));
                    }
                    Ok(())
                })),
                &mut token,
            )?;
        }
        Ok(())
    }

    /// Reapply DIP bounds after parent DPI/size changes. The parent is
    /// responsible for recalculating the SWF-to-window viewport transform.
    pub fn resize(&self) -> Result<(), String> {
        for view in self.views.values() {
            if let Some(frame) = view.frame {
                view.webview.set_bounds(bounds(frame)).map_err(display_error)?;
            }
        }
        Ok(())
    }

    pub fn dispose_all(&mut self) {
        self.views.clear();
    }
}

fn bounds([x, y, width, height]: [f64; 4]) -> Rect {
    Rect {
        position: LogicalPosition::new(x, y).into(),
        size: LogicalSize::new(width, height).into(),
    }
}

fn source(webview: &ICoreWebView2) -> windows::core::Result<String> {
    let mut uri = PWSTR::null();
    unsafe { webview.Source(&mut uri)? };
    Ok(take_pwstr(uri))
}

fn emit(sender: &Sender<WebEvent>, id: &str, kind: &'static str, payload: Value) {
    let _ = sender.send(WebEvent { id: id.to_owned(), kind, payload });
}

fn missing_view(id: &str) -> String {
    format!("WebView {id} does not exist")
}

fn display_error(error: impl std::fmt::Display) -> String {
    error.to_string()
}
