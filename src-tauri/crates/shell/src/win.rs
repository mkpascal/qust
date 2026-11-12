// Small Win32 helpers shared by the modules: wide strings, hidden helper windows, and the
// undocumented uxtheme calls that let menus follow the app's theme.

use windows::core::{PCSTR, PCWSTR};
use windows::Win32::Foundation::{HINSTANCE, HMODULE, HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::System::LibraryLoader::{GetModuleHandleW, GetProcAddress, LoadLibraryW};
use windows::Win32::UI::WindowsAndMessaging::*;

pub(crate) fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(Some(0)).collect()
}

pub(crate) fn hwnd(raw: isize) -> HWND {
    HWND(raw as *mut _)
}

pub(crate) fn instance() -> HINSTANCE {
    unsafe { GetModuleHandleW(None).map(|module| HINSTANCE(module.0)).unwrap_or_default() }
}

pub(crate) type WndProc = unsafe extern "system" fn(HWND, u32, WPARAM, LPARAM) -> LRESULT;

unsafe extern "system" fn default_proc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    DefWindowProcW(hwnd, msg, wparam, lparam)
}

// A window that is never shown: a target for messages (tray callbacks, WM_HOTKEY, WM_COPYDATA).
// Not message-only (HWND_MESSAGE): those don't get broadcasts such as "TaskbarCreated". The styles
// are tray-icon's and global-hotkey's: WS_EX_TOOLWINDOW keeps it off the taskbar even after
// Explorer restarts.
pub(crate) struct HiddenWindow {
    pub hwnd: HWND,
}

impl HiddenWindow {
    pub fn new(class: &str, title: Option<&str>) -> windows::core::Result<Self> {
        Self::with_proc(class, title, default_proc)
    }

    pub fn with_proc(class: &str, title: Option<&str>, proc: WndProc) -> windows::core::Result<Self> {
        let class_name = wide(class);
        let title = title.map(wide);
        unsafe {
            let wnd_class = WNDCLASSW {
                lpfnWndProc: Some(proc),
                hInstance: instance(),
                lpszClassName: PCWSTR(class_name.as_ptr()),
                ..Default::default()
            };
            // Fails harmlessly when the class exists (a second window of the same kind).
            RegisterClassW(&wnd_class);
            let hwnd = CreateWindowExW(
                WS_EX_NOACTIVATE | WS_EX_TRANSPARENT | WS_EX_LAYERED | WS_EX_TOOLWINDOW,
                PCWSTR(class_name.as_ptr()),
                title.as_ref().map_or(PCWSTR::null(), |title| PCWSTR(title.as_ptr())),
                WS_OVERLAPPED,
                CW_USEDEFAULT,
                0,
                CW_USEDEFAULT,
                0,
                None,
                None,
                Some(instance()),
                None,
            )?;
            Ok(HiddenWindow { hwnd })
        }
    }
}

impl Drop for HiddenWindow {
    fn drop(&mut self) {
        unsafe {
            let _ = DestroyWindow(self.hwnd);
        }
    }
}

// --- Dark menus -------------------------------------------------------------------------------
//
// Win32 menus only turn dark through uxtheme's unnamed exports, which Explorer, Notepad and
// Tauri's tao use too (ordinal 135 SetPreferredAppMode, 133 AllowDarkModeForWindow, 136
// FlushMenuThemes, 104 RefreshImmersiveColorPolicyState; Windows 10 1903 and later).

#[repr(C)]
#[allow(dead_code)]
#[derive(Clone, Copy)]
pub(crate) enum PreferredAppMode {
    Default = 0,
    AllowDark = 1,
    ForceDark = 2,
    ForceLight = 3,
}

fn uxtheme() -> Option<HMODULE> {
    static MODULE: std::sync::OnceLock<isize> = std::sync::OnceLock::new();
    let module = *MODULE.get_or_init(|| unsafe { LoadLibraryW(windows::core::w!("uxtheme.dll")).map(|module| module.0 as isize).unwrap_or(0) });
    (module != 0).then(|| HMODULE(module as *mut _))
}

fn ordinal(number: u16) -> Option<unsafe extern "system" fn() -> isize> {
    unsafe { GetProcAddress(uxtheme()?, PCSTR(number as usize as *const u8)) }
}

// Whether this Windows has the calls (it has since Windows 10 1903).
pub(crate) fn dark_menus_supported() -> bool {
    ordinal(135).is_some() && ordinal(133).is_some() && ordinal(136).is_some()
}

pub(crate) fn set_preferred_app_mode(mode: PreferredAppMode) -> bool {
    let (Some(set), Some(flush)) = (ordinal(135), ordinal(136)) else {
        return false;
    };
    unsafe {
        let set: unsafe extern "system" fn(PreferredAppMode) -> PreferredAppMode = std::mem::transmute(set);
        let flush: unsafe extern "system" fn() = std::mem::transmute(flush);
        set(mode);
        if let Some(refresh) = ordinal(104) {
            let refresh: unsafe extern "system" fn() = std::mem::transmute(refresh);
            refresh();
        }
        flush();
    }
    true
}

pub(crate) fn allow_dark_mode_for_window(window: HWND, allow: bool) {
    if let Some(allow_for_window) = ordinal(133) {
        unsafe {
            // C++ bool in and out, as tao declares it.
            let allow_for_window: unsafe extern "system" fn(HWND, bool) -> bool = std::mem::transmute(allow_for_window);
            allow_for_window(window, allow);
        }
    }
}
