// The tray icon of the background helper, as background.rs made it with Tauri's tray (tray-icon
// 0.25): tooltip "Qust", the app's icon, a left click (on release) opens the player, and a right
// click shows the menu Open Qust / Play / Stop / separator / Quit Qust. Icon id 1 and no GUID, like
// tray-icon's first icon, so Windows keeps the user's "show in the taskbar corner" choice for
// qust.exe. Recreated when Explorer restarts ("TaskbarCreated").

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrayCommand {
    Open,
    Toggle,
    Quit,
}

pub const TOOLTIP: &str = "Qust";

// (command id, text); None is a separator.
pub const MENU: [Option<(TrayCommand, &str)>; 4] = [Some((TrayCommand::Open, "Open Qust")), Some((TrayCommand::Toggle, "Play / Stop")), None, Some((TrayCommand::Quit, "Quit Qust"))];

impl TrayCommand {
    fn id(self) -> u32 {
        match self {
            TrayCommand::Open => 1,
            TrayCommand::Toggle => 2,
            TrayCommand::Quit => 3,
        }
    }

    fn from_id(id: u32) -> Option<TrayCommand> {
        [TrayCommand::Open, TrayCommand::Toggle, TrayCommand::Quit].into_iter().find(|command| command.id() == id)
    }
}

#[cfg(windows)]
mod imp {
    use super::*;
    use crate::native::{emit, Event, NativeThread, WM_TRAY};
    use crate::win;
    use std::cell::{Cell, RefCell};
    use windows::core::PCWSTR;
    use windows::Win32::Foundation::{HWND, LPARAM, POINT, WPARAM};
    use windows::Win32::UI::Shell::{Shell_NotifyIconW, NIF_ICON, NIF_MESSAGE, NIF_TIP, NIM_ADD, NIM_DELETE, NOTIFYICONDATAW};
    use windows::Win32::UI::WindowsAndMessaging::*;

    const ICON_ID: u32 = 1;

    struct TrayState {
        icon: HICON,
        owns_icon: bool,
    }

    thread_local! {
        static WINDOW: Cell<isize> = const { Cell::new(0) };
        static TRAY: RefCell<Option<TrayState>> = const { RefCell::new(None) };
    }

    pub(crate) fn thread_started(window: HWND) {
        WINDOW.with(|cell| cell.set(window.0 as isize));
    }

    pub(crate) fn taskbar_created() -> u32 {
        static MESSAGE: std::sync::OnceLock<u32> = std::sync::OnceLock::new();
        *MESSAGE.get_or_init(|| unsafe { RegisterWindowMessageW(windows::core::w!("TaskbarCreated")) })
    }

    // The tray icon on the shell thread.
    pub struct Tray {
        native: NativeThread,
    }

    impl Tray {
        pub fn new(native: &NativeThread) -> Self {
            Tray { native: native.clone() }
        }

        // Shows the icon (once; showing it again does nothing). False when the shell refused it
        // (Explorer not running yet: it is added when the taskbar appears).
        pub fn show(&self) -> bool {
            self.native.call(add_on_thread).unwrap_or(false)
        }

        pub fn hide(&self) {
            self.native.call(remove_on_thread);
        }

        pub fn is_shown(&self) -> bool {
            self.native.call(|| TRAY.with(|tray| tray.borrow().is_some())).unwrap_or(false)
        }
    }

    // The app's icon at the small-icon size: resource 1 of the executable (GPUI's window icon), or
    // the default application icon when it has none (tests).
    fn load_icon() -> (HICON, bool) {
        unsafe {
            let size = GetSystemMetrics(SM_CXSMICON);
            if let Ok(handle) = LoadImageW(Some(win::instance()), PCWSTR(1 as _), IMAGE_ICON, size, size, LR_DEFAULTCOLOR) {
                return (HICON(handle.0), true);
            }
            (LoadIconW(None, IDI_APPLICATION).unwrap_or_default(), false)
        }
    }

    fn data(window: HWND) -> NOTIFYICONDATAW {
        NOTIFYICONDATAW { cbSize: std::mem::size_of::<NOTIFYICONDATAW>() as u32, hWnd: window, uID: ICON_ID, ..Default::default() }
    }

    fn register(window: HWND, icon: HICON) -> bool {
        let mut nid = data(window);
        nid.uFlags = NIF_MESSAGE | NIF_ICON | NIF_TIP;
        nid.uCallbackMessage = WM_TRAY;
        nid.hIcon = icon;
        for (slot, unit) in nid.szTip.iter_mut().zip(TOOLTIP.encode_utf16()) {
            *slot = unit;
        }
        unsafe { Shell_NotifyIconW(NIM_ADD, &nid).as_bool() }
    }

    fn window() -> HWND {
        win::hwnd(WINDOW.with(|cell| cell.get()))
    }

    fn add_on_thread() -> bool {
        if TRAY.with(|tray| tray.borrow().is_some()) {
            return true;
        }
        let (icon, owns_icon) = load_icon();
        let added = register(window(), icon);
        // Kept even when refused: TaskbarCreated adds it once Explorer is up.
        TRAY.with(|tray| *tray.borrow_mut() = Some(TrayState { icon, owns_icon }));
        added
    }

    pub(crate) fn remove_on_thread() {
        let Some(state) = TRAY.with(|tray| tray.borrow_mut().take()) else { return };
        unsafe {
            let _ = Shell_NotifyIconW(NIM_DELETE, &data(window()));
            if state.owns_icon {
                let _ = DestroyIcon(state.icon);
            }
        }
    }

    pub(crate) fn readd_on_thread() {
        let icon = TRAY.with(|tray| tray.borrow().as_ref().map(|state| state.icon));
        if let Some(icon) = icon {
            register(window(), icon);
        }
    }

    pub(crate) fn handle_callback(window: HWND, event: u32) {
        match event {
            WM_LBUTTONUP => emit(Event::TrayClick),
            WM_RBUTTONUP => {
                if let Some(command) = show_menu(window) {
                    emit(Event::TrayMenu(command));
                }
            }
            _ => {}
        }
    }

    // tray-icon's show_tray_menu: the hidden window comes to the front so a click elsewhere closes
    // the menu, which opens above the cursor; WM_NULL afterwards, as the shell docs ask.
    fn show_menu(window: HWND) -> Option<TrayCommand> {
        unsafe {
            let menu = CreatePopupMenu().ok()?;
            for item in MENU {
                match item {
                    Some((command, text)) => {
                        let text = win::wide(text);
                        let _ = AppendMenuW(menu, MF_STRING, command.id() as usize, PCWSTR(text.as_ptr()));
                    }
                    None => {
                        let _ = AppendMenuW(menu, MF_SEPARATOR, 0, PCWSTR::null());
                    }
                }
            }
            win::allow_dark_mode_for_window(window, true);
            let mut cursor = POINT::default();
            let _ = GetCursorPos(&mut cursor);
            let _ = SetForegroundWindow(window);
            let chosen = TrackPopupMenu(menu, TPM_BOTTOMALIGN | TPM_LEFTALIGN | TPM_RETURNCMD, cursor.x, cursor.y, Some(0), window, None);
            let _ = PostMessageW(Some(window), WM_NULL, WPARAM(0), LPARAM(0));
            let _ = DestroyMenu(menu);
            TrayCommand::from_id(chosen.0 as u32)
        }
    }

    // For tests: what the shell knows about our icon (its rectangle), proving it is registered.
    pub fn icon_rect(native: &NativeThread) -> Option<(i32, i32, i32, i32)> {
        use windows::Win32::UI::Shell::{Shell_NotifyIconGetRect, NOTIFYICONIDENTIFIER};
        let window = native.hwnd();
        native
            .call(move || unsafe {
                let id = NOTIFYICONIDENTIFIER { cbSize: std::mem::size_of::<NOTIFYICONIDENTIFIER>() as u32, hWnd: win::hwnd(window), uID: ICON_ID, ..Default::default() };
                Shell_NotifyIconGetRect(&id).ok().map(|rect| (rect.left, rect.top, rect.right, rect.bottom))
            })
            .flatten()
    }
}

#[cfg(windows)]
pub use imp::{icon_rect, Tray};
#[cfg(windows)]
pub(crate) use imp::{handle_callback, readd_on_thread, remove_on_thread, taskbar_created, thread_started};

// macOS: an NSStatusItem would go here (see the report); until then there is no tray icon, and
// `show` says so.
#[cfg(not(windows))]
pub struct Tray;

#[cfg(not(windows))]
impl Tray {
    pub fn new(_native: &crate::NativeThread) -> Self {
        Tray
    }

    pub fn show(&self) -> bool {
        false
    }

    pub fn hide(&self) {}

    pub fn is_shown(&self) -> bool {
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn menu_is_the_old_one() {
        let texts: Vec<Option<&str>> = MENU.iter().map(|item| item.map(|(_, text)| text)).collect();
        assert_eq!(texts, [Some("Open Qust"), Some("Play / Stop"), None, Some("Quit Qust")]);
        for command in [TrayCommand::Open, TrayCommand::Toggle, TrayCommand::Quit] {
            assert_eq!(TrayCommand::from_id(command.id()), Some(command));
        }
        assert_eq!(TrayCommand::from_id(0), None);
    }
}
