//! Tooltips (`title` attributes): the native Windows tooltip, in its own popup window, so it is
//! never clipped by our 860 × 267 window.
//!
//! WebView2 showed `title` tooltips with Chromium's `TooltipWin`: the system tooltip control
//! (`tooltips_class32`, common controls 6, visual styles; GPUI's manifest asks for them), shown
//! by tracking (`TTF_TRACK | TTF_ABSOLUTE`) at the mouse position offset down by the visible
//! height of the cursor, kept inside the monitor's work area, `TTS_NOPREFIX`, and not themed
//! dark (WebView2's tooltips stay light in dark mode). This module drives the same control the
//! same way, so its look is the system's own by construction: font, colors, border, shadow and
//! rounded corners are whatever Windows draws for that control.
//!
//! Timing is GPUI's tooltip machinery, which matches Chromium's: shown 500 ms after the pointer
//! comes to rest on the element (Chromium's `TooltipController` uses 500 ms), hidden when the
//! pointer leaves, on a mouse press and on scrolling. Chromium also hides a tooltip after it has
//! been shown 10 s; that is done here too (recalled from Chromium's `TooltipController`, not
//! measured on this PC).
//!
//! Use `.native_tooltip(text)` on any stateful element (`div().id(..)`). Captures and tests
//! never show anything: the harness turns native tooltips off ([`set_native`]) and reads the
//! text that would have shown ([`last_shown`]).
//!
//! macOS: GPUI's in-window tooltip with a plain light box (clipped at the window edge, unlike
//! the native one); not checked against anything.

use gpui::{div, prelude::FluentBuilder, px, App, AppContext, Context, Global, IntoElement, ParentElement, Render, SharedString, StatefulInteractiveElement, Styled, Window};
use std::time::Duration;

/// How long Chromium leaves a tooltip up.
pub const SHOWN_TIMEOUT: Duration = Duration::from_secs(10);

#[derive(Default)]
struct Tooltips {
    disabled: bool,
    last: Option<SharedString>,
    // The tooltip currently up (its generation), so a late hide of an older one is ignored.
    generation: u64,
}

impl Global for Tooltips {}

/// Turns the native tooltip window on or off (the capture harness turns it off).
pub fn set_native(enabled: bool, cx: &mut App) {
    cx.default_global::<Tooltips>().disabled = !enabled;
}

/// The text of the last tooltip GPUI asked to show (shown natively or not).
pub fn last_shown(cx: &App) -> Option<SharedString> {
    cx.try_global::<Tooltips>().and_then(|tooltips| tooltips.last.clone())
}

pub trait TooltipExt: StatefulInteractiveElement + Sized {
    /// A native tooltip with `text` (the page's `title`).
    fn native_tooltip(self, text: impl Into<SharedString>) -> Self {
        let text: SharedString = text.into();
        self.tooltip(move |window, cx| show(text.clone(), window, cx).into())
    }
}

impl<T: StatefulInteractiveElement> TooltipExt for T {}

/// What GPUI holds while the tooltip is up: draws nothing in the window (except on macOS), and
/// hides the native tooltip when GPUI drops it.
pub struct TooltipView {
    text: SharedString,
    generation: u64,
    native: bool,
}

impl Render for TooltipView {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        // macOS fallback: a plain box like the system's.
        div().when(!self.native, |d| {
            d.mt(px(20.0))
                .px(px(6.0))
                .py(px(2.0))
                .bg(gpui::rgb(0xf9f9f9))
                .border_1()
                .border_color(gpui::rgb(0xcccccc))
                .text_size(px(12.0))
                .text_color(gpui::rgb(0x1a1a1a))
                .child(self.text.clone())
        })
    }
}

impl Drop for TooltipView {
    fn drop(&mut self) {
        if self.native {
            native::hide(self.generation);
        }
    }
}

fn show(text: SharedString, window: &mut Window, cx: &mut App) -> gpui::Entity<TooltipView> {
    let tooltips = cx.default_global::<Tooltips>();
    tooltips.last = Some(text.clone());
    tooltips.generation += 1;
    let generation = tooltips.generation;
    let native = !tooltips.disabled && native::SUPPORTED;
    if native {
        native::show(&text, generation, window);
        let timeout = window.spawn(cx, async move |cx| {
            cx.background_executor().timer(SHOWN_TIMEOUT).await;
            let _ = cx.update(|_, _| native::hide(generation));
        });
        timeout.detach();
    }
    cx.new(|_| TooltipView { text, generation, native })
}

#[cfg(windows)]
mod native {
    use gpui::Window;
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};
    use std::cell::RefCell;
    use windows::core::PCWSTR;
    use windows::Win32::Foundation::{HWND, LPARAM, POINT, RECT, WPARAM};
    use windows::Win32::Graphics::Gdi::{
        DeleteObject, GetDIBits, GetMonitorInfoW, MonitorFromPoint, BITMAPINFO, BITMAPINFOHEADER, DIB_RGB_COLORS, HDC, MONITORINFO, MONITOR_DEFAULTTONEAREST,
    };
    use windows::Win32::System::LibraryLoader::GetModuleHandleW;
    use windows::Win32::UI::Controls::{
        InitCommonControlsEx, ICC_WIN95_CLASSES, INITCOMMONCONTROLSEX, TOOLTIPS_CLASSW, TTF_ABSOLUTE, TTF_IDISHWND, TTF_TRACK, TTM_ADDTOOLW, TTM_GETBUBBLESIZE,
        TTM_SETMAXTIPWIDTH, TTM_TRACKACTIVATE, TTM_TRACKPOSITION, TTM_UPDATETIPTEXTW, TTS_NOPREFIX, TTTOOLINFOW,
    };
    use windows::Win32::UI::WindowsAndMessaging::{
        CreateWindowExW, DestroyWindow, GetAncestor, GetCursorInfo, GetCursorPos, GetIconInfo, IsWindow, SendMessageW, SetWindowPos, WindowFromPoint, CURSORINFO, GA_ROOT,
        HWND_TOPMOST, ICONINFO, SWP_NOACTIVATE, SWP_NOSIZE, WINDOW_EX_STYLE, WS_EX_TOPMOST, WS_EX_TRANSPARENT, WS_POPUP,
    };

    pub const SUPPORTED: bool = true;

    struct Native {
        tooltip: HWND,
        owner: HWND,
        shown: Option<u64>,
    }

    thread_local! {
        // One tooltip control per owner window, created on first use (the UI thread only).
        static TOOLTIPS: RefCell<Vec<Native>> = const { RefCell::new(Vec::new()) };
    }

    fn owner_of(window: &Window) -> Option<HWND> {
        match HasWindowHandle::window_handle(window).ok()?.as_raw() {
            RawWindowHandle::Win32(handle) => Some(HWND(handle.hwnd.get() as *mut _)),
            _ => None,
        }
    }

    fn tool(owner: HWND, text: &mut [u16]) -> TTTOOLINFOW {
        TTTOOLINFOW {
            cbSize: std::mem::size_of::<TTTOOLINFOW>() as u32,
            uFlags: TTF_IDISHWND | TTF_TRACK | TTF_ABSOLUTE,
            hwnd: owner,
            uId: owner.0 as usize,
            lpszText: windows::core::PWSTR(text.as_mut_ptr()),
            ..Default::default()
        }
    }

    fn control(owner: HWND) -> Option<HWND> {
        TOOLTIPS.with(|tooltips| {
            let mut tooltips = tooltips.borrow_mut();
            tooltips.retain(|t| unsafe { IsWindow(Some(t.owner)).as_bool() && IsWindow(Some(t.tooltip)).as_bool() });
            if let Some(existing) = tooltips.iter().find(|t| t.owner == owner) {
                return Some(existing.tooltip);
            }
            unsafe {
                let init = INITCOMMONCONTROLSEX { dwSize: std::mem::size_of::<INITCOMMONCONTROLSEX>() as u32, dwICC: ICC_WIN95_CLASSES };
                let _ = InitCommonControlsEx(&init);
                let instance = GetModuleHandleW(PCWSTR::null()).ok()?;
                let tooltip = CreateWindowExW(
                    WINDOW_EX_STYLE(WS_EX_TRANSPARENT.0 | WS_EX_TOPMOST.0),
                    TOOLTIPS_CLASSW,
                    PCWSTR::null(),
                    WS_POPUP | windows::Win32::UI::WindowsAndMessaging::WINDOW_STYLE(TTS_NOPREFIX),
                    0,
                    0,
                    0,
                    0,
                    Some(owner),
                    None,
                    Some(instance.into()),
                    None,
                )
                .ok()?;
                let mut empty = [0u16];
                let info = tool(owner, &mut empty);
                SendMessageW(tooltip, TTM_ADDTOOLW, Some(WPARAM(0)), Some(LPARAM(&info as *const _ as isize)));
                tooltips.push(Native { tooltip, owner, shown: None });
                Some(tooltip)
            }
        })
    }

    // How far the visible part of the current cursor reaches below its hot spot, in screen
    // pixels (Chromium's GetCurrentCursorVisibleHeight).
    fn cursor_visible_height() -> i32 {
        unsafe {
            let mut info = CURSORINFO { cbSize: std::mem::size_of::<CURSORINFO>() as u32, ..Default::default() };
            if GetCursorInfo(&mut info).is_err() || info.hCursor.is_invalid() {
                return 20;
            }
            let mut icon = ICONINFO::default();
            if GetIconInfo(info.hCursor.into(), &mut icon).is_err() {
                return 20;
            }
            let hotspot = icon.yHotspot as i32;
            let mut height = 20;
            if !icon.hbmMask.is_invalid() {
                let mut header = BITMAPINFO::default();
                header.bmiHeader.biSize = std::mem::size_of::<BITMAPINFOHEADER>() as u32;
                let dc = windows::Win32::Graphics::Gdi::GetDC(None);
                if GetDIBits(dc, icon.hbmMask, 0, 0, None, &mut header, DIB_RGB_COLORS) != 0 {
                    let w = header.bmiHeader.biWidth;
                    let h = header.bmiHeader.biHeight.abs();
                    header.bmiHeader.biBitCount = 32;
                    header.bmiHeader.biCompression = 0;
                    header.bmiHeader.biHeight = -h;
                    let mut pixels = vec![0u32; (w * h) as usize];
                    if GetDIBits(dc, icon.hbmMask, 0, h as u32, Some(pixels.as_mut_ptr() as *mut _), &mut header, DIB_RGB_COLORS) != 0 {
                        // A monochrome cursor's mask is AND over XOR (twice the height); a color
                        // cursor's mask is its own height. Opaque pixels are 0 in the AND mask.
                        let rows = if icon.hbmColor.is_invalid() { h / 2 } else { h };
                        let last = (0..rows).rev().find(|&y| (0..w).any(|x| pixels[(y * w + x) as usize] & 0x00ff_ffff == 0));
                        if let Some(last) = last {
                            height = (last + 1 - hotspot).max(0);
                        }
                    }
                }
                windows::Win32::Graphics::Gdi::ReleaseDC(None, dc);
            }
            let _ = DeleteObject(icon.hbmMask.into());
            if !icon.hbmColor.is_invalid() {
                let _ = DeleteObject(icon.hbmColor.into());
            }
            let _: Option<HDC> = None;
            height
        }
    }

    pub fn show(text: &str, generation: u64, window: &Window) {
        let Some(owner) = owner_of(window) else { return };
        let mut cursor = POINT::default();
        unsafe {
            let _ = GetCursorPos(&mut cursor);
            // Only while the pointer is over this window itself: when another window covers it
            // (a panel a click just opened, under a pointer that hasn't moved since), GPUI still
            // takes the element as hovered, and the browser would show nothing.
            let under = WindowFromPoint(cursor);
            if under.is_invalid() || GetAncestor(under, GA_ROOT) != owner {
                return;
            }
        }
        let Some(tooltip) = control(owner) else { return };
        unsafe {
            let mut wide: Vec<u16> = text.encode_utf16().chain(Some(0)).collect();
            let info = tool(owner, &mut wide);
            let monitor = MonitorFromPoint(cursor, MONITOR_DEFAULTTONEAREST);
            let mut monitor_info = MONITORINFO { cbSize: std::mem::size_of::<MONITORINFO>() as u32, ..Default::default() };
            let work: RECT = if GetMonitorInfoW(monitor, &mut monitor_info).as_bool() { monitor_info.rcWork } else { RECT { left: 0, top: 0, right: 1920, bottom: 1080 } };
            // Chromium caps a tooltip at half the work area's width before it wraps.
            SendMessageW(tooltip, TTM_SETMAXTIPWIDTH, Some(WPARAM(0)), Some(LPARAM(((work.right - work.left) / 2) as isize)));
            SendMessageW(tooltip, TTM_UPDATETIPTEXTW, Some(WPARAM(0)), Some(LPARAM(&info as *const _ as isize)));
            let offset = cursor_visible_height();
            let bubble = SendMessageW(tooltip, TTM_GETBUBBLESIZE, Some(WPARAM(0)), Some(LPARAM(&info as *const _ as isize))).0 as u32;
            let (width, height) = ((bubble & 0xffff) as i32, (bubble >> 16) as i32);
            let mut x = cursor.x;
            let mut y = cursor.y + offset;
            if x + width > work.right {
                x = work.right - width;
            }
            if y + height > work.bottom {
                y = cursor.y - height;
            }
            x = x.max(work.left);
            y = y.max(work.top);
            SendMessageW(tooltip, TTM_TRACKPOSITION, Some(WPARAM(0)), Some(LPARAM(((y as u32 & 0xffff) << 16 | (x as u32 & 0xffff)) as isize)));
            SendMessageW(tooltip, TTM_TRACKACTIVATE, Some(WPARAM(1)), Some(LPARAM(&info as *const _ as isize)));
            let _ = SetWindowPos(tooltip, Some(HWND_TOPMOST), x, y, 0, 0, SWP_NOACTIVATE | SWP_NOSIZE);
        }
        TOOLTIPS.with(|tooltips| {
            for t in tooltips.borrow_mut().iter_mut() {
                t.shown = if t.owner == owner { Some(generation) } else { None };
            }
        });
    }

    pub fn hide(generation: u64) {
        TOOLTIPS.with(|tooltips| {
            for t in tooltips.borrow_mut().iter_mut().filter(|t| t.shown == Some(generation)) {
                unsafe {
                    let mut empty = [0u16];
                    let info = tool(t.owner, &mut empty);
                    SendMessageW(t.tooltip, TTM_TRACKACTIVATE, Some(WPARAM(0)), Some(LPARAM(&info as *const _ as isize)));
                }
                t.shown = None;
            }
        });
    }

    #[allow(dead_code)]
    pub fn destroy_all() {
        TOOLTIPS.with(|tooltips| {
            for t in tooltips.borrow_mut().drain(..) {
                unsafe {
                    let _ = DestroyWindow(t.tooltip);
                }
            }
        });
    }
}

#[cfg(not(windows))]
mod native {
    pub const SUPPORTED: bool = false;
    pub fn show(_: &str, _: u64, _: &gpui::Window) {}
    pub fn hide(_: u64) {}
}
