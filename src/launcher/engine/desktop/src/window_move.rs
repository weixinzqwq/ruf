//! Drive the existing player loop while DefWindowProc owns the move/size loop.
use crate::custom_event::RuffleEvent;
use std::cell::Cell;
use std::rc::Rc;
use std::sync::Arc;
use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
use windows_sys::Win32::UI::Shell::{DefSubclassProc, RemoveWindowSubclass, SetWindowSubclass};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    KillTimer, SetTimer, WM_ENTERSIZEMOVE, WM_EXITSIZEMOVE, WM_NCDESTROY, WM_TIMER,
};
use winit::event_loop::EventLoopProxy;
use winit::raw_window_handle::{HasWindowHandle, RawWindowHandle};
use winit::window::{Window, WindowId};

struct State {
    proxy: EventLoopProxy<RuffleEvent>,
    window_id: WindowId,
    installed: Cell<bool>,
    active: Cell<bool>,
    pending: Cell<bool>,
    ticks: Cell<u64>,
}

impl State {
    fn wake(&self) {
        if self.active.get() && !self.pending.replace(true)
            && self.proxy.send_event(RuffleEvent::WindowMoveTick(self.window_id)).is_err()
        {
            self.pending.set(false);
        }
    }
}

pub(super) struct WindowMoveLoop {
    // Keep both the HWND and subclass data alive; Rc also keeps this guard on
    // the window thread. Removing the subclass precedes releasing either one.
    _window: Arc<Window>,
    hwnd: HWND,
    state: Rc<State>,
}

impl WindowMoveLoop {
    pub fn new(window: Arc<Window>, proxy: EventLoopProxy<RuffleEvent>) -> anyhow::Result<Self> {
        let RawWindowHandle::Win32(handle) = window.window_handle()?.as_raw() else {
            anyhow::bail!("Expected a Windows window");
        };
        let hwnd = handle.hwnd.get() as HWND;
        let state = Rc::new(State {
            proxy, window_id: window.id(), installed: Cell::new(false),
            active: Cell::new(false), pending: Cell::new(false), ticks: Cell::new(0),
        });
        let id = Rc::as_ptr(&state) as usize;
        // SAFETY: created on the HWND's thread. Rc storage remains stable until
        // Drop removes the callback. The allocation address is also our unique
        // window-local timer ID, so other components' timers are untouched.
        if unsafe { SetWindowSubclass(hwnd, Some(window_proc), id, id) } == 0 {
            anyhow::bail!("Could not install window move timer");
        }
        state.installed.set(true);
        Ok(Self { _window: window, hwnd, state })
    }

    pub fn tick_pending(&self) -> bool {
        self.state.active.get() && self.state.pending.get()
    }

    pub fn finish_tick(&self) {
        self.state.pending.set(false);
        self.state.ticks.set(self.state.ticks.get() + 1);
    }
}

impl Drop for WindowMoveLoop {
    fn drop(&mut self) {
        if self.state.installed.replace(false) {
            let id = Rc::as_ptr(&self.state) as usize;
            // SAFETY: the guard owns the window and runs on its thread.
            unsafe {
                KillTimer(self.hwnd, id);
                RemoveWindowSubclass(self.hwnd, Some(window_proc), id);
            }
        }
    }
}

unsafe extern "system" fn window_proc(
    hwnd: HWND, message: u32, wparam: WPARAM, lparam: LPARAM, id: usize, data: usize,
) -> LRESULT {
    // SAFETY: the guard owns this Rc allocation until after subclass removal.
    // Only Cell mutation is used, including during nested window messages.
    let state = unsafe { &*(data as *const State) };
    match message {
        WM_ENTERSIZEMOVE => {
            if !state.active.replace(true) {
                state.ticks.set(0);
                // USER_TIMER_MINIMUM is 10 ms. The player's elapsed-time clock
                // still decides when frames are due; this is only a wake-up.
                if unsafe { SetTimer(hwnd, id, 10, None) } == 0 {
                    tracing::warn!("Window move timer could not be started");
                }
                tracing::info!("Window move/size loop entered");
                state.wake();
            }
        }
        WM_TIMER if wparam == id => {
            state.wake();
            return 0;
        }
        WM_EXITSIZEMOVE => {
            state.active.set(false);
            state.pending.set(false);
            unsafe { KillTimer(hwnd, id); }
            tracing::info!(ticks = state.ticks.get(), "Window move/size loop exited");
        }
        WM_NCDESTROY => {
            state.active.set(false);
            state.pending.set(false);
            state.installed.set(false);
            unsafe {
                KillTimer(hwnd, id);
                RemoveWindowSubclass(hwnd, Some(window_proc), id);
            }
        }
        _ => {}
    }
    // Preserve winit's resizing, capture, snapping and WebView subclass chain.
    unsafe { DefSubclassProc(hwnd, message, wparam, lparam) }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        DispatchMessageW, MSG, PM_REMOVE, PeekMessageW, PostMessageW, SendMessageW, WM_APP,
    };
    use winit::application::ApplicationHandler;
    use winit::event::{StartCause, WindowEvent};
    use winit::event_loop::{ActiveEventLoop, EventLoop};
    use winit::platform::run_on_demand::EventLoopExtRunOnDemand;
    use winit::platform::windows::EventLoopBuilderExtWindows;

    const TEST_NESTED_LOOP: u32 = WM_APP + 71;

    #[derive(Default)]
    struct Probe {
        nested: Cell<bool>,
        completed: Cell<bool>,
        ticks: Cell<u64>,
        about_to_wait_inside: Cell<u64>,
        before: Cell<u64>,
        moving: Cell<u64>,
        after: Cell<u64>,
        moving_again: Cell<u64>,
        after_drop: Cell<u64>,
    }

    // Exercise an actual nested Win32 dispatch loop while winit's outer loop
    // is suspended, including holding the mouse still (no move/resize events).
    unsafe fn pump_for(duration: Duration) {
        let until = Instant::now() + duration;
        while Instant::now() < until {
            let mut message = MSG::default();
            if unsafe { PeekMessageW(&mut message, std::ptr::null_mut(), 0, 0, PM_REMOVE) } != 0 {
                unsafe { DispatchMessageW(&message); }
            } else {
                std::thread::sleep(Duration::from_millis(1));
            }
        }
    }

    unsafe extern "system" fn test_proc(
        hwnd: HWND, message: u32, wparam: WPARAM, lparam: LPARAM, id: usize, data: usize,
    ) -> LRESULT {
        if message == TEST_NESTED_LOOP {
            println!("native move test: entering nested message loop");
            let probe = unsafe { &*(data as *const Probe) };
            probe.nested.set(true);
            unsafe {
                pump_for(Duration::from_millis(80));
                probe.before.set(probe.ticks.get());
                println!("native move test: entering move");
                SendMessageW(hwnd, WM_ENTERSIZEMOVE, 0, 0);
                pump_for(Duration::from_millis(200));
                probe.moving.set(probe.ticks.get());
                println!("native move test: first move ticks={}", probe.moving.get());
                SendMessageW(hwnd, WM_EXITSIZEMOVE, 0, 0);
                pump_for(Duration::from_millis(80));
                probe.after.set(probe.ticks.get());
                SendMessageW(hwnd, WM_ENTERSIZEMOVE, 0, 0);
                pump_for(Duration::from_millis(150));
                probe.moving_again.set(probe.ticks.get());
                // Test dropping the guard with a live timer as well as the
                // normal WM_EXITSIZEMOVE cleanup above.
                pump_for(Duration::from_millis(100));
                probe.after_drop.set(probe.ticks.get());
                RemoveWindowSubclass(hwnd, Some(test_proc), id);
            }
            probe.nested.set(false);
            probe.completed.set(true);
            println!("native move test: leaving nested message loop");
            return 0;
        }
        unsafe { DefSubclassProc(hwnd, message, wparam, lparam) }
    }

    struct TestApp {
        proxy: EventLoopProxy<RuffleEvent>,
        window: Option<Arc<Window>>,
        guard: Option<WindowMoveLoop>,
        probe: Rc<Probe>,
        started: bool,
    }

    impl ApplicationHandler<RuffleEvent> for TestApp {
        fn resumed(&mut self, event_loop: &ActiveEventLoop) {
            println!("native move test: creating hidden window");
            let window = Arc::new(event_loop.create_window(
                Window::default_attributes().with_visible(false),
            ).expect("hidden test window"));
            let guard = WindowMoveLoop::new(window.clone(), self.proxy.clone()).expect("move hook");
            let hwnd = guard.hwnd;
            let probe = Rc::as_ptr(&self.probe) as usize;
            assert_ne!(unsafe { SetWindowSubclass(hwnd, Some(test_proc), probe, probe) }, 0);
            unsafe { PostMessageW(hwnd, TEST_NESTED_LOOP, 0, 0); }
            self.window = Some(window);
            self.guard = Some(guard);
            self.started = true;
            println!("native move test: hidden window ready");
        }

        fn user_event(&mut self, _event_loop: &ActiveEventLoop, event: RuffleEvent) {
            if let RuffleEvent::WindowMoveTick(id) = event
                && let Some(guard) = &self.guard
                && id == guard.state.window_id
                && guard.tick_pending()
            {
                self.probe.ticks.set(self.probe.ticks.get() + 1);
                // Reentrant wake-ups must stay coalesced until this update ends.
                let timer_id = Rc::as_ptr(&guard.state) as usize;
                unsafe { SendMessageW(guard.hwnd, WM_TIMER, timer_id, 0); }
                guard.finish_tick();
                if self.probe.moving_again.get() != 0 {
                    self.guard.take();
                }
            }
        }

        fn window_event(&mut self, _: &ActiveEventLoop, _: WindowId, _: WindowEvent) {}

        fn new_events(&mut self, _: &ActiveEventLoop, _: StartCause) {}

        fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
            if self.probe.nested.get() {
                self.probe.about_to_wait_inside.set(self.probe.about_to_wait_inside.get() + 1);
            } else if self.started && self.probe.completed.get() {
                // run_on_demand waits once after AboutToWait even when exit
                // was requested there; do not leave the hidden test asleep.
                event_loop.set_control_flow(winit::event_loop::ControlFlow::Poll);
                event_loop.exit();
            }
        }
    }

    #[test]
    fn window_move_updates_inside_native_loop_and_stops_after_exit_or_drop() {
        println!("native move test: creating event loop");
        let mut event_loop = EventLoop::<RuffleEvent>::with_user_event()
            .with_any_thread(true).build().expect("test event loop");
        let probe = Rc::new(Probe::default());
        let mut app = TestApp {
            proxy: event_loop.create_proxy(), window: None, guard: None,
            probe: probe.clone(), started: false,
        };
        println!("native move test: starting event loop");
        event_loop.run_app_on_demand(&mut app).expect("nested loop test");
        println!("native move loop: before={}, moving={}, after={}, moving_again={}, after_drop={}, outer_waits={}",
            probe.before.get(), probe.moving.get(), probe.after.get(),
            probe.moving_again.get(), probe.after_drop.get(), probe.about_to_wait_inside.get());
        assert_eq!(probe.about_to_wait_inside.get(), 0, "must exercise the suspended outer loop");
        assert_eq!(probe.before.get(), 0, "no background ticking while not moving");
        assert!(probe.moving.get() >= 3, "updates must continue without mouse movement");
        assert_eq!(probe.after.get(), probe.moving.get(), "exit must cancel timer and stale pulses");
        assert!(probe.moving_again.get() >= probe.after.get() + 3, "second drag must restart ticking");
        assert_eq!(probe.after_drop.get(), probe.moving_again.get() + 1, "guard drop must stop a live timer");
        assert!(probe.after_drop.get() < 100, "coalescing must prevent a self-sustaining update queue");
    }
}
