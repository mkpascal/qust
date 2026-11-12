// Window helpers GPUI lacks, on native handles (HWND as isize; get it with `gpui_glue::hwnd`):
//
// - Settings and Stream information are owned by the player: above it, no taskbar button,
//   minimized with it (`set_owner`), opened centered over it and kept on its monitor
//   (`centered_over`, the maths of main.rs's centered_over).
// - The title bar follows the app's theme, also when it's forced and after the system changes
//   (`follow_theme`: GPUI resets the title bar to the system's theme on WM_SETTINGCHANGE, so a
//   subclass re-applies ours after GPUI has handled it).
// - Esc closes the panels (`close_on_escape`), except while a shortcut is being recorded.
// - Recording a shortcut reads keys from the window's messages (`KeyCapture`), see `recorder`.
// - `bring_to_front`: GPUI's activate_window() always injects an Alt key press (SendInput); this
//   does what Tauri's set_focus did: SetForegroundWindow, and that trick only if it was refused.

// A rectangle in logical pixels: x, y, width, height.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Rect {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

// main.rs's centered_over: the logical position (outer top-left) that centers a window whose
// content is `size` over `owner` (its outer rectangle), kept inside the owner's monitor work area
// with room for the frame (16 px across, 40 down).
pub fn centered_over(owner: Rect, size: (f64, f64), work_area: Option<Rect>) -> (f64, f64) {
    let mut x = owner.x + (owner.width - size.0) / 2.0;
    let mut y = owner.y + (owner.height - size.1) / 2.0;
    if let Some(area) = work_area {
        x = x.clamp(area.x, (area.x + area.width - size.0 - 16.0).max(area.x));
        y = y.clamp(area.y, (area.y + area.height - size.1 - 40.0).max(area.y));
    }
    (x, y)
}

#[cfg(windows)]
mod imp {
    use super::*;
    use crate::recorder::{self, Recorded};
    use crate::win;
    use std::cell::RefCell;
    use std::collections::HashMap;
    use windows::core::BOOL;
    use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
    use windows::Win32::Graphics::Dwm::{DwmGetWindowAttribute, DwmSetWindowAttribute, DWMWA_USE_IMMERSIVE_DARK_MODE};
    use windows::Win32::Graphics::Gdi::{GetMonitorInfoW, MonitorFromWindow, MONITORINFO, MONITOR_DEFAULTTONEAREST};
    use windows::Win32::UI::HiDpi::GetDpiForWindow;
    use windows::Win32::UI::Input::KeyboardAndMouse::*;
    use windows::Win32::UI::Shell::{DefSubclassProc, RemoveWindowSubclass, SetWindowSubclass};
    use windows::Win32::UI::WindowsAndMessaging::*;

    pub fn scale(window: isize) -> f64 {
        let dpi = unsafe { GetDpiForWindow(win::hwnd(window)) };
        if dpi == 0 {
            1.0
        } else {
            dpi as f64 / 96.0
        }
    }

    // The window's outer rectangle, in logical pixels of its own scale.
    pub fn outer_rect(window: isize) -> Option<Rect> {
        let mut rect = RECT::default();
        unsafe { GetWindowRect(win::hwnd(window), &mut rect).ok()? };
        let scale = scale(window);
        Some(Rect { x: rect.left as f64 / scale, y: rect.top as f64 / scale, width: (rect.right - rect.left) as f64 / scale, height: (rect.bottom - rect.top) as f64 / scale })
    }

    // The work area of the window's monitor, in logical pixels of the window's scale.
    pub fn work_area(window: isize) -> Option<Rect> {
        unsafe {
            let monitor = MonitorFromWindow(win::hwnd(window), MONITOR_DEFAULTTONEAREST);
            let mut info = MONITORINFO { cbSize: std::mem::size_of::<MONITORINFO>() as u32, ..Default::default() };
            if !GetMonitorInfoW(monitor, &mut info).as_bool() {
                return None;
            }
            let scale = scale(window);
            let area = info.rcWork;
            Some(Rect { x: area.left as f64 / scale, y: area.top as f64 / scale, width: (area.right - area.left) as f64 / scale, height: (area.bottom - area.top) as f64 / scale })
        }
    }

    pub fn client_size(window: isize) -> Option<(i32, i32)> {
        let mut rect = RECT::default();
        unsafe { GetClientRect(win::hwnd(window), &mut rect).ok()? };
        Some((rect.right - rect.left, rect.bottom - rect.top))
    }

    // Where a panel with content `size` (logical) opens over `owner`: logical, for GPUI's window
    // bounds, and physical (the owner's scale) for `set_outer_position`.
    pub fn panel_position(owner: isize, size: (f64, f64)) -> Option<((f64, f64), (i32, i32))> {
        let logical = centered_over(outer_rect(owner)?, size, work_area(owner));
        let scale = scale(owner);
        Some((logical, ((logical.0 * scale).round() as i32, (logical.1 * scale).round() as i32)))
    }

    // Moves a window's outer top-left to physical screen coordinates.
    pub fn set_outer_position(window: isize, x: i32, y: i32) {
        unsafe {
            let _ = SetWindowPos(win::hwnd(window), None, x, y, 0, 0, SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE);
        }
    }

    // Makes `child` an owned window of `owner`: it stays above the owner, has no taskbar button
    // and is minimized with it (Tauri's `parent(owner)` did this; GPUI only owns modal dialogs).
    pub fn set_owner(child: isize, owner: isize) {
        unsafe {
            let window = win::hwnd(child);
            let visible = IsWindowVisible(window).as_bool();
            if visible {
                let _ = ShowWindow(window, SW_HIDE);
            }
            let style = GetWindowLongPtrW(window, GWL_EXSTYLE);
            SetWindowLongPtrW(window, GWL_EXSTYLE, style & !(WS_EX_APPWINDOW.0 as isize));
            SetWindowLongPtrW(window, GWLP_HWNDPARENT, owner);
            if visible {
                let _ = ShowWindow(window, SW_SHOWNA);
            }
        }
    }

    pub fn owner_of(window: isize) -> isize {
        unsafe { GetWindow(win::hwnd(window), GW_OWNER).map(|owner| owner.0 as isize).unwrap_or(0) }
    }

    pub fn has_taskbar_style(window: isize) -> bool {
        unsafe { GetWindowLongPtrW(win::hwnd(window), GWL_EXSTYLE) & WS_EX_APPWINDOW.0 as isize != 0 }
    }

    // The minimize box off (Tauri's `minimizable(false)` on the panels).
    pub fn set_minimizable(window: isize, minimizable: bool) {
        unsafe {
            let window = win::hwnd(window);
            let style = GetWindowLongPtrW(window, GWL_STYLE);
            let style = if minimizable { style | WS_MINIMIZEBOX.0 as isize } else { style & !(WS_MINIMIZEBOX.0 as isize) };
            SetWindowLongPtrW(window, GWL_STYLE, style);
            let _ = SetWindowPos(window, None, 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE | SWP_FRAMECHANGED);
        }
    }

    // A dark or light title bar, redrawn at once (tao's refresh_titlebar_theme_color).
    pub fn set_dark_title_bar(window: isize, dark: bool) {
        unsafe {
            let window = win::hwnd(window);
            let value: BOOL = dark.into();
            let _ = DwmSetWindowAttribute(window, DWMWA_USE_IMMERSIVE_DARK_MODE, &value as *const _ as *const _, std::mem::size_of::<BOOL>() as u32);
            if GetActiveWindow() == window {
                DefWindowProcW(window, WM_NCACTIVATE, WPARAM(0), LPARAM(0));
                DefWindowProcW(window, WM_NCACTIVATE, WPARAM(1), LPARAM(0));
            } else {
                DefWindowProcW(window, WM_NCACTIVATE, WPARAM(1), LPARAM(0));
                DefWindowProcW(window, WM_NCACTIVATE, WPARAM(0), LPARAM(0));
            }
        }
    }

    pub fn dark_title_bar(window: isize) -> bool {
        let mut value = BOOL(0);
        unsafe {
            let _ = DwmGetWindowAttribute(win::hwnd(window), DWMWA_USE_IMMERSIVE_DARK_MODE, &mut value as *mut _ as *mut _, std::mem::size_of::<BOOL>() as u32);
        }
        value.as_bool()
    }

    // Brings a window to the front: unminimized, shown, foreground.
    pub fn bring_to_front(window: isize) {
        unsafe {
            let handle = win::hwnd(window);
            if IsIconic(handle).as_bool() {
                let _ = ShowWindow(handle, SW_RESTORE);
            } else if !IsWindowVisible(handle).as_bool() {
                let _ = ShowWindow(handle, SW_SHOW);
            }
            if GetForegroundWindow() == handle {
                return;
            }
            if SetForegroundWindow(handle).as_bool() {
                return;
            }
            // tao's force_window_active: an Alt press makes this the last input's owner.
            let scan = MapVirtualKeyW(VK_MENU.0 as u32, MAPVK_VK_TO_VSC) as u16;
            let key = |flags: KEYBD_EVENT_FLAGS| INPUT { r#type: INPUT_KEYBOARD, Anonymous: INPUT_0 { ki: KEYBDINPUT { wVk: VK_LMENU, wScan: scan, dwFlags: flags, ..Default::default() } } };
            let inputs = [key(KEYEVENTF_EXTENDEDKEY), key(KEYEVENTF_EXTENDEDKEY | KEYEVENTF_KEYUP)];
            SendInput(&inputs, std::mem::size_of::<INPUT>() as i32);
            let _ = SetForegroundWindow(handle);
        }
    }

    // --- The subclass: theme, Esc, key capture ----------------------------------------------

    const SUBCLASS_ID: usize = 0x5155_5354; // "QUST"

    #[derive(Default)]
    struct Hooks {
        theme: bool,
        escape: bool,
        capture: Option<Box<dyn FnMut(Recorded)>>,
    }

    thread_local! {
        // Per window, on the thread that owns it (GPUI's main thread).
        static HOOKS: RefCell<HashMap<isize, Hooks>> = RefCell::new(HashMap::new());
    }

    fn hooked(window: isize, change: impl FnOnce(&mut Hooks)) {
        let install = HOOKS.with(|hooks| {
            let mut hooks = hooks.borrow_mut();
            let fresh = !hooks.contains_key(&window);
            change(hooks.entry(window).or_default());
            fresh
        });
        if install {
            unsafe {
                let _ = SetWindowSubclass(win::hwnd(window), Some(subclass_proc), SUBCLASS_ID, 0);
            }
        }
    }

    pub(crate) fn ensure_subclass(window: isize) {
        hooked(window, |_| {});
    }

    // Message for `post_to`: lParam is a Box<Job>.
    const WM_QUST_RUN: u32 = WM_APP + 0x51;
    type Job = Box<dyn FnOnce() + Send>;

    // Runs `job` on the window's thread from its message loop (not inside another event's
    // handling). The window must have the subclass (`ensure_subclass`, on its thread). False when
    // the window is gone.
    pub(crate) fn post_to(window: isize, job: impl FnOnce() + Send + 'static) -> bool {
        let raw = Box::into_raw(Box::new(Box::new(job) as Job));
        let posted = unsafe { PostMessageW(Some(win::hwnd(window)), WM_QUST_RUN, WPARAM(0), LPARAM(raw as isize)) };
        if posted.is_err() {
            drop(unsafe { Box::from_raw(raw) });
            return false;
        }
        true
    }

    // Keeps the window's title bar in the app's theme (`theme::set_app_theme`), now and after
    // every system change. Call it on the window's thread, once it exists.
    pub fn follow_theme(window: isize) {
        hooked(window, |hooks| hooks.theme = true);
        set_dark_title_bar(window, crate::theme::app_theme().is_dark());
    }

    pub(crate) fn retheme_all() {
        let windows: Vec<isize> = HOOKS.with(|hooks| hooks.borrow().iter().filter(|(_, hooks)| hooks.theme).map(|(window, _)| *window).collect());
        let dark = crate::theme::app_theme().is_dark();
        for window in windows {
            set_dark_title_bar(window, dark);
        }
    }

    // Esc closes the window (WM_CLOSE, so GPUI closes it as usual), except while recording.
    pub fn close_on_escape(window: isize) {
        hooked(window, |hooks| hooks.escape = true);
    }

    // While it lives, every key down in the window is swallowed and turned into a `Recorded`; the
    // first that isn't `Ignore` (or the window losing the focus: `Cancel`) is handed to `on_key`
    // and ends the capture. One at a time per window; the
    // callback runs inside the window procedure, so it must only hand the result on (post it to
    // the UI), not call into GPUI. Pause the global shortcuts first, or they never get here.
    pub struct KeyCapture {
        window: isize,
    }

    impl KeyCapture {
        pub fn start(window: isize, on_key: impl FnMut(Recorded) + 'static) -> KeyCapture {
            hooked(window, |hooks| hooks.capture = Some(Box::new(on_key)));
            KeyCapture { window }
        }

        pub fn is_active(&self) -> bool {
            HOOKS.with(|hooks| hooks.borrow().get(&self.window).is_some_and(|hooks| hooks.capture.is_some()))
        }
    }

    impl Drop for KeyCapture {
        fn drop(&mut self) {
            HOOKS.with(|hooks| {
                if let Some(hooks) = hooks.borrow_mut().get_mut(&self.window) {
                    hooks.capture = None;
                }
            });
        }
    }

    // The `KeyboardEvent.code` and flags of a WM_KEYDOWN / WM_SYSKEYDOWN.
    pub fn key_message(wparam: usize, lparam: isize) -> (String, bool, bool, bool, bool) {
        let scan = ((lparam >> 16) & 0xFF) as u32;
        let extended = (lparam >> 24) & 1 == 1;
        let vk = wparam as u16;
        let code = recorder::code_from_scan_code(scan, extended).or_else(|| if scan == 0 { recorder::code_from_media_vk(vk) } else { None }).unwrap_or("");
        let down = |key: VIRTUAL_KEY| unsafe { GetKeyState(key.0 as i32) } < 0;
        (code.to_string(), down(VK_CONTROL), down(VK_MENU), down(VK_SHIFT), down(VK_LWIN) || down(VK_RWIN))
    }

    fn take_capture(window: isize) -> Option<Box<dyn FnMut(Recorded)>> {
        HOOKS.with(|hooks| hooks.borrow_mut().get_mut(&window).and_then(|hooks| hooks.capture.take()))
    }

    unsafe extern "system" fn subclass_proc(window: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM, _id: usize, _data: usize) -> LRESULT {
        let key = window.0 as isize;
        let (theme, escape, capturing) = HOOKS.with(|hooks| hooks.borrow().get(&key).map_or((false, false, false), |hooks| (hooks.theme, hooks.escape, hooks.capture.is_some())));
        match msg {
            WM_QUST_RUN => {
                let job = Box::from_raw(lparam.0 as *mut Job);
                job();
                return LRESULT(0);
            }
            WM_KEYDOWN | WM_SYSKEYDOWN if capturing => {
                let (code, ctrl, alt, shift, meta) = key_message(wparam.0, lparam.0);
                let recorded = recorder::record(&code, ctrl, alt, shift, meta);
                // Ignored keys are swallowed and change nothing. Otherwise the capture ends; it is
                // taken out while it runs, so a callback that starts a new one can't conflict.
                if recorded != Recorded::Ignore {
                    if let Some(mut capture) = take_capture(key) {
                        capture(recorded);
                    }
                }
                return LRESULT(0);
            }
            WM_CHAR | WM_SYSCHAR | WM_DEADCHAR | WM_SYSDEADCHAR if capturing => return LRESULT(0),
            WM_KILLFOCUS if capturing => {
                if let Some(mut capture) = take_capture(key) {
                    capture(Recorded::Cancel);
                }
            }
            WM_ACTIVATE if capturing && (wparam.0 & 0xFFFF) as u32 == WA_INACTIVE => {
                if let Some(mut capture) = take_capture(key) {
                    capture(Recorded::Cancel);
                }
            }
            WM_KEYDOWN if escape && wparam.0 as u16 == VK_ESCAPE.0 => {
                let _ = PostMessageW(Some(window), WM_CLOSE, WPARAM(0), LPARAM(0));
                return LRESULT(0);
            }
            WM_SETTINGCHANGE | WM_THEMECHANGED if theme => {
                // GPUI first (it puts the system's theme on the title bar), then ours.
                let result = DefSubclassProc(window, msg, wparam, lparam);
                set_dark_title_bar(key, crate::theme::app_theme().is_dark());
                return result;
            }
            WM_NCDESTROY => {
                HOOKS.with(|hooks| hooks.borrow_mut().remove(&key));
                let _ = RemoveWindowSubclass(window, Some(subclass_proc), SUBCLASS_ID);
            }
            _ => {}
        }
        DefSubclassProc(window, msg, wparam, lparam)
    }

    // The cursor position, physical screen pixels.
    pub fn cursor_position() -> (i32, i32) {
        let mut point = POINT::default();
        unsafe {
            let _ = GetCursorPos(&mut point);
        }
        (point.x, point.y)
    }
}

#[cfg(windows)]
pub use imp::*;

// Elsewhere: honest no-ops (macOS: the app uses addChildWindow and NSAppearance; see the report).
#[cfg(not(windows))]
mod imp {
    use super::*;
    use crate::recorder::Recorded;

    pub fn scale(_window: isize) -> f64 {
        1.0
    }
    pub fn outer_rect(_window: isize) -> Option<Rect> {
        None
    }
    pub fn work_area(_window: isize) -> Option<Rect> {
        None
    }
    pub fn client_size(_window: isize) -> Option<(i32, i32)> {
        None
    }
    pub fn panel_position(_owner: isize, _size: (f64, f64)) -> Option<((f64, f64), (i32, i32))> {
        None
    }
    pub fn set_outer_position(_window: isize, _x: i32, _y: i32) {}
    pub fn set_owner(_child: isize, _owner: isize) {}
    pub fn owner_of(_window: isize) -> isize {
        0
    }
    pub fn has_taskbar_style(_window: isize) -> bool {
        false
    }
    pub fn set_minimizable(_window: isize, _minimizable: bool) {}
    pub fn set_dark_title_bar(_window: isize, _dark: bool) {}
    pub fn dark_title_bar(_window: isize) -> bool {
        false
    }
    pub fn bring_to_front(_window: isize) {}
    pub fn follow_theme(_window: isize) {}
    pub(crate) fn retheme_all() {}
    pub fn close_on_escape(_window: isize) {}
    pub fn cursor_position() -> (i32, i32) {
        (0, 0)
    }

    pub struct KeyCapture;

    impl KeyCapture {
        pub fn start(_window: isize, _on_key: impl FnMut(Recorded) + 'static) -> KeyCapture {
            KeyCapture
        }
        pub fn is_active(&self) -> bool {
            false
        }
    }
}

#[cfg(not(windows))]
pub use imp::*;

#[cfg(test)]
mod tests {
    use super::*;

    const AREA: Rect = Rect { x: 0.0, y: 0.0, width: 1920.0, height: 1032.0 };

    #[test]
    fn centered_over_the_player() {
        // The player (860×267 content, ~876×306 outer) in the middle of the screen.
        let owner = Rect { x: 500.0, y: 300.0, width: 876.0, height: 306.0 };
        assert_eq!(centered_over(owner, (580.0, 590.0), Some(AREA)), (648.0, 158.0));
        assert_eq!(centered_over(owner, (580.0, 590.0), None), (648.0, 158.0));
    }

    #[test]
    fn kept_on_the_monitor() {
        // Near the top: the panel would start above the work area.
        let owner = Rect { x: 500.0, y: 0.0, width: 876.0, height: 306.0 };
        assert_eq!(centered_over(owner, (520.0, 850.0), Some(AREA)).1, 0.0);
        // Near the bottom right: kept 16 px from the right edge and 40 from the bottom.
        let owner = Rect { x: 1600.0, y: 900.0, width: 876.0, height: 306.0 };
        let (x, y) = centered_over(owner, (580.0, 590.0), Some(AREA));
        assert_eq!((x, y), (1920.0 - 580.0 - 16.0, 1032.0 - 590.0 - 40.0));
        // A second monitor to the left (negative coordinates).
        let left = Rect { x: -1280.0, y: 0.0, width: 1280.0, height: 984.0 };
        let owner = Rect { x: -1300.0, y: 100.0, width: 876.0, height: 306.0 };
        assert_eq!(centered_over(owner, (580.0, 590.0), Some(left)).0, -1152.0);
        let owner = Rect { x: -1500.0, y: 100.0, width: 876.0, height: 306.0 };
        assert_eq!(centered_over(owner, (580.0, 590.0), Some(left)).0, -1280.0);
        // Taller than the work area: pinned to its top.
        let small = Rect { x: 0.0, y: 0.0, width: 1280.0, height: 720.0 };
        assert_eq!(centered_over(owner, (520.0, 850.0), Some(small)).1, 0.0);
    }
}
