//! The slider shared by volume, balance and the seek bar (`.slider`, `makeSlider` in script.js).
//!
//! Look (style.css "Sliders"): the 24 px tall box; a 4 px rail (radius 2, `--slider-track`) at
//! its vertical middle, inset 10 px left and right (8 / 8 for the side-by-side pair, 8 left / 10
//! right for the seek bar); the accent fill from `min(origin, value)` to `max(origin, value)`
//! (origin: the left end, or the middle for balance); balance's 1 × 12 px centre tick; the 20 px
//! thumb (16 px in the pair) in `--thumb` with a 1 px `--control-stroke` ring and `0 1px 2px
//! rgba(0,0,0,.25)`, and its accent dot inset 4 px, scaled 1.167 while the slider is hovered and
//! 0.833 while dragged (`transform 0.1s ease`). Disabled: fill and dot `--text-disabled`.
//!
//! Behaviour (BEHAVIOUR.md 2.3.3), exactly: integers, rounded and clamped. Pointer: primary
//! button down anywhere on the box (unless disabled) starts a drag that follows the pointer
//! everywhere (GPUI captures the mouse) and commits on release; within 4 % of the span of the
//! centre a pointer value snaps to it (`snap`). Double-click: back to the centre when there is
//! one. Wheel: ±5 % of the span then commit, and never scrolls anything. Keys when focused:
//! Right/Up +5 %, Left/Down −5 %, PageUp +10 %, PageDown −10 %, Home min, End max, then commit.
//! While dragging, values from the owner are ignored; otherwise a new value from the owner
//! replaces the shown one.
//!
//! Events: `on_input(value)` for every change of the shown value (volume and balance send their
//! command there; labels follow it, the seek time too while dragging), `on_commit(value)` at the
//! end of a drag and after a wheel or key step (the seek bar seeks there).

use crate::a11y::A11yExt;
use crate::color::Color;
use crate::focus::is_focus_visible;
use crate::motion::{motion, timing, Transition, EASE};
use crate::paint::css_shadow;
use crate::state::Forced;
use crate::surface::outline;
use crate::theme::theme;
use crate::tooltip::TooltipExt;
use gpui::prelude::FluentBuilder;
use gpui::{
    canvas, div, fill, point, px, size, App, Bounds, Corners, DispatchPhase, ElementId, FocusHandle, InteractiveElement, IntoElement, MouseButton,
    MouseDownEvent, MouseMoveEvent, MouseUpEvent, ParentElement, Pixels, RenderOnce, Role, ScrollDelta, SharedString, StatefulInteractiveElement,
    StyleRefinement, Styled, Window,
};
use std::rc::Rc;

/// Thumb size and rail insets.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SliderSize {
    /// Volume and balance on their own rows: thumb 20, rail inset 10 / 10.
    Regular,
    /// Volume and balance side by side under the seek bar: thumb 16, rail inset 8 / 8.
    Pair,
    /// The seek bar: thumb 20, rail inset 8 left / 10 right.
    Seek,
}

impl SliderSize {
    fn thumb(self) -> f32 {
        if self == SliderSize::Pair {
            16.0
        } else {
            20.0
        }
    }

    fn insets(self) -> (f32, f32) {
        match self {
            SliderSize::Regular => (10.0, 10.0),
            SliderSize::Pair => (8.0, 8.0),
            SliderSize::Seek => (8.0, 10.0),
        }
    }
}

type ValueHandler = Rc<dyn Fn(i64, &mut Window, &mut App)>;

/// What a slider remembers between frames (element state, keyed by its id).
pub struct SliderMemory {
    shown: i64,
    last_owner_value: i64,
    dragging: bool,
    hovered: bool,
    dot: Transition,
    // Where the box was laid out last frame, to map a mouse-down to a value.
    bounds: Option<Bounds<Pixels>>,
}

impl SliderMemory {
    pub fn shown(&self) -> i64 {
        self.shown
    }

    pub fn dragging(&self) -> bool {
        self.dragging
    }
}

#[derive(IntoElement)]
pub struct Slider {
    id: ElementId,
    value: i64,
    min: i64,
    max: i64,
    center: Option<i64>,
    size: SliderSize,
    tick: bool,
    snap: bool,
    disabled: bool,
    forced: Forced,
    forced_dragging: bool,
    focus: Option<FocusHandle>,
    label: SharedString,
    tooltip: Option<SharedString>,
    on_input: Option<ValueHandler>,
    on_commit: Option<ValueHandler>,
    style: StyleRefinement,
}

impl Slider {
    /// A slider from `min` to `max` showing `value`, named `label` for screen readers.
    pub fn new(id: impl Into<ElementId>, value: i64, min: i64, max: i64, label: impl Into<SharedString>) -> Slider {
        Slider {
            id: id.into(),
            value,
            min,
            max,
            center: None,
            size: SliderSize::Regular,
            tick: false,
            snap: false,
            disabled: false,
            forced: Forced::NONE,
            forced_dragging: false,
            focus: None,
            label: label.into(),
            tooltip: None,
            on_input: None,
            on_commit: None,
            style: StyleRefinement::default(),
        }
    }

    /// `.is-centered`: the fill starts at `center`, a tick marks it, the pointer snaps to it and
    /// a double-click returns to it (balance).
    pub fn centered(mut self, center: i64) -> Self {
        self.center = Some(center);
        self.tick = true;
        self.snap = true;
        self
    }

    pub fn size(mut self, size: SliderSize) -> Self {
        self.size = size;
        self
    }

    /// `aria-disabled="true"`: ignores pointer, wheel and keys, leaves the tab order.
    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }

    pub fn forced(mut self, forced: Forced) -> Self {
        self.forced = forced;
        self
    }

    /// Shows the dragging look (`.is-dragging`) without a drag, for captures.
    pub fn forced_dragging(mut self, dragging: bool) -> Self {
        self.forced_dragging = dragging;
        self
    }

    pub fn focus(mut self, handle: &FocusHandle) -> Self {
        self.focus = Some(handle.clone());
        self
    }

    pub fn tooltip(mut self, text: impl Into<SharedString>) -> Self {
        self.tooltip = Some(text.into());
        self
    }

    pub fn on_input(mut self, handler: impl Fn(i64, &mut Window, &mut App) + 'static) -> Self {
        self.on_input = Some(Rc::new(handler));
        self
    }

    pub fn on_commit(mut self, handler: impl Fn(i64, &mut Window, &mut App) + 'static) -> Self {
        self.on_commit = Some(Rc::new(handler));
        self
    }
}

impl Styled for Slider {
    fn style(&mut self) -> &mut StyleRefinement {
        &mut self.style
    }
}

/// The pure part of `makeSlider`: what a change does to the value.
#[derive(Clone, Copy, Debug)]
pub struct Range {
    pub min: i64,
    pub max: i64,
    pub center: Option<i64>,
}

impl Range {
    fn span(&self) -> f64 {
        (self.max - self.min) as f64
    }

    /// `clamp()`: rounded (half up, like `Math.round`) and clamped.
    pub fn clamp(&self, value: f64) -> i64 {
        let value = value.max(self.min as f64).min(self.max as f64);
        (value + 0.5).floor() as i64
    }

    /// `set(next, snap)` without the side effects.
    pub fn set(&self, value: f64, snap: bool) -> i64 {
        let next = self.clamp(value);
        match self.center {
            Some(center) if snap && ((next - center) as f64).abs() <= self.span() * 0.04 => center,
            _ => next,
        }
    }

    /// The value at pointer `x` over a rail from `left` of `width`.
    pub fn from_pointer(&self, x: f32, left: f32, width: f32, snap: bool) -> i64 {
        self.set(self.min as f64 + ((x - left) / width) as f64 * self.span(), snap)
    }

    /// A key's new value (None for keys the slider ignores).
    pub fn key(&self, current: i64, key: &str) -> Option<i64> {
        let step = match key {
            "right" | "up" => 0.05,
            "left" | "down" => -0.05,
            "pageup" => 0.1,
            "pagedown" => -0.1,
            "home" => return Some(self.min),
            "end" => return Some(self.max),
            _ => return None,
        };
        Some(self.set(current as f64 + step * self.span(), false))
    }

    /// A wheel notch (`up`: away from the user).
    pub fn wheel(&self, current: i64, up: bool) -> i64 {
        self.set(current as f64 + if up { 1.0 } else { -1.0 } * self.span() * 0.05, false)
    }

    fn ratio(&self, value: i64) -> f32 {
        ((value - self.min) as f64 / self.span()) as f32
    }
}

impl RenderOnce for Slider {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = *theme(cx);
        let motion = motion(cx);
        let range = Range { min: self.min, max: self.max, center: self.center };
        let owner_value = range.clamp(self.value as f64);
        let memory = window.use_keyed_state(self.id.clone(), cx, |_, _| SliderMemory {
            shown: owner_value,
            last_owner_value: owner_value,
            dragging: false,
            hovered: false,
            dot: Transition::new(1.0, timing::SLIDER_DOT, EASE),
            bounds: None,
        });
        // A new value from the owner replaces the shown one, unless the user is dragging.
        let (shown, _dragging, hovered, dot_scale, dot_running) = memory.update(cx, |m, _| {
            if owner_value != m.last_owner_value {
                m.last_owner_value = owner_value;
                if !m.dragging {
                    m.shown = owner_value;
                }
            }
            let dragging = m.dragging || self.forced_dragging;
            let hovered = m.hovered || self.forced.hover;
            m.dot.set(if dragging { 0.833 } else if hovered { 1.167 } else { 1.0 }, &motion);
            (m.shown, dragging, hovered, m.dot.value(&motion), m.dot.running(&motion))
        });
        let _ = hovered;
        if dot_running {
            window.request_animation_frame();
        }

        let interactive = !self.disabled;
        let ring = self.forced.focus_visible || self.focus.as_ref().is_some_and(|handle| is_focus_visible(handle, window, cx));
        let (fill_color, dot_color) = if self.disabled { (theme.text_disabled, theme.text_disabled) } else { (theme.accent, theme.accent) };
        let paint = SliderPaint {
            ratio: range.ratio(shown),
            origin: self.center.map_or(0.0, |center| range.ratio(center)),
            size: self.size,
            tick: self.tick,
            track: theme.slider_track,
            fill: fill_color,
            thumb: theme.thumb,
            ring: theme.control_stroke,
            dot: dot_color,
            dot_scale: dot_scale as f32,
        };

        let snap = self.snap;
        let on_input = self.on_input.clone();
        let on_commit = self.on_commit.clone();
        let focus = self.focus.clone();
        let memory_for_paint = memory.clone();

        let mut root = div()
            .id(self.id.clone())
            .relative()
            .h(px(24.0))
            .rounded(px(4.0))
            .role(Role::Slider)
            .aria_label(self.label.clone())
            .aria_numeric_value(shown as f64)
            .aria_min_numeric_value(self.min as f64)
            .aria_max_numeric_value(self.max as f64)
            .a11y_disabled(self.disabled)
            .child(
                canvas(
                    |_, _, _| {},
                    move |bounds, _, window, _| {
                        paint.paint(bounds, window);
                        if !interactive {
                            return;
                        }
                        // The drag follows the pointer everywhere until the button is released.
                        // The listeners are there on every frame (they check the drag at event
                        // time): a press and its release can come before the next frame.
                        let (left, right) = paint.size.insets();
                        let rail_left = bounds.origin.x.as_f32() + left;
                        let rail_width = bounds.size.width.as_f32() - left - right;
                        let memory = memory_for_paint.clone();
                        let on_input = on_input.clone();
                        window.on_mouse_event(move |event: &MouseMoveEvent, phase, window, cx| {
                            if phase != DispatchPhase::Bubble || !memory.read(cx).dragging {
                                return;
                            }
                            let next = range.from_pointer(event.position.x.as_f32(), rail_left, rail_width, snap);
                            change(&memory, next, on_input.as_ref(), window, cx);
                        });
                        let memory = memory_for_paint.clone();
                        let on_commit = on_commit.clone();
                        window.on_mouse_event(move |event: &MouseUpEvent, phase, window, cx| {
                            if phase != DispatchPhase::Bubble || event.button != MouseButton::Left || !memory.read(cx).dragging {
                                return;
                            }
                            let value = memory.update(cx, |m, cx| {
                                m.dragging = false;
                                cx.notify();
                                m.shown
                            });
                            if let Some(handler) = &on_commit {
                                handler(value, window, cx);
                            }
                        });
                    },
                )
                .absolute()
                .inset_0(),
            )
            .when(ring, |b| b.child(outline(4.0, 0.0, 1.0, 2.0, theme.focus)))
            .when_some(self.tooltip, |b, text| b.native_tooltip(text));

        {
            let memory = memory.clone();
            root = root.on_hover(move |hovered, _, cx| {
                memory.update(cx, |m, cx| {
                    if m.hovered != *hovered {
                        m.hovered = *hovered;
                        cx.notify();
                    }
                })
            });
        }

        if interactive {
            if let Some(handle) = &focus {
                root = root.track_focus(handle);
            }
            let (left, right) = self.size.insets();
            let memory_down = memory.clone();
            let on_input_down = self.on_input.clone();
            let focus_down = focus.clone();
            root = root.on_mouse_down(MouseButton::Left, move |event: &MouseDownEvent, window, cx| {
                if let Some(handle) = &focus_down {
                    handle.focus(window, cx);
                }
                // The rail's position comes from where the box was laid out last frame.
                let Some(bounds) = memory_down.read(cx).bounds() else { return };
                memory_down.update(cx, |m, cx| {
                    m.dragging = true;
                    cx.notify();
                });
                let rail_left = bounds.origin.x.as_f32() + left;
                let rail_width = bounds.size.width.as_f32() - left - right;
                let next = range.from_pointer(event.position.x.as_f32(), rail_left, rail_width, snap);
                change(&memory_down, next, on_input_down.as_ref(), window, cx);
                if event.click_count == 2 {
                    if let Some(center) = range.center {
                        change(&memory_down, center, on_input_down.as_ref(), window, cx);
                    }
                }
                cx.stop_propagation();
            });
            let memory_wheel = memory.clone();
            let on_input_wheel = self.on_input.clone();
            let on_commit_wheel = self.on_commit.clone();
            root = root.on_scroll(move |event, window, cx| {
                cx.stop_propagation();
                let dy = match event.delta {
                    ScrollDelta::Pixels(delta) => delta.y.as_f32(),
                    ScrollDelta::Lines(delta) => delta.y,
                };
                if dy == 0.0 {
                    return;
                }
                let current = memory_wheel.read(cx).shown;
                change(&memory_wheel, range.wheel(current, dy > 0.0), on_input_wheel.as_ref(), window, cx);
                if let Some(handler) = &on_commit_wheel {
                    handler(memory_wheel.read(cx).shown, window, cx);
                }
            });
            let memory_key = memory.clone();
            let on_input_key = self.on_input.clone();
            let on_commit_key = self.on_commit.clone();
            root = root.on_key_down(move |event, window, cx| {
                let current = memory_key.read(cx).shown;
                let Some(next) = range.key(current, &event.keystroke.key) else { return };
                if event.keystroke.modifiers.modified() {
                    return;
                }
                change(&memory_key, next, on_input_key.as_ref(), window, cx);
                if let Some(handler) = &on_commit_key {
                    handler(memory_key.read(cx).shown, window, cx);
                }
                cx.stop_propagation();
            });
        } else {
            // A disabled slider still swallows the wheel (`preventDefault` came first).
            root = root.on_scroll(|_, _, cx| cx.stop_propagation());
        }
        let memory_bounds = memory.clone();
        root.child(canvas(move |bounds, _, cx| memory_bounds.update(cx, |m, _| m.set_bounds(bounds)), |_, _, _, _| {}).absolute().inset_0())
            .refine_style(&self.style)
    }
}

use crate::button::RefineStyle;

impl SliderMemory {
    fn set_bounds(&mut self, bounds: Bounds<Pixels>) {
        self.bounds = Some(bounds);
    }

    fn bounds(&self) -> Option<Bounds<Pixels>> {
        self.bounds
    }
}

fn change(memory: &gpui::Entity<SliderMemory>, next: i64, on_input: Option<&ValueHandler>, window: &mut Window, cx: &mut App) {
    let changed = memory.update(cx, |m, cx| {
        if m.shown == next {
            return false;
        }
        m.shown = next;
        cx.notify();
        true
    });
    if changed {
        if let Some(handler) = on_input {
            handler(next, window, cx);
        }
    }
}

/// The slider's pixels, from its box.
#[derive(Clone, Copy)]
struct SliderPaint {
    ratio: f32,
    origin: f32,
    size: SliderSize,
    tick: bool,
    track: Color,
    fill: Color,
    thumb: Color,
    ring: Color,
    dot: Color,
    dot_scale: f32,
}

impl SliderPaint {
    fn paint(&self, bounds: Bounds<Pixels>, window: &mut Window) {
        let (left, right) = self.size.insets();
        let x0 = bounds.origin.x.as_f32() + left;
        let width = bounds.size.width.as_f32() - left - right;
        // top: 50%; margin-top: -2px; height: 4px
        let rail_top = bounds.origin.y.as_f32() + bounds.size.height.as_f32() / 2.0 - 2.0;
        let rect = |x: f32, y: f32, w: f32, h: f32| Bounds::new(point(px(x), px(y)), size(px(w), px(h)));
        window.paint_quad(fill(rect(x0, rail_top, width, 4.0), self.track).corner_radii(px(2.0)));
        if self.tick {
            // .slider-rail::before: 1 × 12 at left 50%, top −4.
            window.paint_quad(fill(rect(x0 + width * 0.5, rail_top - 4.0, 1.0, 12.0), self.track));
        }
        let from = self.origin.min(self.ratio);
        let to = self.origin.max(self.ratio);
        if to > from {
            window.paint_quad(fill(rect(x0 + from * width, rail_top, (to - from) * width, 4.0), self.fill).corner_radii(px(2.0)));
        }
        let thumb = self.size.thumb();
        let cx = x0 + self.ratio * width;
        let cy = rail_top + 2.0;
        let thumb_bounds = rect(cx - thumb / 2.0, cy - thumb / 2.0, thumb, thumb);
        let radius = Corners::all(px(thumb / 2.0));
        // box-shadow: 0 0 0 1px var(--control-stroke), 0 1px 2px rgba(0,0,0,.25); the first
        // listed paints on top.
        window.paint_drop_shadows(thumb_bounds, radius, &[css_shadow(0.0, 1.0, 2.0, 0.0, Color::rgba(0, 0, 0, 64.0 / 255.0))]);
        window.paint_drop_shadows(thumb_bounds, radius, &[css_shadow(0.0, 0.0, 0.0, 1.0, self.ring)]);
        window.paint_quad(fill(thumb_bounds, self.thumb).corner_radii(px(thumb / 2.0)));
        // ::after { inset: 4px } scaled about its centre.
        let dot = (thumb - 8.0) * self.dot_scale;
        window.paint_quad(fill(rect(cx - dot / 2.0, cy - dot / 2.0, dot, dot), self.dot).corner_radii(px(dot / 2.0)));
    }
}
