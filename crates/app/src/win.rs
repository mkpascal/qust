// The few window calls the shell doesn't have, for the hidden and off-screen modes (tests and
// measurements) and for showing the player at the system's default position like tao did.

#[cfg(windows)]
mod imp {
    use windows::Win32::Foundation::HWND;
    use windows::Win32::UI::WindowsAndMessaging::*;

    fn h(window: isize) -> HWND {
        HWND(window as *mut _)
    }

    // Off every screen, shown without activation, no taskbar button (a tool window renders like
    // any other).
    pub fn show_offscreen(window: isize, x: i32, y: i32) {
        unsafe {
            let style = GetWindowLongPtrW(h(window), GWL_EXSTYLE);
            SetWindowLongPtrW(h(window), GWL_EXSTYLE, (style & !(WS_EX_APPWINDOW.0 as isize)) | WS_EX_TOOLWINDOW.0 as isize);
            let _ = SetWindowPos(h(window), Some(HWND_BOTTOM), x, y, 0, 0, SWP_NOSIZE | SWP_NOACTIVATE | SWP_SHOWWINDOW | SWP_FRAMECHANGED);
        }
    }

    pub fn is_window(window: isize) -> bool {
        unsafe { IsWindow(Some(h(window))).as_bool() }
    }

    // When the pointer is no longer over `window` (a panel just opened over it, under a pointer
    // that hasn't moved), tells it the pointer left, as the browser did: Windows only sends
    // WM_MOUSELEAVE once the pointer moves, and until then the button under it stays hovered.
    pub fn leave_if_covered(window: isize) {
        use windows::Win32::Foundation::{LPARAM, POINT, WPARAM};
        unsafe {
            let mut cursor = POINT::default();
            if GetCursorPos(&mut cursor).is_err() {
                return;
            }
            let under = WindowFromPoint(cursor);
            if under.is_invalid() || GetAncestor(under, GA_ROOT) != h(window) {
                // WM_MOUSELEAVE (in the windows crate's Controls module).
                const WM_MOUSELEAVE: u32 = 0x02A3;
                let _ = PostMessageW(Some(h(window)), WM_MOUSELEAVE, WPARAM(0), LPARAM(0));
            }
        }
    }

    // Milliseconds since this process was created (start-up measurements).
    pub fn process_age_ms() -> f64 {
        use windows::Win32::Foundation::FILETIME;
        use windows::Win32::System::SystemInformation::GetSystemTimePreciseAsFileTime;
        use windows::Win32::System::Threading::{GetCurrentProcess, GetProcessTimes};
        let ticks = |t: FILETIME| ((t.dwHighDateTime as u64) << 32) | t.dwLowDateTime as u64;
        unsafe {
            let (mut created, mut exited, mut kernel, mut user) = (FILETIME::default(), FILETIME::default(), FILETIME::default(), FILETIME::default());
            if GetProcessTimes(GetCurrentProcess(), &mut created, &mut exited, &mut kernel, &mut user).is_err() {
                return 0.0;
            }
            let now = GetSystemTimePreciseAsFileTime();
            ticks(now).saturating_sub(ticks(created)) as f64 / 10_000.0
        }
    }
}

#[cfg(not(windows))]
mod imp {
    pub fn process_age_ms() -> f64 {
        0.0
    }

    pub fn show_offscreen(_window: isize, _x: i32, _y: i32) {}

    pub fn is_window(_window: isize) -> bool {
        true
    }

    pub fn leave_if_covered(_window: isize) {}
}

pub use imp::*;
