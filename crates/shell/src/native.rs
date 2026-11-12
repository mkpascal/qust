// The shell thread: a thread of its own with a hidden window and a plain message loop, which owns
// the global shortcuts (WM_HOTKEY), the tray icon and its menu, and the single-instance window
// (WM_COPYDATA from a second launch). It doesn't depend on GPUI: it runs the same whether the UI
// is up, headless or not started, it costs nothing while idle (GetMessage), and the tray menu's
// modal loop runs here, so it never holds up the windows' rendering.
//
// What happens here reaches the app through the event handler (`on_event`), called on this
// thread; the app forwards it to its own thread (see `gpui_glue`).

use crate::win::{self, HiddenWindow};
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::System::Threading::GetCurrentThreadId;
use windows::Win32::UI::WindowsAndMessaging::*;

// What the shell thread reports.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Event {
    // A registered global shortcut was pressed (its id, see `hotkeys`).
    Hotkey(i32),
    // The tray icon was clicked with the left button (on release).
    TrayClick,
    // A tray menu item was chosen.
    TrayMenu(crate::tray::TrayCommand),
    // Another launch of Qust handed over its command line (argv, including the executable) and
    // working directory.
    Launched { args: Vec<String>, cwd: String },
    // The system's light / dark setting or accent color may have changed (WM_SETTINGCHANGE
    // "ImmersiveColorSet", WM_DWMCOLORIZATIONCOLORCHANGED): read `theme::system_dark()` and
    // `theme::accent()` again. Arrives with or without windows open.
    AppearanceChanged,
}

type Sink = Arc<dyn Fn(Event) + Send + Sync>;
type Job = Box<dyn FnOnce() + Send>;

const CLASS: &str = "QustShell";
pub(crate) const WM_CALL: u32 = WM_APP + 1;
pub(crate) const WM_TRAY: u32 = WM_APP + 2;
const WM_STOP: u32 = WM_APP + 3;

struct Shared {
    sink: Mutex<Option<Sink>>,
}

#[derive(Clone)]
pub struct NativeThread {
    hwnd: isize,
    thread_id: u32,
    shared: Arc<Shared>,
    join: Arc<Mutex<Option<JoinHandle<()>>>>,
}

thread_local! {
    // Set on the shell thread only.
    static SHARED: std::cell::RefCell<Option<Arc<Shared>>> = const { std::cell::RefCell::new(None) };
}

impl NativeThread {
    // Starts the thread and returns once its window exists.
    pub fn start() -> std::io::Result<NativeThread> {
        let shared = Arc::new(Shared { sink: Mutex::new(None) });
        let (ready_tx, ready_rx) = mpsc::channel::<Result<(isize, u32), String>>();
        let thread_shared = shared.clone();
        let join = std::thread::Builder::new().name("qust-shell".into()).spawn(move || {
            SHARED.with(|cell| *cell.borrow_mut() = Some(thread_shared));
            let window = match HiddenWindow::with_proc(CLASS, Some("Qust shell"), proc) {
                Ok(window) => window,
                Err(error) => {
                    let _ = ready_tx.send(Err(error.to_string()));
                    return;
                }
            };
            crate::tray::thread_started(window.hwnd);
            let _ = ready_tx.send(Ok((window.hwnd.0 as isize, unsafe { GetCurrentThreadId() })));
            let mut msg = MSG::default();
            unsafe {
                while GetMessageW(&mut msg, None, 0, 0).as_bool() {
                    let _ = TranslateMessage(&msg);
                    DispatchMessageW(&msg);
                }
            }
            drop(window);
        })?;
        let (hwnd, thread_id) = ready_rx
            .recv()
            .map_err(|_| std::io::Error::other("the shell thread ended"))?
            .map_err(std::io::Error::other)?;
        Ok(NativeThread { hwnd, thread_id, shared, join: Arc::new(Mutex::new(Some(join))) })
    }

    // Where the events go (replaces the previous handler). Called on the shell thread.
    pub fn on_event(&self, handler: impl Fn(Event) + Send + Sync + 'static) {
        *self.shared.sink.lock().unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(Arc::new(handler));
    }

    pub fn hwnd(&self) -> isize {
        self.hwnd
    }

    pub(crate) fn is_current(&self) -> bool {
        unsafe { GetCurrentThreadId() == self.thread_id }
    }

    // Runs `f` on the shell thread and waits for its result. None when the thread is gone.
    pub fn call<T: Send + 'static>(&self, f: impl FnOnce() -> T + Send + 'static) -> Option<T> {
        if self.is_current() {
            return Some(f());
        }
        let (tx, rx) = mpsc::sync_channel(1);
        let job: Job = Box::new(move || {
            let _ = tx.send(f());
        });
        let raw = Box::into_raw(Box::new(job));
        let posted = unsafe { PostMessageW(Some(win::hwnd(self.hwnd)), WM_CALL, WPARAM(0), LPARAM(raw as isize)) };
        if posted.is_err() {
            // Not delivered: take the job back so it is freed.
            drop(unsafe { Box::from_raw(raw) });
            return None;
        }
        rx.recv().ok()
    }

    // Removes the tray icon, unregisters the shortcuts and ends the thread. Also done when the
    // process exits, by Windows; this is for tests and for a clean quit.
    pub fn stop(&self) {
        unsafe {
            let _ = PostMessageW(Some(win::hwnd(self.hwnd)), WM_STOP, WPARAM(0), LPARAM(0));
        }
        let handle = self.join.lock().unwrap_or_else(|poisoned| poisoned.into_inner()).take();
        if let Some(handle) = handle {
            if !self.is_current() {
                let _ = handle.join();
            }
        }
    }
}

// Sends an event to the handler, from the shell thread.
pub(crate) fn emit(event: Event) {
    let sink = SHARED.with(|cell| cell.borrow().as_ref().and_then(|shared| shared.sink.lock().ok().and_then(|sink| sink.clone())));
    if let Some(sink) = sink {
        sink(event);
    }
}

unsafe extern "system" fn proc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    match msg {
        WM_CALL => {
            let job = Box::from_raw(lparam.0 as *mut Job);
            job();
            LRESULT(0)
        }
        WM_HOTKEY => {
            emit(Event::Hotkey(wparam.0 as i32));
            LRESULT(0)
        }
        WM_TRAY => {
            crate::tray::handle_callback(hwnd, lparam.0 as u32);
            LRESULT(0)
        }
        WM_STOP => {
            crate::tray::remove_on_thread();
            crate::hotkeys::unregister_all_on_thread(hwnd);
            crate::instance::stop_listening_on_thread();
            let _ = DestroyWindow(hwnd);
            LRESULT(0)
        }
        WM_SETTINGCHANGE => {
            let area = if lparam.0 == 0 { String::new() } else { windows::core::PCWSTR(lparam.0 as *const u16).to_string().unwrap_or_default() };
            if area == "ImmersiveColorSet" {
                emit(Event::AppearanceChanged);
            }
            DefWindowProcW(hwnd, msg, wparam, lparam)
        }
        WM_DWMCOLORIZATIONCOLORCHANGED => {
            emit(Event::AppearanceChanged);
            DefWindowProcW(hwnd, msg, wparam, lparam)
        }
        WM_DESTROY => {
            PostQuitMessage(0);
            LRESULT(0)
        }
        _ if msg == crate::tray::taskbar_created() => {
            crate::tray::readd_on_thread();
            DefWindowProcW(hwnd, msg, wparam, lparam)
        }
        _ => DefWindowProcW(hwnd, msg, wparam, lparam),
    }
}
