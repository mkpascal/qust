//! Qust's look on GPUI: the design tokens of the page (style.css, as `reference/layout/tokens.json`
//! recorded them) and the elements every window is built from, each a rebuild of its CSS class
//! value for value. No knowledge of the player.
//!
//! # Conventions for the window crates
//!
//! **Constructing.** Elements are builders that render once (`#[derive(IntoElement)]`,
//! `RenderOnce`): `Button::new("play").icon_only(Icon::Play).accent().disabled(!can_play)`.
//! Those with a size in the layout implement `Styled` (`.flex_1()`, `.w(px(44.))`): the
//! caller's style is applied over the element's own, like a more specific CSS rule. Simple
//! parts are functions returning a `Div` to add children to (`surface(Frame::island(t))`,
//! `list::row(..)`, `section::setting_card(t)`). Everything takes its colors from the
//! [`Theme`] global; nothing is passed colors except where the page computed one from context
//! (the marquee's backdrop, the analyzer's background).
//!
//! **Layout is CSS's.** Bordered boxes keep a transparent 1 px layout border (the visible
//! border is a layer), so the page's `padding`, `gap`, sizes and border-box sizing apply as
//! written, and absolute children are placed from the padding box as in CSS ([`surface`]).
//! Fonts and sizes are the page's (`.font_text()`, `.type_size(13., 18.)`, `.semibold()`,
//! `.tabular_nums()` from [`TextStyles`]).
//!
//! **Events.** Elements report through closures given to the builder: `on_click(|event,
//! window, cx|)`, `Slider::on_input(|value, window, cx|)` / `on_commit`, `Switch::on_change`,
//! `Combo::on_click(|bounds, window, cx|)` (with its bounds, to open a menu under it). The
//! closures run on the UI thread inside GPUI's dispatch; they send player commands through
//! `qust_ui::Player` (which never blocks) and update the owner's own view state.
//!
//! **States.** Hover and pressed come from the pointer (GPUI's hover / active styles; no
//! re-render for them). Every interactive element also takes [`Forced`] (`.forced(Forced::HOVER)`)
//! so captures can show states the way the fixtures force them. Disabled wins over hover and
//! pressed, as in the cascade.
//!
//! **Theme and accent.** `theme(cx)` returns the installed [`Theme`] (dark or light palette,
//! `--accent` resolved per mode, `accent_light` for the analyzer and the clock icon).
//! `Theme::install(theme, cx)` switches it and redraws every window. Read it in `render`; never
//! cache colors in view state.
//!
//! **Focus and tab order.** A focusable element gets a handle from [`focus::tab_stop`] (store
//! it in the view, create it once) and `.focus(&handle)` on the builder; the tab order is paint
//! order, which is the page's DOM order when views keep the HTML's order. [`init`] binds
//! Tab / Shift+Tab; put [`focus::tab_navigation`] on each window's root element. The focus ring
//! follows Chromium's `:focus-visible` rule ([`focus::is_focus_visible`]), not GPUI's.
//! Dialogs keep Tab inside with [`focus::FocusCycle`].
//!
//! **Frequent updates.** GPUI marks a view and all its ancestor views dirty when it notifies
//! (or asks for the next frame), and a dirty view re-renders its whole subtree, cached child
//! views included; views embedded with `.cached(style)` that aren't dirty are replayed from the
//! last frame. So an animating view must not be a descendant of the window's content:
//! - whatever changes often (the analyzer at 60 Hz, the marquee, the equalizers, the clock) is
//!   a **live layer**: a small view the window root (`qust_ui::WindowRoot`) draws as a sibling
//!   of the content, at the place of a slot the content leaves for it, or at window coordinates
//!   it reads from its owner ([`equalizer::EqualizerLayer`]);
//! - the content and every view in it re-render only when what they show changed;
//! - a view observes only the model entities it renders (`cx.observe(&model, |_, _, cx|
//!   cx.notify())`); `qust_ui` splits the player's state into small models for that.
//! - an animation asks for the next frame only while something moves; holds and frozen
//!   captures ask for nothing ([`motion`]).
//!
//! **Accessibility.** Each element carries its HTML counterpart's role and name ([`a11y`]).
//!
//! **Tooltips** are native Windows tooltips (`.native_tooltip(text)`, [`tooltip`]).
//!
//! **GPUI facts learned the hard way** (all in the module docs where they matter): an SVG takes
//! no inherited color (always set `text_color` on an icon); `BoxShadow::blur_radius` is a
//! sigma, half the CSS blur ([`paint::css_shadow`]); content masks are rectangles (rounded
//! clipping needs rounded children); hover styles are suppressed while the last input was the
//! keyboard; keyboard clicks fire on key-up; GPUI has no transforms on divs (the dialog's
//! opening scale is applied to its frame only); GPUI applies opacity per primitive, not per
//! group ([`paint::Fade`]); GPUI snaps halves toward zero where Chromium rounds them up
//! ([`paint::snap_half_up`]).
//!
//! **Text** is laid out as Chromium laid it out where GPUI differs (kerning, baseline, ellipsis,
//! letter-spacing, fallback fonts): see [`text`]. Put single-line text in a [`text::Line`]
//! (`line(t)`), which inherits the text style like a string child.

pub mod a11y;
pub mod analyzer;
pub mod button;
pub mod color;
pub mod combo;
pub mod dialog;
pub mod equalizer;
pub mod focus;
pub mod icons;
pub mod list;
pub mod marquee;
pub mod motion;
pub mod paint;
pub mod scrollbar;
pub mod section;
pub mod slider;
pub mod state;
pub mod surface;
pub mod switch;
pub mod text;
pub mod textbox;
pub mod theme;
pub mod tokens;
pub mod tooltip;

pub use analyzer::Analyzer;
pub use button::{Button, ButtonKind};
pub use color::Color;
pub use combo::{shortcut_keys, Combo, ShortcutBox, ShortcutDisplay};
pub use dialog::DialogCard;
pub use equalizer::{EqualizerLayer, EqualizerPlacement, EqualizerView};
pub use icons::{icon, rotated_icon, Assets, Icon};
pub use marquee::Marquee;
pub use motion::{motion, Motion};
pub use slider::{Mirror, Slider, SliderSize};
pub use state::Forced;
pub use paint::Fade;
pub use surface::{dashed_border, focus_ring, surface, Border, Frame};
pub use switch::Switch;
pub use text::{line, Line, TextStyles};
pub use textbox::{input_state, TextBox};
pub use theme::{theme, Accent, Theme};
pub use tooltip::TooltipExt;

/// Once at startup, after `gpui_component::init`: key bindings for Tab, focus tracking for the
/// focus ring, and a theme (dark with the reference accent until `qust_ui` installs the real
/// one).
pub fn init(cx: &mut gpui::App) {
    focus::init(cx);
    if cx.try_global::<Theme>().is_none() {
        Theme::new(true, Accent::reference()).install(cx);
    }
}
