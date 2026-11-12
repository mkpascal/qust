// The only part that knows GPUI: window handles, getting from the shell thread (or a window
// procedure) into GPUI, and switching GPUI between windowed and headless for the tray-only mode.
//
// Background mode, measured (see the report): a GPUI app started windowed keeps its vsync thread
// running with no window open, about 60 wakeups a second; headless (WindowingRequest::Headless)
// has no vsync thread and no DirectX devices, and idles like a plain message loop. So the app
// starts headless when launched into the tray, goes windowed before opening the player
// (`windowed`), and back to headless once its last window has closed (`headless`).

use crate::Event;
use futures::channel::mpsc;
use futures::StreamExt;
use gpui::{App, GraphicalEnvironment, Task, Window, WindowingRequest};
use raw_window_handle::{HasWindowHandle, RawWindowHandle};

// The native handle of a GPUI window (HWND on Windows, NSView* on macOS), for `window`, `menu`,
// `drawer` and `picker`.
pub fn hwnd(window: &Window) -> Option<isize> {
    match HasWindowHandle::window_handle(window).ok()?.as_raw() {
        RawWindowHandle::Win32(handle) => Some(handle.hwnd.get()),
        RawWindowHandle::AppKit(handle) => Some(handle.ns_view.as_ptr() as isize),
        _ => None,
    }
}

type Job = Box<dyn FnOnce(&mut App) + Send>;

// Runs closures on GPUI's main thread, in the app's context, from any thread (the shell thread,
// the updater, a menu's `done`, a window procedure). Cheap to clone.
#[derive(Clone)]
pub struct MainSender {
    tx: mpsc::UnboundedSender<Job>,
}

impl MainSender {
    // Starts the task that runs what is posted. Call it once, in `Application::run`.
    pub fn new(cx: &mut App) -> MainSender {
        let (tx, mut rx) = mpsc::unbounded::<Job>();
        cx.spawn(async move |cx| {
            while let Some(job) = rx.next().await {
                cx.update(job);
            }
        })
        .detach();
        MainSender { tx }
    }

    // Queued; runs soon on the main thread. False once the app is gone.
    pub fn post(&self, job: impl FnOnce(&mut App) + Send + 'static) -> bool {
        self.tx.unbounded_send(Box::new(job)).is_ok()
    }
}

// Sends every shell event (shortcuts, tray, second launches) to `handler` on the main thread.
pub fn forward_events(native: &crate::NativeThread, sender: MainSender, handler: impl Fn(Event, &mut App) + Send + Sync + 'static) {
    let handler = std::sync::Arc::new(handler);
    native.on_event(move |event| {
        let handler = handler.clone();
        sender.post(move |cx| handler(event, cx));
    });
}

// Windowed (DirectX and vsync running), before opening a window. Resolves at once when it
// already is.
pub fn windowed(cx: &mut App) -> Task<Result<(), String>> {
    let task = cx.request_windowing(WindowingRequest::Windowed(GraphicalEnvironment::detect()));
    cx.spawn(async move |_| match task.await {
        Ok(()) => Ok(()),
        Err(error) if error.to_string().contains("already windowed") => Ok(()),
        Err(error) => Err(error.to_string()),
    })
}

// Headless (no DirectX, no vsync thread): once no window is open. Resolves at once when it
// already is.
pub fn headless(cx: &mut App) -> Task<Result<(), String>> {
    let task = cx.request_windowing(WindowingRequest::Headless);
    cx.spawn(async move |_| match task.await {
        Ok(()) => Ok(()),
        Err(error) if error.to_string().contains("already headless") => Ok(()),
        Err(error) => Err(error.to_string()),
    })
}
