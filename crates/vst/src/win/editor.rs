// A plugin's own window: a plain top-level window on the plugin thread with the plugin's view in
// it, titled with the plugin's name, centered over the window that opened it (Settings) and kept
// on its monitor, its title bar in the app's theme. It follows the size the plugin asks for, and
// the plugin follows the window when it can be resized. The view is told the monitor's scale.
//
// The window procedure only touches the editors (never the host's state), so it may run inside
// any call into a plugin. Closing it tells the host afterwards (WM_EDITOR_CLOSED), which saves
// the plugin's settings.

use super::objects::Frame;
use std::cell::RefCell;
use std::rc::Rc;
use std::sync::atomic::Ordering;
use vst3::Steinberg::Vst::{IEditController, IEditControllerTrait};
use vst3::Steinberg::*;
use vst3::{ComPtr, ComWrapper};
use windows::core::PCWSTR;
use windows::core::BOOL;
use windows::Win32::Foundation::{HINSTANCE, HWND, LPARAM, LRESULT, RECT, WPARAM};
use windows::Win32::Graphics::Dwm::{DwmSetWindowAttribute, DWMWA_USE_IMMERSIVE_DARK_MODE};
use windows::Win32::Graphics::Gdi::{GetMonitorInfoW, MonitorFromWindow, MONITORINFO, MONITOR_DEFAULTTONEAREST};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::HiDpi::{AdjustWindowRectExForDpi, GetDpiForWindow};
use windows::Win32::UI::WindowsAndMessaging::*;

pub const NO_WINDOW: &str = "This plugin has no window of its own.";
/// To the host's window, when an editor closed: wParam is the plugin's id.
pub const WM_EDITOR_CLOSED: u32 = WM_APP + 2;

const CLASS: &str = "QustPluginWindow";

pub struct Editor {
    pub plugin: u64,
    pub hwnd: isize,
    view: ComPtr<IPlugView>,
    frame: ComWrapper<Frame>,
    scale: Option<ComPtr<IPlugViewContentScaleSupport>>,
    resizable: bool,
    notify: isize,
}

thread_local! {
    static EDITORS: RefCell<Vec<Rc<Editor>>> = const { RefCell::new(Vec::new()) };
}

fn hwnd(raw: isize) -> HWND {
    HWND(raw as *mut _)
}

fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(Some(0)).collect()
}

fn instance() -> HINSTANCE {
    unsafe { GetModuleHandleW(None).map(|module| HINSTANCE(module.0)).unwrap_or_default() }
}

fn by_window(window: isize) -> Option<Rc<Editor>> {
    EDITORS.with(|editors| editors.borrow().iter().find(|editor| editor.hwnd == window).cloned())
}

fn by_plugin(plugin: u64) -> Option<Rc<Editor>> {
    EDITORS.with(|editors| editors.borrow().iter().find(|editor| editor.plugin == plugin).cloned())
}

pub fn is_open(plugin: u64) -> bool {
    by_plugin(plugin).is_some()
}

fn register() {
    thread_local! {
        static REGISTERED: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    }
    if REGISTERED.with(|registered| registered.replace(true)) {
        return;
    }
    let class = wide(CLASS);
    unsafe {
        let hinstance = instance();
        // Icon 1: the executable's (as GPUI's windows and the tray use).
        let icon = LoadIconW(Some(hinstance), PCWSTR(std::ptr::without_provenance::<u16>(1))).unwrap_or_default();
        let wnd_class = WNDCLASSEXW {
            cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
            style: CS_HREDRAW | CS_VREDRAW,
            lpfnWndProc: Some(proc),
            hInstance: hinstance,
            hIcon: icon,
            hIconSm: icon,
            hCursor: LoadCursorW(None, IDC_ARROW).unwrap_or_default(),
            lpszClassName: PCWSTR(class.as_ptr()),
            ..Default::default()
        };
        RegisterClassExW(&wnd_class);
    }
}

/// The client area to `width` × `height` physical pixels, keeping the window where it is.
pub fn set_client_size(window: isize, width: i32, height: i32) {
    unsafe {
        let handle = hwnd(window);
        let style = WINDOW_STYLE(GetWindowLongW(handle, GWL_STYLE) as u32);
        let ex_style = WINDOW_EX_STYLE(GetWindowLongW(handle, GWL_EXSTYLE) as u32);
        let mut rect = RECT { left: 0, top: 0, right: width.max(1), bottom: height.max(1) };
        let _ = AdjustWindowRectExForDpi(&mut rect, style, false, ex_style, GetDpiForWindow(handle));
        let _ = SetWindowPos(handle, None, 0, 0, rect.right - rect.left, rect.bottom - rect.top, SWP_NOMOVE | SWP_NOZORDER | SWP_NOACTIVATE);
    }
}

fn scale_of(window: isize) -> f32 {
    (unsafe { GetDpiForWindow(hwnd(window)) }.max(96)) as f32 / 96.0
}

pub fn set_dark_title(window: isize, dark: bool) {
    unsafe {
        let value: BOOL = dark.into();
        let _ = DwmSetWindowAttribute(hwnd(window), DWMWA_USE_IMMERSIVE_DARK_MODE, &value as *const _ as *const _, std::mem::size_of::<BOOL>() as u32);
    }
}

/// Every open editor's title bar in the new theme.
pub fn set_dark(dark: bool) {
    let windows: Vec<isize> = EDITORS.with(|editors| editors.borrow().iter().map(|editor| editor.hwnd).collect());
    for window in windows {
        set_dark_title(window, dark);
        unsafe {
            let _ = SetWindowPos(hwnd(window), None, 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE | SWP_FRAMECHANGED);
        }
    }
}

fn front(window: isize) {
    unsafe {
        let handle = hwnd(window);
        if IsIconic(handle).as_bool() {
            let _ = ShowWindow(handle, SW_RESTORE);
        }
        let _ = SetForegroundWindow(handle);
    }
}

/// Opens the plugin's window (or brings it to the front).
pub fn open(plugin: u64, title: &str, controller: &ComPtr<IEditController>, near: Option<isize>, dark: bool, notify: isize) -> Result<(), String> {
    if let Some(editor) = by_plugin(plugin) {
        front(editor.hwnd);
        return Ok(());
    }
    register();
    unsafe {
        let view = ComPtr::from_raw(controller.createView(c"editor".as_ptr())).ok_or(NO_WINDOW)?;
        if view.isPlatformTypeSupported(kPlatformTypeHWND) != kResultOk {
            return Err(NO_WINDOW.into());
        }
        let resizable = view.canResize() == kResultOk;
        let mut style = WS_OVERLAPPED | WS_CAPTION | WS_SYSMENU | WS_MINIMIZEBOX | WS_CLIPCHILDREN;
        if resizable {
            style |= WS_THICKFRAME | WS_MAXIMIZEBOX;
        }
        // Created where it will show, so it gets that monitor's scale from the start.
        let mut around = RECT::default();
        let placed = near.is_some_and(|near| GetWindowRect(hwnd(near), &mut around).is_ok());
        let (x, y) = if placed { (around.left, around.top) } else { (CW_USEDEFAULT, CW_USEDEFAULT) };
        let class = wide(CLASS);
        let caption = wide(title);
        let window = CreateWindowExW(WINDOW_EX_STYLE::default(), PCWSTR(class.as_ptr()), PCWSTR(caption.as_ptr()), style, x, y, 400, 300, None, None, Some(instance()), None)
            .map_err(|_| NO_WINDOW.to_string())?;
        let window = window.0 as isize;
        set_dark_title(window, dark);
        let frame = ComWrapper::new(Frame { hwnd: window.into(), resizing: false.into() });
        if let Some(frame) = frame.as_com_ref::<IPlugFrame>() {
            view.setFrame(frame.as_ptr());
        }
        let scale = view.cast::<IPlugViewContentScaleSupport>();
        if let Some(scale) = &scale {
            scale.setContentScaleFactor(scale_of(window));
        }
        let editor = Rc::new(Editor { plugin, hwnd: window, view: Clone::clone(&view), frame, scale, resizable, notify });
        EDITORS.with(|editors| editors.borrow_mut().push(editor.clone()));
        let follow = |editor: &Editor| {
            let mut rect: ViewRect = std::mem::zeroed();
            if editor.view.getSize(&mut rect) == kResultOk {
                editor.frame.resizing.store(true, Ordering::Release);
                set_client_size(window, rect.right - rect.left, rect.bottom - rect.top);
                editor.frame.resizing.store(false, Ordering::Release);
            }
        };
        follow(&editor);
        if view.attached(window as *mut std::ffi::c_void, kPlatformTypeHWND) != kResultOk {
            EDITORS.with(|editors| editors.borrow_mut().retain(|e| e.hwnd != window));
            view.setFrame(std::ptr::null_mut());
            let _ = DestroyWindow(hwnd(window));
            return Err(NO_WINDOW.into());
        }
        // Some views know their size only once attached.
        follow(&editor);
        if placed {
            center(window, near.unwrap_or_default());
        }
        let _ = ShowWindow(hwnd(window), SW_SHOW);
        let _ = SetForegroundWindow(hwnd(window));
        Ok(())
    }
}

// Centered over `near`, inside its monitor's work area.
unsafe fn center(window: isize, near: isize) {
    let (mut own, mut around) = (RECT::default(), RECT::default());
    if GetWindowRect(hwnd(window), &mut own).is_err() || GetWindowRect(hwnd(near), &mut around).is_err() {
        return;
    }
    let (width, height) = (own.right - own.left, own.bottom - own.top);
    let mut x = around.left + (around.right - around.left - width) / 2;
    let mut y = around.top + (around.bottom - around.top - height) / 2;
    let mut monitor = MONITORINFO { cbSize: std::mem::size_of::<MONITORINFO>() as u32, ..Default::default() };
    if GetMonitorInfoW(MonitorFromWindow(hwnd(near), MONITOR_DEFAULTTONEAREST), &mut monitor).as_bool() {
        let work = monitor.rcWork;
        x = x.min(work.right - width).max(work.left);
        y = y.min(work.bottom - height).max(work.top);
    }
    let _ = SetWindowPos(hwnd(window), None, x, y, 0, 0, SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE);
}

// The view leaves, then the window.
unsafe fn close_window(window: isize) {
    let editor = EDITORS.with(|editors| {
        let mut editors = editors.borrow_mut();
        let index = editors.iter().position(|editor| editor.hwnd == window)?;
        Some(editors.remove(index))
    });
    if let Some(editor) = editor {
        editor.view.removed();
        editor.view.setFrame(std::ptr::null_mut());
        editor.frame.hwnd.store(0, Ordering::Release);
        let (plugin, notify) = (editor.plugin, editor.notify);
        drop(editor);
        let _ = PostMessageW(Some(hwnd(notify)), WM_EDITOR_CLOSED, WPARAM(plugin as usize), LPARAM(0));
    }
    let _ = DestroyWindow(hwnd(window));
}

/// Closes the plugin's window, if open (before the plugin goes).
pub fn close(plugin: u64) {
    if let Some(editor) = by_plugin(plugin) {
        let window = editor.hwnd;
        drop(editor);
        unsafe { close_window(window) };
    }
}

pub fn close_all() {
    let windows: Vec<isize> = EDITORS.with(|editors| editors.borrow().iter().map(|editor| editor.hwnd).collect());
    for window in windows {
        unsafe { close_window(window) };
    }
}

unsafe extern "system" fn proc(window: HWND, message: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    let raw = window.0 as isize;
    match message {
        WM_CLOSE => {
            close_window(raw);
            return LRESULT(0);
        }
        WM_ERASEBKGND => return LRESULT(1),
        WM_SIZE if wparam.0 as u32 != SIZE_MINIMIZED => {
            if let Some(editor) = by_window(raw) {
                if editor.resizable && !editor.frame.resizing.load(Ordering::Acquire) {
                    let mut client = RECT::default();
                    if GetClientRect(window, &mut client).is_ok() {
                        let mut rect = ViewRect { left: 0, top: 0, right: client.right, bottom: client.bottom };
                        editor.view.checkSizeConstraint(&mut rect);
                        editor.view.onSize(&mut rect);
                    }
                }
            }
        }
        WM_DPICHANGED => {
            if let Some(editor) = by_window(raw) {
                let suggested = *(lparam.0 as *const RECT);
                let _ = SetWindowPos(window, None, suggested.left, suggested.top, suggested.right - suggested.left, suggested.bottom - suggested.top, SWP_NOZORDER | SWP_NOACTIVATE);
                if let Some(scale) = &editor.scale {
                    scale.setContentScaleFactor((wparam.0 & 0xffff) as f32 / 96.0);
                }
                let mut rect: ViewRect = std::mem::zeroed();
                if editor.view.getSize(&mut rect) == kResultOk {
                    editor.frame.resizing.store(true, Ordering::Release);
                    set_client_size(raw, rect.right - rect.left, rect.bottom - rect.top);
                    editor.frame.resizing.store(false, Ordering::Release);
                    editor.view.onSize(&mut rect);
                }
                return LRESULT(0);
            }
        }
        _ => {}
    }
    DefWindowProcW(window, message, wparam, lparam)
}
