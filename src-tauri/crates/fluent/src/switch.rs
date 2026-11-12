//! The Settings toggle (`.toggle`: a label "On"/"Off", then the track; `role="switch"`).
//!
//! Track 40 × 20, 1 px `--text-2` border, radius 10, `--control` fill; thumb 12 px `--text-2`
//! at (3, 3) inside the border. Checked: accent track and border, `--on-accent` thumb moved
//! 20 px. Hovering anywhere on the toggle (label included): thumb 14 px at top 2 and, checked,
//! moved 18 px. `transform 0.15s cubic-bezier(0.1, 0.9, 0.2, 1), width 0.1s ease` (the height
//! and top jump). A click anywhere on it or Space when focused toggles; the focus ring is drawn
//! on the track. Disabled (`.setting.is-disabled`): label and track at opacity 0.45, inert.

use crate::focus::is_focus_visible;
use crate::motion::{motion, timing, Transition, EASE, FLUENT};
use crate::state::Forced;
use crate::surface::focus_ring;
use crate::text::TextStyles;
use crate::theme::theme;
use gpui::prelude::FluentBuilder;
use gpui::{
    div, px, App, ClickEvent, ElementId, FocusHandle, InteractiveElement, IntoElement, ParentElement, RenderOnce, Role, SharedString, StatefulInteractiveElement,
    Styled, Toggled, Window,
};
use std::rc::Rc;

struct SwitchMemory {
    hovered: bool,
    translate: Transition,
    width: Transition,
}

#[derive(IntoElement)]
pub struct Switch {
    id: ElementId,
    checked: bool,
    disabled: bool,
    forced: Forced,
    focus: Option<FocusHandle>,
    label: SharedString,
    on_change: Option<Rc<dyn Fn(bool, &mut Window, &mut App)>>,
}

impl Switch {
    /// `label` names it for screen readers (the setting's title); the visible text is On / Off.
    pub fn new(id: impl Into<ElementId>, checked: bool, label: impl Into<SharedString>) -> Switch {
        Switch { id: id.into(), checked, disabled: false, forced: Forced::NONE, focus: None, label: label.into(), on_change: None }
    }

    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }

    pub fn forced(mut self, forced: Forced) -> Self {
        self.forced = forced;
        self
    }

    pub fn focus(mut self, handle: &FocusHandle) -> Self {
        self.focus = Some(handle.clone());
        self
    }

    /// Called with the new value (the page's `change` event).
    pub fn on_change(mut self, handler: impl Fn(bool, &mut Window, &mut App) + 'static) -> Self {
        self.on_change = Some(Rc::new(handler));
        self
    }
}

impl RenderOnce for Switch {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = *theme(cx);
        let motion = motion(cx);
        let checked = self.checked;
        let memory = window.use_keyed_state(self.id.clone(), cx, |_, _| SwitchMemory {
            hovered: false,
            translate: Transition::new(if checked { 20.0 } else { 0.0 }, timing::TOGGLE_THUMB, FLUENT),
            width: Transition::new(12.0, timing::TOGGLE_WIDTH, EASE),
        });
        let hover = !self.disabled && (self.forced.hover || memory.read(cx).hovered);
        let (translate, width, running) = memory.update(cx, |m, _| {
            m.translate.set(match (checked, hover) {
                (true, true) => 18.0,
                (true, false) => 20.0,
                _ => 0.0,
            }, &motion);
            m.width.set(if hover { 14.0 } else { 12.0 }, &motion);
            (m.translate.value(&motion) as f32, m.width.value(&motion) as f32, m.translate.running(&motion) || m.width.running(&motion))
        });
        if running {
            window.request_animation_frame();
        }
        let (track_fill, track_border, thumb) = if checked { (theme.accent, theme.accent, theme.on_accent) } else { (theme.control, theme.text_2, theme.text_2) };
        // `top: 3px` and `height: 12px`, or 2 and 14 while hovered (they jump; only the width
        // transitions). Absolute positions start inside the 1 px border, as in CSS.
        let (top, height) = if hover { (2.0, 14.0) } else { (3.0, 12.0) };
        let ring = self.forced.focus_visible || self.focus.as_ref().is_some_and(|handle| is_focus_visible(handle, window, cx));

        let track = div()
            .relative()
            .w(px(40.0))
            .h(px(20.0))
            .rounded(px(10.0))
            .bg(track_fill)
            .border_1()
            .border_color(track_border)
            .child(div().absolute().top(px(top)).left(px(3.0 + translate)).w(px(width)).h(px(height)).rounded(px(height.max(width) / 2.0)).bg(thumb))
            .when(ring, |t| t.child(focus_ring(10.0, &theme)));

        let interactive = !self.disabled;
        let mut root = div()
            .id(self.id.clone())
            .flex()
            .flex_row()
            .items_center()
            .gap(px(12.0))
            .font_text()
            .type_size(14.0, 20.0)
            .text_color(theme.text)
            .when(self.disabled, |r| r.opacity(0.45))
            .role(Role::Switch)
            .aria_label(self.label.clone())
            .aria_toggled(if checked { Toggled::True } else { Toggled::False })
            .child(div().child(if checked { "On" } else { "Off" }))
            .child(track);
        if interactive {
            let memory = memory.clone();
            root = root.on_hover(move |hovered, _, cx| {
                memory.update(cx, |m, cx| {
                    m.hovered = *hovered;
                    cx.notify();
                })
            });
            if let Some(handle) = &self.focus {
                root = root.track_focus(handle);
            }
            if let Some(handler) = self.on_change {
                root = root.on_click(move |event: &ClickEvent, window, cx| {
                    // A checkbox toggles on Space, not Enter.
                    if let ClickEvent::Keyboard(key) = event {
                        if key.button == gpui::KeyboardButton::Enter {
                            return;
                        }
                    }
                    handler(!checked, window, cx)
                });
            }
        }
        root
    }
}
