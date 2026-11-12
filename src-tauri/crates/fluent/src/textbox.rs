//! The text box (`.textbox`): our frame around GPUI Kit's `Input`, which does the editing (caret,
//! selection, IME, clipboard, undo).
//!
//! 32 tall, padding 0 10, radius 4, 1 px `--control-stroke` border with an
//! `--input-stroke-bottom` bottom edge, `--control` fill â€” under the border too (`.textbox` has no
//! `background-clip`) â€” 14/20 `--text`, placeholder `--text-3`. Hover `--control-hover`.
//! Focused: `--input-focus` fill and the accent underline (accent bottom border plus an inset
//! `0 -1px 0` accent line: 2 px), never a focus ring. Read-only (a local file's path): no fill,
//! no underline (all sides `--control-stroke`), `--text-2` text.
//!
//! The owner keeps the `Entity<InputState>` (make it with [`input_state`], which turns off GPUI
//! Kit's own right-click menu: it uses `TPM_NONOTIFY` and freezes rendering while open). The
//! edit menu (Cut / Copy / Paste / Select All) is `qust-shell`'s; hook it up with
//! [`TextBox::on_context_menu`].

use crate::button::RefineStyle;
use crate::color::Color;
use crate::state::Forced;
use crate::surface::{border_layers, inset_layer, Border};
use crate::text::{TextStyles, FONT_TEXT};
use crate::theme::{theme, Theme};
use gpui::prelude::FluentBuilder;
use gpui::Focusable;
use gpui::{
    div, px, App, AppContext, ElementId, Entity, InteractiveElement, IntoElement, MouseButton, MouseDownEvent, ParentElement, Point, Pixels, RenderOnce,
    SharedString, StyleRefinement, Styled, Window,
};
use gpui_component::input::{Input, InputState};
use std::rc::Rc;

/// GPUI Kit's theme colors the `Input` reads, from ours: text, placeholder (`--text-3`), caret
/// (the text color, 1 px), selection (Chromium's default: the system highlight `#0078d7`
/// behind the text; not in the reference renders). Called by `Theme::install`.
pub fn sync_component_theme(theme: &Theme, cx: &mut App) {
    if cx.try_global::<gpui_component::Theme>().is_none() {
        return;
    }
    let mode = if theme.dark { gpui_component::ThemeMode::Dark } else { gpui_component::ThemeMode::Light };
    gpui_component::Theme::change(mode, None, cx);
    let (text, placeholder, background) = (theme.text, theme.text_3, theme.bg);
    let selection = Color::rgba(0, 120, 215, 0.5);
    let theme_mut = gpui_component::Theme::global_mut(cx);
    theme_mut.font_family = FONT_TEXT.into();
    theme_mut.font_size = px(14.0);
    theme_mut.colors.foreground = text.into();
    theme_mut.colors.muted_foreground = placeholder.into();
    theme_mut.colors.caret = text.into();
    theme_mut.colors.selection = selection.into();
    theme_mut.colors.background = background.into();
    theme_mut.colors.input = Color::TRANSPARENT.into();
}

/// An `InputState` for a [`TextBox`]: single line, GPUI Kit's context menu off.
pub fn input_state(placeholder: impl Into<SharedString>, value: impl Into<SharedString>, window: &mut Window, cx: &mut App) -> Entity<InputState> {
    let placeholder = placeholder.into();
    let value = value.into();
    cx.new(|cx| InputState::new(window, cx).placeholder(placeholder).default_value(value).context_menu(false))
}

type MenuHandler = Rc<dyn Fn(Point<Pixels>, &mut Window, &mut App)>;

#[derive(IntoElement)]
pub struct TextBox {
    id: ElementId,
    state: Entity<InputState>,
    readonly: bool,
    forced: Forced,
    label: Option<SharedString>,
    on_context_menu: Option<MenuHandler>,
    style: StyleRefinement,
}

impl TextBox {
    pub fn new(id: impl Into<ElementId>, state: &Entity<InputState>) -> TextBox {
        TextBox { id: id.into(), state: state.clone(), readonly: false, forced: Forced::NONE, label: None, on_context_menu: None, style: StyleRefinement::default() }
    }

    /// `readonly` (the Edit track dialog's file path).
    pub fn readonly(mut self, readonly: bool) -> Self {
        self.readonly = readonly;
        self
    }

    pub fn forced(mut self, forced: Forced) -> Self {
        self.forced = forced;
        self
    }

    /// The field's label (`<label class="field">`), its accessible name.
    pub fn aria_label(mut self, label: impl Into<SharedString>) -> Self {
        self.label = Some(label.into());
        self
    }

    /// Right-click: the field is focused first, then this runs with the window position.
    pub fn on_context_menu(mut self, handler: impl Fn(Point<Pixels>, &mut Window, &mut App) + 'static) -> Self {
        self.on_context_menu = Some(Rc::new(handler));
        self
    }
}

impl Styled for TextBox {
    fn style(&mut self) -> &mut StyleRefinement {
        &mut self.style
    }
}

const GROUP: &str = "qust-textbox";

impl RenderOnce for TextBox {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = *theme(cx);
        let focus = self.state.read(cx).focus_handle(cx);
        let focused = self.forced.focus || focus.is_focused(window);
        let (fill, border, underline, text) = if self.readonly {
            (None, Border::all(theme.control_stroke), false, theme.text_2)
        } else if focused {
            (Some(theme.input_focus), Border::bottom(theme.control_stroke, theme.accent), true, theme.text)
        } else {
            let fill = if self.forced.hover { theme.control_hover } else { theme.control };
            (Some(fill), Border::bottom(theme.control_stroke, theme.input_stroke_bottom), false, theme.text)
        };
        let hoverable = !self.readonly && !focused && !self.forced.hover;
        let state = self.state.clone();
        let on_menu = self.on_context_menu.clone();
        div()
            .id(self.id)
            .group(GROUP)
            .relative()
            .flex()
            .items_center()
            .h(px(32.0))
            .border_1()
            .border_color(Color::TRANSPARENT)
            // No background-clip: the fill goes under the border.
            .when_some(fill, |d, fill| {
                d.child(inset_layer(-1.0, 4.0).bg(fill).when(hoverable, |layer| layer.group_hover(GROUP, |s| s.bg(theme.control_hover))))
            })
            .children(border_layers(4.0, border, false, -1.0))
            // box-shadow: inset 0 -1px 0 accent: the second underline row, inside the border.
            .when(underline, |d| d.child(div().absolute().left_0().right_0().bottom_0().h(px(1.0)).bg(theme.accent)))
            .child(
                Input::new(&self.state)
                    .appearance(false)
                    .readonly(self.readonly)
                    .when_some(self.label, |input, label| input.aria_label(label))
                    .relative()
                    .w_full()
                    .h(px(30.0))
                    .px(px(10.0))
                    .py(px(0.0))
                    .font_text()
                    .type_size(14.0, 20.0)
                    .text_color(text),
            )
            .on_mouse_down(MouseButton::Right, move |event: &MouseDownEvent, window, cx| {
                let handle = state.read(cx).focus_handle(cx);
                handle.focus(window, cx);
                if let Some(handler) = &on_menu {
                    handler(event.position, window, cx);
                }
            })
            .refine_style(&self.style)
    }
}
