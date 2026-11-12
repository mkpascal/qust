// The shell thread elsewhere than Windows. On macOS the tray (NSStatusItem) and hotkeys (Carbon)
// belong on the main thread, which GPUI runs, so there is no thread of our own: calls run where
// they are made, and events go to the handler as on Windows.

use std::sync::{Arc, Mutex};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Event {
    Hotkey(i32),
    TrayClick,
    TrayMenu(crate::tray::TrayCommand),
    Launched { args: Vec<String>, cwd: String },
    AppearanceChanged,
}

type Sink = Arc<dyn Fn(Event) + Send + Sync>;

#[derive(Clone)]
pub struct NativeThread {
    sink: Arc<Mutex<Option<Sink>>>,
}

impl NativeThread {
    pub fn start() -> std::io::Result<NativeThread> {
        Ok(NativeThread { sink: Arc::new(Mutex::new(None)) })
    }

    pub fn on_event(&self, handler: impl Fn(Event) + Send + Sync + 'static) {
        *self.sink.lock().unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(Arc::new(handler));
    }

    pub fn hwnd(&self) -> isize {
        0
    }

    pub fn call<T: Send + 'static>(&self, f: impl FnOnce() -> T + Send + 'static) -> Option<T> {
        Some(f())
    }

    #[allow(dead_code)]
    pub(crate) fn emit(&self, event: Event) {
        let sink = self.sink.lock().ok().and_then(|sink| sink.clone());
        if let Some(sink) = sink {
            sink(event);
        }
    }

    pub fn stop(&self) {}
}
