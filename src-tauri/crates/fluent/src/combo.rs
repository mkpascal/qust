//! The Settings combo (`.combo`) and the shortcut box with its key caps (`.shortcut`, `kbd`).
//!
//! Combo: 32 tall, min-width 170, padding 0 10 0 12, gap 12, space-between: the current choice
//! (14/20 `--text`) and a 12 px chevron in `--text-2`; the button's border and fill; hover
//! `--control-hover`, pressed `--control-pressed` (the text doesn't dim, unlike `.button`). A
//! click hands the handler the combo's bounds: the page opened the native menu at its bottom-left
//! + 2 px (`qust-shell` shows the menu).
//!
//! Shortcut box: like the combo, but padding 0 4 0 12, gap 4, content at the right, 13/20
//! `--text-2`; hover `--control-hover`. Its states: keys as caps (accent fill, `--on-accent`,
//! semibold 12/20, padding 1 7, radius 4, min-width 24, centred); "Not set" in `--text-3`;
//! unavailable caps (`--control-hover`, `--text-2`, struck through); recording: a dashed 1 px
//! accent border and "Press a shortcut…" in `--text`.

use crate::button::RefineStyle;
use crate::focus::is_focus_visible;
use crate::icons::{icon, Icon};
use crate::paint::bounds_probe;
use crate::state::Forced;
use crate::surface::{border_layers, focus_ring, inset_layer, Border};
use crate::text::{line, TextStyles};
use crate::theme::theme;
use crate::tooltip::TooltipExt;
use gpui::prelude::FluentBuilder;
use gpui::{
    div, px, App, Bounds, ElementId, FocusHandle, InteractiveElement, IntoElement, ParentElement, Pixels, RenderOnce, Role, SharedString,
    StatefulInteractiveElement, StyleRefinement, Styled, Window,
};
use std::rc::Rc;

type BoundsHandler = Rc<dyn Fn(Bounds<Pixels>, &mut Window, &mut App)>;

const GROUP: &str = "qust-combo";

#[derive(IntoElement)]
pub struct Combo {
    id: ElementId,
    text: SharedString,
    forced: Forced,
    focus: Option<FocusHandle>,
    on_click: Option<BoundsHandler>,
    label: Option<SharedString>,
    style: StyleRefinement,
}

impl Combo {
    pub fn new(id: impl Into<ElementId>, text: impl Into<SharedString>) -> Combo {
        Combo { id: id.into(), text: text.into(), forced: Forced::NONE, focus: None, on_click: None, label: None, style: StyleRefinement::default() }
    }

    pub fn forced(mut self, forced: Forced) -> Self {
        self.forced = forced;
        self
    }

    pub fn focus(mut self, handle: &FocusHandle) -> Self {
        self.focus = Some(handle.clone());
        self
    }

    /// The accessible name (the setting's title).
    pub fn aria_label(mut self, label: impl Into<SharedString>) -> Self {
        self.label = Some(label.into());
        self
    }

    /// Called with the combo's border box (window coordinates, CSS px), as the page's
    /// `getBoundingClientRect()`: the native menu opens at its bottom-left + 2 px.
    pub fn on_click(mut self, handler: impl Fn(Bounds<Pixels>, &mut Window, &mut App) + 'static) -> Self {
        self.on_click = Some(Rc::new(handler));
        self
    }
}

impl Styled for Combo {
    fn style(&mut self) -> &mut StyleRefinement {
        &mut self.style
    }
}

impl RenderOnce for Combo {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = *theme(cx);
        let forced = self.forced;
        let fill = if forced.active { theme.control_pressed } else if forced.hover { theme.control_hover } else { theme.control };
        let ring = forced.focus_visible || self.focus.as_ref().is_some_and(|handle| is_focus_visible(handle, window, cx));
        let (bounds, probe) = bounds_probe((self.id.clone(), "bounds"), window, cx);
        let mut root = div()
            .id(self.id.clone())
            .group(GROUP)
            .relative()
            .flex()
            .flex_row()
            .items_center()
            .justify_between()
            .gap(px(12.0))
            .min_w(px(170.0))
            .h(px(32.0))
            .border_1()
            .border_color(crate::Color::TRANSPARENT)
            .pl(px(12.0))
            .pr(px(10.0))
            .font_text()
            .type_size(14.0, 20.0)
            .text_color(theme.text)
            .child(probe)
            .child(
                inset_layer(0.0, 3.0)
                    .bg(fill)
                    .when(!forced.any(), |layer| layer.group_hover(GROUP, |s| s.bg(theme.control_hover)))
                    .id("fill")
                    .when(!forced.any(), |layer| layer.group_active(GROUP, |s| s.bg(theme.control_pressed))),
            )
            .children(border_layers(4.0, Border::top(theme.control_stroke, theme.control_stroke_top), false, -1.0))
            .child(line(self.text.clone()).relative())
            .child(icon(Icon::SettingsChevron, 12.0).relative().text_color(theme.text_2))
            .role(Role::ComboBox)
            .aria_label(self.label.unwrap_or(self.text))
            .when(ring, |r| r.child(focus_ring(4.0, &theme)));
        if let Some(handle) = &self.focus {
            root = root.track_focus(handle);
        }
        if let Some(handler) = self.on_click {
            root = root.on_click(move |_, window, cx| {
                // The probe measures the padding box, inside the 1 px border.
                if let Some(bounds) = *bounds.read(cx) {
                    handler(bounds.dilate(px(1.0)), window, cx);
                }
            });
        }
        root.refine_style(&self.style)
    }
}

/// The key names a shortcut box shows for an accelerator (`shortcutKeys` in bridge.js):
/// `Ctrl+Alt+KeyH` → Ctrl, Alt, H; Super → Win, Digit5 → 5, Numpad… → "Num …", Arrow… →
/// Up / Down / Left / Right, Backquote → `; everything else as the code.
pub fn shortcut_keys(accelerator: &str) -> Vec<String> {
    accelerator
        .split('+')
        .map(|part| match part {
            "Super" => "Win".to_string(),
            "ArrowUp" => "Up".to_string(),
            "ArrowDown" => "Down".to_string(),
            "ArrowLeft" => "Left".to_string(),
            "ArrowRight" => "Right".to_string(),
            "Backquote" => "`".to_string(),
            _ => {
                if let Some(letter) = part.strip_prefix("Key").filter(|rest| rest.len() == 1 && rest.chars().all(|c| c.is_ascii_uppercase())) {
                    letter.to_string()
                } else if let Some(digit) = part.strip_prefix("Digit").filter(|rest| rest.len() == 1 && rest.chars().all(|c| c.is_ascii_digit())) {
                    digit.to_string()
                } else if let Some(rest) = part.strip_prefix("Numpad") {
                    format!("Num {rest}")
                } else {
                    part.to_string()
                }
            }
        })
        .collect()
}

/// What a shortcut box shows.
#[derive(Clone, Debug, PartialEq)]
pub enum ShortcutDisplay {
    /// Assigned: the keys, and whether another app owns it (struck through).
    Keys { keys: Vec<String>, unavailable: bool },
    /// "Not set".
    Unassigned,
    /// "Press a shortcut…" with the dashed accent border.
    Recording,
}

#[derive(IntoElement)]
pub struct ShortcutBox {
    id: ElementId,
    display: ShortcutDisplay,
    forced: Forced,
    focus: Option<FocusHandle>,
    on_click: Option<Rc<dyn Fn(&mut Window, &mut App)>>,
    label: SharedString,
}

impl ShortcutBox {
    pub fn new(id: impl Into<ElementId>, display: ShortcutDisplay, label: impl Into<SharedString>) -> ShortcutBox {
        ShortcutBox { id: id.into(), display, forced: Forced::NONE, focus: None, on_click: None, label: label.into() }
    }

    pub fn forced(mut self, forced: Forced) -> Self {
        self.forced = forced;
        self
    }

    pub fn focus(mut self, handle: &FocusHandle) -> Self {
        self.focus = Some(handle.clone());
        self
    }

    pub fn on_click(mut self, handler: impl Fn(&mut Window, &mut App) + 'static) -> Self {
        self.on_click = Some(Rc::new(handler));
        self
    }
}

/// One key cap (`.shortcut kbd`).
pub fn keycap(text: impl Into<SharedString>, unavailable: bool, theme: &crate::Theme) -> gpui::Div {
    let (fill, color) = if unavailable { (theme.control_hover, theme.text_2) } else { (theme.accent, theme.on_accent) };
    div()
        .flex()
        .justify_center()
        .min_w(px(24.0))
        .px(px(7.0))
        .py(px(1.0))
        .rounded(px(4.0))
        .bg(fill)
        .text_color(color)
        .font_text()
        .semibold()
        .type_size(12.0, 20.0)
        .when(unavailable, |k| k.line_through())
        .child(line(text))
}

impl RenderOnce for ShortcutBox {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = *theme(cx);
        let forced = self.forced;
        let recording = self.display == ShortcutDisplay::Recording;
        let fill = if forced.hover { theme.control_hover } else { theme.control };
        let border = if recording { Border::all(theme.accent) } else { Border::top(theme.control_stroke, theme.control_stroke_top) };
        let color = match &self.display {
            ShortcutDisplay::Recording => theme.text,
            ShortcutDisplay::Unassigned => theme.text_3,
            ShortcutDisplay::Keys { .. } => theme.text_2,
        };
        let ring = forced.focus_visible || self.focus.as_ref().is_some_and(|handle| is_focus_visible(handle, window, cx));
        let content: Vec<gpui::AnyElement> = match self.display {
            ShortcutDisplay::Keys { keys, unavailable } => keys.into_iter().map(|key| keycap(key, unavailable, &theme).relative().into_any_element()).collect(),
            ShortcutDisplay::Unassigned => vec![line("Not set").relative().into_any_element()],
            ShortcutDisplay::Recording => vec![line("Press a shortcut\u{2026}").relative().into_any_element()],
        };
        let mut root = div()
            .id(self.id.clone())
            .group(GROUP)
            .relative()
            .flex()
            .flex_row()
            .items_center()
            .justify_end()
            .gap(px(4.0))
            .min_w(px(170.0))
            .h(px(32.0))
            .border_1()
            .border_color(crate::Color::TRANSPARENT)
            .pl(px(12.0))
            .pr(px(4.0))
            .font_text()
            .type_size(13.0, 20.0)
            .text_color(color)
            .child(inset_layer(0.0, 3.0).bg(fill).when(!forced.any(), |layer| layer.group_hover(GROUP, |s| s.bg(theme.control_hover))))
            .children(border_layers(4.0, border, recording, -1.0))
            .children(content)
            .role(Role::Button)
            .aria_label(self.label)
            .native_tooltip("Click, then press a new shortcut")
            .when(ring, |r| r.child(focus_ring(4.0, &theme)));
        if let Some(handle) = &self.focus {
            root = root.track_focus(handle);
        }
        if let Some(handler) = self.on_click {
            root = root.on_click(move |_, window, cx| handler(window, cx));
        }
        root
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn key_names() {
        assert_eq!(shortcut_keys("Ctrl+Alt+KeyH"), ["Ctrl", "Alt", "H"]);
        assert_eq!(shortcut_keys("Ctrl+Alt+Insert"), ["Ctrl", "Alt", "Insert"]);
        assert_eq!(shortcut_keys("Super+Shift+Digit5"), ["Win", "Shift", "5"]);
        assert_eq!(shortcut_keys("Ctrl+Numpad7"), ["Ctrl", "Num 7"]);
        assert_eq!(shortcut_keys("Alt+ArrowUp"), ["Alt", "Up"]);
        assert_eq!(shortcut_keys("Ctrl+Backquote"), ["Ctrl", "`"]);
        assert_eq!(shortcut_keys("MediaPlayPause"), ["MediaPlayPause"]);
    }
}
