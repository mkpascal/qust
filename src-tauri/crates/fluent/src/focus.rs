//! Focus: Tab order, the focus ring rule, and the dialog's Tab cycle.
//!
//! **Tab order** is declaration order: a focusable element gets a `FocusHandle` made with
//! [`tab_stop`] (`tab_index(0)`, `tab_stop(true)`) and is rendered with
//! `.track_focus(&handle)`. GPUI walks tab stops in paint order, which is the page's DOM order
//! when views are laid out in the same order as the HTML. Disabled controls don't track their
//! handle (they leave the order, as disabled buttons did). [`init`] binds Tab / Shift+Tab to
//! [`FocusNext`] / [`FocusPrev`], and [`tab_navigation`] on a window's root element handles them.
//!
//! **The ring** follows Chromium's `:focus-visible`, not GPUI's: GPUI shows its `focus_visible`
//! style only while the last input was the keyboard, so the ring would vanish as soon as the
//! mouse moves. Chromium keeps it on the element that keyboard input focused until focus moves.
//! [`is_focus_visible`] is that rule: the element is focused and is the one that was focused
//! right after the last keystroke in its window. A click elsewhere moves focus and hides it; a
//! script focus after a keystroke (the dialog's Name field after Enter) shows it.
//!
//! **Dialogs** keep Tab inside themselves: [`FocusCycle`] holds the dialog's handles in order
//! and wraps Tab on the last to the first and Shift+Tab on the first to the last.

use gpui::{actions, App, FocusHandle, Global, InteractiveElement, KeyBinding, WeakFocusHandle, Window, WindowId};
use std::collections::HashMap;

actions!(qust_focus, [FocusNext, FocusPrev]);

#[derive(Default)]
struct KeyboardFocus {
    by_window: HashMap<WindowId, WeakFocusHandle>,
}

impl Global for KeyboardFocus {}

/// Binds Tab / Shift+Tab and starts tracking keyboard focus. Called once by `qust_fluent::init`.
pub fn init(cx: &mut App) {
    cx.bind_keys([KeyBinding::new("tab", FocusNext, None), KeyBinding::new("shift-tab", FocusPrev, None)]);
    cx.set_global(KeyboardFocus::default());
    cx.observe_keystrokes(|_, window, cx| {
        let window_id = window.window_handle().window_id();
        let focused = window.focused(cx).map(|handle| handle.downgrade());
        let record = cx.global_mut::<KeyboardFocus>();
        match focused {
            Some(handle) => {
                record.by_window.insert(window_id, handle);
            }
            None => {
                record.by_window.remove(&window_id);
            }
        }
    })
    .detach();
}

/// A focus handle that takes part in Tab navigation.
pub fn tab_stop(cx: &mut App) -> FocusHandle {
    cx.focus_handle().tab_index(0).tab_stop(true)
}

/// Whether `handle` shows its focus ring (`:focus-visible`), by Chromium's rule above.
pub fn is_focus_visible(handle: &FocusHandle, window: &Window, cx: &App) -> bool {
    if !handle.is_focused(window) {
        return false;
    }
    let window_id = window.window_handle().window_id();
    cx.try_global::<KeyboardFocus>()
        .and_then(|record| record.by_window.get(&window_id))
        .and_then(|weak| weak.upgrade())
        .is_some_and(|recorded| recorded == *handle)
}

/// Marks the focus `handle` as keyboard-focused, for a script focus that follows keyboard input
/// outside a keystroke (rarely needed: keystrokes are tracked already).
pub fn mark_keyboard_focus(handle: &FocusHandle, window: &Window, cx: &mut App) {
    let window_id = window.window_handle().window_id();
    if cx.has_global::<KeyboardFocus>() {
        cx.global_mut::<KeyboardFocus>().by_window.insert(window_id, handle.downgrade());
    }
}

/// Tab and Shift+Tab move focus through the window's tab stops. Put it on the window's root.
pub fn tab_navigation<E: InteractiveElement>(element: E) -> E {
    element
        .on_action(|_: &FocusNext, window, cx| window.focus_next(cx))
        .on_action(|_: &FocusPrev, window, cx| window.focus_prev(cx))
}

/// A dialog's own Tab order, wrapping at both ends (BEHAVIOUR.md 2.8).
#[derive(Clone, Default)]
pub struct FocusCycle {
    pub handles: Vec<FocusHandle>,
}

impl FocusCycle {
    pub fn new(handles: Vec<FocusHandle>) -> FocusCycle {
        FocusCycle { handles }
    }

    /// Moves focus one step (`back` for Shift+Tab). Returns false when nothing in the cycle had
    /// focus (then focus goes to the first, or the last going back).
    pub fn step(&self, back: bool, window: &mut Window, cx: &mut App) -> bool {
        if self.handles.is_empty() {
            return false;
        }
        let current = self.handles.iter().position(|handle| handle.is_focused(window) || handle.contains_focused(window, cx));
        let n = self.handles.len();
        let next = match (current, back) {
            (Some(i), false) => (i + 1) % n,
            (Some(i), true) => (i + n - 1) % n,
            (None, false) => 0,
            (None, true) => n - 1,
        };
        self.handles[next].focus(window, cx);
        current.is_some()
    }

    /// Handlers for a dialog's root element: Tab / Shift+Tab stay inside.
    pub fn trap<E: InteractiveElement>(&self, element: E) -> E {
        let forward = self.clone();
        let back = self.clone();
        element
            .capture_action(move |_: &FocusNext, window, cx| {
                forward.step(false, window, cx);
                cx.stop_propagation();
            })
            .capture_action(move |_: &FocusPrev, window, cx| {
                back.step(true, window, cx);
                cx.stop_propagation();
            })
    }
}
