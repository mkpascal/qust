// Native menus (right-click, the Settings choices) are built by the pages with Tauri's menu API,
// but shown through `popup_menu` here instead of `menu.popup()`.
//
// A menu blocks the main thread until it closes. Tauri shows it from inside an event-loop
// callback, and while a callback runs the event loop buffers everything else it receives. The
// clock and the analyzer reach the page through that loop, so they froze behind an open menu
// (the page drew them by itself before the audio moved to Rust). Shown from a timer callback, the
// menu's own message loop runs outside any event-loop callback, and the loop keeps working.

use tauri::menu::{ContextMenu, Menu};
use tauri::{AppHandle, LogicalPosition, Manager, ResourceId, Webview, Window, Wry};

type Task = Box<dyn FnOnce() + Send>;

// `rid` is the menu's resource id (`menu.rid` in JS). With no position, the menu opens at the cursor.
#[tauri::command(async)]
pub fn popup_menu(app: AppHandle, webview: Webview, window: Window, rid: ResourceId, x: Option<f64>, y: Option<f64>) -> Result<(), String> {
    let menu = webview.resources_table().get::<Menu<Wry>>(rid).map_err(|error| error.to_string())?;
    show(
        &app,
        Box::new(move || {
            let _ = match (x, y) {
                (Some(x), Some(y)) => menu.popup_at(window, LogicalPosition::new(x, y)),
                _ => menu.popup(window),
            };
        }),
    );
    Ok(())
}

#[cfg(windows)]
fn show(app: &AppHandle, task: Task) {
    use std::sync::Mutex;
    use windows::Win32::Foundation::HWND;
    use windows::Win32::UI::WindowsAndMessaging::{KillTimer, SetTimer};

    static TASKS: Mutex<Vec<Task>> = Mutex::new(Vec::new());

    // Called by the main thread's message loop itself, between two events.
    unsafe extern "system" fn run(_: HWND, _: u32, timer: usize, _: u32) {
        let _ = KillTimer(None, timer);
        let tasks = std::mem::take(&mut *TASKS.lock().unwrap());
        for task in tasks {
            task();
        }
    }

    TASKS.lock().unwrap().push(task);
    // A timer belongs to the thread that sets it.
    let _ = app.run_on_main_thread(|| unsafe {
        SetTimer(None, 0, 0, Some(run));
    });
}

// Elsewhere the menu is shown the usual way (it returns once the menu has closed).
#[cfg(not(windows))]
fn show(_: &AppHandle, task: Task) {
    task();
}
