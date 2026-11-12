//! `.button` and its variants (style.css "Buttons"): the plain button, `.accent`, the toggle
//! (`aria-pressed`), `.icon` (44 px, no padding), text buttons with an icon and a trailing
//! chevron (the deck bar), and the update dot (`#settingsButton.has-update::after`).
//!
//! Look, value for value: 32 px tall, min-width 32, padding 0 12, gap 8, radius 4, 1 px
//! `--control-stroke` border with a `--control-stroke-top` top edge, `--control` fill inside the
//! border, 14/20 text. Hover `--control-hover`; pressed (mouse held) `--control-pressed` and
//! `--text-2`; disabled `--control-disabled` and `--text-disabled` (disabled wins over hover and
//! pressed, as in the cascade). Accent: accent fill, `--on-accent` text, border
//! `rgba(255,255,255,.08)` with a `rgba(0,0,0,.14)` bottom; hover 90 %, pressed 80 % with text at
//! 75 %; disabled `--accent-disabled`, transparent border, `--text-disabled`. A pressed toggle
//! looks like the accent button with hover 90 %; held down it keeps that look (its rules come
//! later in style.css than `.button:active`). No transitions: state changes are instant.
//!
//! Enter and Space activate a focused button (GPUI fires the click on key-up; Chromium fires
//! Enter on key-down, Space on key-up).

use crate::a11y::A11yExt;
use crate::color::Color;
use crate::focus::is_focus_visible;
use crate::icons::{icon, Icon};
use crate::state::Forced;
use crate::surface::{border_layers, focus_ring, inset_layer, Border};
use crate::text::{line, TextStyles};
use crate::theme::theme;
use crate::tooltip::TooltipExt;
use gpui::prelude::FluentBuilder;
use gpui::{
    div, px, AnyElement, App, ClickEvent, ElementId, FocusHandle, InteractiveElement, IntoElement, ParentElement, RenderOnce, Role, SharedString,
    StatefulInteractiveElement, StyleRefinement, Styled, Window,
};
use std::rc::Rc;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ButtonKind {
    Standard,
    /// `.button.accent`
    Accent,
    /// A toggle button (`aria-pressed`), pressed or not.
    Toggle(bool),
}

type ClickHandler = Rc<dyn Fn(&ClickEvent, &mut Window, &mut App)>;

/// A button. `Button::new(id).icon(Icon::Play).accent()…`, then style it like a div for its
/// size in the layout (`.flex_1()`, `.w(px(44.))`); `.text_size()` and padding override the
/// defaults the way the page's more specific rules did (the deck bar's `font-size: 13px;
/// padding: 0 8px; gap: 7px`).
#[derive(IntoElement)]
pub struct Button {
    id: ElementId,
    kind: ButtonKind,
    icon: Option<(Icon, f32)>,
    label: Option<SharedString>,
    trailing: Option<AnyElement>,
    icon_only: bool,
    gap: f32,
    disabled: bool,
    forced: Forced,
    focus: Option<FocusHandle>,
    on_click: Option<ClickHandler>,
    aria_label: Option<SharedString>,
    expanded: Option<bool>,
    tooltip: Option<SharedString>,
    update_dot: bool,
    style: StyleRefinement,
}

impl Button {
    pub fn new(id: impl Into<ElementId>) -> Button {
        Button {
            id: id.into(),
            kind: ButtonKind::Standard,
            icon: None,
            label: None,
            trailing: None,
            icon_only: false,
            gap: 8.0,
            disabled: false,
            forced: Forced::NONE,
            focus: None,
            on_click: None,
            aria_label: None,
            expanded: None,
            tooltip: None,
            update_dot: false,
            style: StyleRefinement::default(),
        }
    }

    pub fn accent(mut self) -> Self {
        self.kind = ButtonKind::Accent;
        self
    }

    /// A toggle (`aria-pressed`).
    pub fn toggle(mut self, pressed: bool) -> Self {
        self.kind = ButtonKind::Toggle(pressed);
        self
    }

    pub fn kind(mut self, kind: ButtonKind) -> Self {
        self.kind = kind;
        self
    }

    /// A leading 16 px icon.
    pub fn icon(mut self, icon: Icon) -> Self {
        self.icon = Some((icon, 16.0));
        self
    }

    /// `.button.icon`: an icon alone, 44 px wide with no padding (`aria-label` names it).
    pub fn icon_only(mut self, icon: Icon) -> Self {
        self.icon = Some((icon, 16.0));
        self.icon_only = true;
        self
    }

    pub fn label(mut self, label: impl Into<SharedString>) -> Self {
        self.label = Some(label.into());
        self
    }

    /// Something after the label (the Playlist button's chevron).
    pub fn trailing(mut self, element: impl IntoElement) -> Self {
        self.trailing = Some(element.into_any_element());
        self
    }

    /// The flex gap between icon, label and trailing element (8; the deck bar's is 7).
    pub fn gap(mut self, gap: f32) -> Self {
        self.gap = gap;
        self
    }

    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }

    /// Hover / pressed / focus-visible forced on, as a fixture's `force` does.
    pub fn forced(mut self, forced: Forced) -> Self {
        self.forced = forced;
        self
    }

    /// Makes it a tab stop with this handle (see `focus::tab_stop`).
    pub fn focus(mut self, handle: &FocusHandle) -> Self {
        self.focus = Some(handle.clone());
        self
    }

    pub fn on_click(mut self, handler: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static) -> Self {
        self.on_click = Some(Rc::new(handler));
        self
    }

    pub fn aria_label(mut self, label: impl Into<SharedString>) -> Self {
        self.aria_label = Some(label.into());
        self
    }

    /// `aria-expanded` (the Playlist button).
    pub fn expanded(mut self, expanded: bool) -> Self {
        self.expanded = Some(expanded);
        self
    }

    /// The `title` attribute: a native tooltip.
    pub fn tooltip(mut self, text: impl Into<SharedString>) -> Self {
        self.tooltip = Some(text.into());
        self
    }

    /// The update dot (`#settingsButton.has-update::after`).
    pub fn update_dot(mut self, on: bool) -> Self {
        self.update_dot = on;
        self
    }
}

impl Styled for Button {
    fn style(&mut self) -> &mut StyleRefinement {
        &mut self.style
    }
}

/// The colors of a button in one state.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ButtonColors {
    pub fill: Color,
    pub text: Color,
    pub border: Border,
}

/// The cascade of `.button` rules for `kind` in a state.
pub fn button_colors(kind: ButtonKind, hover: bool, pressed: bool, disabled: bool, theme: &crate::Theme) -> ButtonColors {
    let edge = Color::rgba(255, 255, 255, 20.0 / 255.0);
    let edge_bottom = Color::rgba(0, 0, 0, 36.0 / 255.0);
    let accent_border = Border::bottom(edge, edge_bottom);
    let plain_border = Border::top(theme.control_stroke, theme.control_stroke_top);
    let accented = matches!(kind, ButtonKind::Accent | ButtonKind::Toggle(true));
    match kind {
        _ if disabled && kind == ButtonKind::Accent => ButtonColors { fill: theme.accent_disabled, text: theme.text_disabled, border: Border::all(Color::TRANSPARENT) },
        _ if disabled && accented => ButtonColors { fill: theme.accent, text: theme.on_accent, border: accent_border },
        _ if disabled => ButtonColors { fill: theme.control_disabled, text: theme.text_disabled, border: plain_border },
        ButtonKind::Accent if pressed => ButtonColors { fill: theme.accent.mix(0.8), text: theme.on_accent.mix(0.75), border: accent_border },
        ButtonKind::Accent if hover => ButtonColors { fill: theme.accent.mix(0.9), text: theme.on_accent, border: accent_border },
        ButtonKind::Accent => ButtonColors { fill: theme.accent, text: theme.on_accent, border: accent_border },
        // `[aria-pressed="true"]:hover` (0,3,0) beats `.button:active` (0,2,0); held without
        // hover (a forced :active) the pressed toggle's own rule, later in the file, wins.
        ButtonKind::Toggle(true) if hover => ButtonColors { fill: theme.accent.mix(0.9), text: theme.on_accent, border: accent_border },
        ButtonKind::Toggle(true) => ButtonColors { fill: theme.accent, text: theme.on_accent, border: accent_border },
        _ if pressed => ButtonColors { fill: theme.control_pressed, text: theme.text_2, border: plain_border },
        _ if hover => ButtonColors { fill: theme.control_hover, text: theme.text, border: plain_border },
        _ => ButtonColors { fill: theme.control, text: theme.text, border: plain_border },
    }
}

const GROUP: &str = "qust-button";

impl RenderOnce for Button {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = *theme(cx);
        let forced = self.forced;
        let base = button_colors(self.kind, forced.hover, forced.active, self.disabled, &theme);
        let hover = button_colors(self.kind, true, forced.active, self.disabled, &theme);
        let pressed = button_colors(self.kind, true, true, self.disabled, &theme);
        let interactive = !self.disabled;
        let ring = forced.focus_visible || self.focus.as_ref().is_some_and(|handle| is_focus_visible(handle, window, cx));

        let fill = inset_layer(0.0, 3.0)
            .bg(base.fill)
            .when(interactive && !forced.active, |layer| layer.group_hover(GROUP, |s| s.bg(hover.fill)))
            .id("fill")
            .when(interactive && !forced.active, |layer| layer.group_active(GROUP, |s| s.bg(pressed.fill)));
        let mut root = div()
            .id(self.id)
            .group(GROUP)
            .relative()
            .flex()
            .flex_row()
            .items_center()
            .justify_center()
            .gap(px(self.gap))
            .h(px(32.0))
            .min_w(px(32.0))
            .border_1()
            .border_color(Color::TRANSPARENT)
            .when(self.icon_only, |b| b.w(px(44.0)))
            .when(!self.icon_only, |b| b.px(px(12.0)))
            .font_text()
            .type_size(14.0, 20.0)
            .text_color(base.text)
            .when(interactive && !forced.active, |b| b.hover(|s| s.text_color(hover.text)).active(|s| s.text_color(pressed.text)))
            .child(fill)
            .children(border_layers(4.0, base.border, false, -1.0))
            .role(Role::Button)
            .when_some(self.aria_label.clone().or(self.label.clone()), |b, label| b.aria_label(label))
            .when_some(self.expanded, |b, expanded| b.aria_expanded(expanded))
            .when(matches!(self.kind, ButtonKind::Toggle(_)), |b| {
                b.aria_toggled(if self.kind == ButtonKind::Toggle(true) { gpui::Toggled::True } else { gpui::Toggled::False })
            })
            .a11y_disabled(self.disabled)
            .when_some(self.icon, |b, (name, size)| {
                // An SVG takes no inherited color: it gets each state's own.
                let glyph = icon(name, size).relative().text_color(base.text);
                b.child(if interactive && !forced.active {
                    glyph
                        .group_hover(GROUP, |s| s.text_color(hover.text))
                        .id("icon")
                        .group_active(GROUP, |s| s.text_color(pressed.text))
                        .into_any_element()
                } else {
                    glyph.into_any_element()
                })
            })
            .when_some(self.label, |b, label| b.child(line(label).relative()))
            .when_some(self.trailing, |b, trailing| b.child(div().relative().flex().child(trailing)))
            .when(self.update_dot, |b| {
                // 7 px accent dot at top 5 / right 7, with a 2 px --bg ring (box-shadow spread).
                b.child(div().absolute().top(px(3.0)).right(px(5.0)).size(px(11.0)).rounded(px(5.5)).bg(theme.bg))
                    .child(div().absolute().top(px(5.0)).right(px(7.0)).size(px(7.0)).rounded(px(3.5)).bg(theme.accent))
            })
            .when(ring, |b| b.child(focus_ring(4.0, &theme)))
            .when_some(self.tooltip, |b, text| b.native_tooltip(text));
        if let Some(handle) = &self.focus {
            if interactive {
                root = root.track_focus(handle);
            }
        }
        if let (Some(handler), true) = (self.on_click, interactive) {
            root = root.on_click(move |event, window, cx| handler(event, window, cx));
        }
        root.refine_style(&self.style)
    }
}

/// `Div::refine_style` for our builders: the caller's style applied over the element's own.
pub(crate) trait RefineStyle: Styled + Sized {
    fn refine_style(mut self, style: &StyleRefinement) -> Self {
        gpui::Refineable::refine(self.style(), style);
        self
    }
}

impl<T: Styled> RefineStyle for T {}
