//! The page's styled scrollbars (`::-webkit-scrollbar`): the playlist's (10 px wide, a
//! `--text-3` thumb inside a 3 px transparent border: 4 px visible, radius 5 on the outer box)
//! and the Settings / Stream information body's (8 px, 2 px border, radius 4). No track, no
//! arrows, no hover style.
//!
//! Chromium's geometry (`ScrollbarTheme`), in whole pixels: the thumb's length is the track ×
//! visible / content **rounded**, at least the bar's thickness; its position is the track room ×
//! offset / max offset **truncated** (1 rather than 0 once scrolled at all) — [`thumb_span`].
//! A press on the track (outside the thumb) pages by 87.5 % of the viewport ([`page_step`])
//! toward the pointer, again after 250 ms and then every 50 ms while the button is held and the
//! thumb hasn't reached the pointer; a press on the thumb drags it.
//!
//! Classic scrollbars take layout space: the content is narrower by the bar's width while it
//! scrolls. [`scroll_area`] is the panels' scrolling body (content plus bar); a list that
//! scrolls by itself (the playlist, virtualized) uses [`thumb_span`], [`page_step`] and
//! [`paint_thumb`] directly.
//!
//! [`Scrollbar`] reads its `ScrollHandle` while it paints, after the scroll area's prepaint of the
//! same frame, so the thumb is right on the very first frame and never a frame behind the content.

use crate::color::Color;
use crate::theme::theme;
use gpui::prelude::FluentBuilder;
use gpui::{
    canvas, div, fill, point, px, size, App, Bounds, DispatchPhase, Div, ElementId, Entity, InteractiveElement, IntoElement, MouseButton, MouseDownEvent,
    MouseMoveEvent, MouseUpEvent, ParentElement, Pixels, RenderOnce, ScrollHandle, StatefulInteractiveElement, Styled, Task, Window,
};
use std::time::Duration;

/// The two scrollbars of the pages.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ScrollbarKind {
    /// `.playlist`: 10 px, 3 px border.
    Playlist,
    /// `body.settings-window`: 8 px, 2 px border.
    Window,
}

impl ScrollbarKind {
    pub fn width(self) -> f32 {
        match self {
            ScrollbarKind::Playlist => 10.0,
            ScrollbarKind::Window => 8.0,
        }
    }

    pub fn border(self) -> f32 {
        match self {
            ScrollbarKind::Playlist => 3.0,
            ScrollbarKind::Window => 2.0,
        }
    }

    pub fn radius(self) -> f32 {
        match self {
            ScrollbarKind::Playlist => 5.0,
            ScrollbarKind::Window => 4.0,
        }
    }
}

/// Chromium's first repeat of a held track press, then the interval between the next ones.
pub const TRACK_REPEAT_FIRST: Duration = Duration::from_millis(250);
pub const TRACK_REPEAT: Duration = Duration::from_millis(50);

/// The thumb's box (border included) along a track, in whole px, or None when nothing scrolls:
/// (start, length). See the module docs.
pub fn thumb_span(track: f32, visible: f32, content: f32, offset: f32, thickness: f32) -> Option<(f32, f32)> {
    if content <= visible + 0.5 || track <= 0.0 {
        return None;
    }
    let length = (track * visible / content).round().max(thickness).min(track);
    let max_offset = content - visible;
    let position = (track - length) * (offset.max(0.0) / max_offset).min(1.0);
    let start = if position > 0.0 && position < 1.0 { 1.0 } else { position.floor() };
    Some((start, length))
}

/// How far a press on the track scrolls: 87.5 % of the viewport, whole px.
pub fn page_step(viewport: f32) -> f32 {
    (viewport * 0.875).floor().max(1.0)
}

/// The scroll offset for a thumb dragged by `dy` px from where it was at `offset`.
pub fn dragged_offset(offset: f32, dy: f32, track: f32, length: f32, max_offset: f32) -> f32 {
    let room = (track - length).max(1.0);
    (offset + dy * max_offset / room).clamp(0.0, max_offset)
}

/// Paints the thumb (`background: var(--text-3) padding-box` inside the transparent border) for
/// a track at `bounds` (the bar's column).
pub fn paint_thumb(bounds: Bounds<Pixels>, kind: ScrollbarKind, start: f32, length: f32, color: Color, window: &mut Window) {
    let b = kind.border();
    let thumb = Bounds::new(point(bounds.origin.x + px(b), bounds.origin.y + px(start + b)), size(px(kind.width() - 2.0 * b), px((length - 2.0 * b).max(0.0))));
    window.paint_quad(fill(thumb, color).corner_radii(px((kind.radius() - b).max(0.0))));
}

#[derive(Default)]
struct BarState {
    // Thumb drag: pointer y and scroll offset at the press.
    drag: Option<(f32, f32)>,
    // Track press: direction (−1 up, 1 down) and the pointer's y, while held.
    track: Option<(f32, f32)>,
    repeat: Option<Task<()>>,
}

/// The bar for a GPUI `ScrollHandle` (the panels' body). Place it in a column of
/// `kind.width()` at the right of the scroll area.
#[derive(IntoElement)]
pub struct Scrollbar {
    id: ElementId,
    handle: ScrollHandle,
    kind: ScrollbarKind,
}

impl Scrollbar {
    pub fn new(id: impl Into<ElementId>, handle: &ScrollHandle, kind: ScrollbarKind) -> Scrollbar {
        Scrollbar { id: id.into(), handle: handle.clone(), kind }
    }
}

// The handle's geometry: (visible, content, offset).
fn geometry(handle: &ScrollHandle) -> (f32, f32, f32) {
    let visible = handle.bounds().size.height.as_f32();
    let max = handle.max_offset().y.as_f32().max(0.0);
    (visible, visible + max, -handle.offset().y.as_f32())
}

fn scroll_to(handle: &ScrollHandle, offset: f32) {
    handle.set_offset(point(handle.offset().x, px(-offset)));
}

// One page toward the pointer; false when the thumb has reached it (or nothing scrolls).
fn page(handle: &ScrollHandle, track: Bounds<Pixels>, kind: ScrollbarKind, direction: f32, y: f32) -> bool {
    let (visible, content, offset) = geometry(handle);
    let Some((start, length)) = thumb_span(track.size.height.as_f32(), visible, content, offset, kind.width()) else { return false };
    let on_track = y - track.origin.y.as_f32();
    let beyond = if direction < 0.0 { on_track < start } else { on_track >= start + length };
    if !beyond {
        return false;
    }
    let next = (offset + direction * page_step(visible)).clamp(0.0, content - visible);
    scroll_to(handle, next);
    true
}

impl RenderOnce for Scrollbar {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let color = theme(cx).text_3;
        let kind = self.kind;
        let handle = self.handle.clone();
        let state: Entity<BarState> = window.use_keyed_state((self.id.clone(), "bar"), cx, |_, _| BarState::default());
        div().id(self.id).w(px(kind.width())).h_full().flex_none().child(
            canvas(
                |_, _, _| {},
                move |bounds: Bounds<Pixels>, _, window, _| {
                    // Read now: the scroll area has been laid out and prepainted this frame.
                    let (visible, content, offset) = geometry(&handle);
                    let track = bounds.size.height.as_f32();
                    let span = thumb_span(track, visible, content, offset, kind.width());
                    if let Some((start, length)) = span {
                        paint_thumb(bounds, kind, start, length, color, window);
                    }
                    // Window-wide while a drag or a track press lasts (the page captured the
                    // pointer); registered on every frame, checked at event time.
                    let (state_move, handle_move) = (state.clone(), handle.clone());
                    window.on_mouse_event(move |event: &MouseMoveEvent, phase, window, cx| {
                        if phase != DispatchPhase::Bubble {
                            return;
                        }
                        let y = event.position.y.as_f32();
                        let (drag, track_press) = {
                            let s = state_move.read(cx);
                            (s.drag, s.track)
                        };
                        if let Some((y0, offset0)) = drag {
                            let (visible, content, _) = geometry(&handle_move);
                            if let Some((_, length)) = thumb_span(track, visible, content, offset0, kind.width()) {
                                scroll_to(&handle_move, dragged_offset(offset0, y - y0, track, length, content - visible));
                                window.refresh();
                            }
                        } else if let Some((direction, _)) = track_press {
                            state_move.update(cx, |s, _| s.track = Some((direction, y)));
                        }
                    });
                    let state_up = state.clone();
                    window.on_mouse_event(move |event: &MouseUpEvent, phase, _, cx| {
                        if phase == DispatchPhase::Bubble && event.button == MouseButton::Left {
                            state_up.update(cx, |s, _| {
                                s.drag = None;
                                s.track = None;
                                s.repeat = None;
                            });
                        }
                    });
                    let (state_down, handle_down) = (state.clone(), handle.clone());
                    window.on_mouse_event(move |event: &MouseDownEvent, phase, window, cx| {
                        if phase != DispatchPhase::Bubble || event.button != MouseButton::Left || !bounds.contains(&event.position) {
                            return;
                        }
                        cx.stop_propagation();
                        let Some((start, length)) = span else { return };
                        let y = event.position.y.as_f32();
                        let on_track = y - bounds.origin.y.as_f32();
                        if on_track >= start && on_track < start + length {
                            state_down.update(cx, |s, _| s.drag = Some((y, offset)));
                            return;
                        }
                        let direction = if on_track < start { -1.0 } else { 1.0 };
                        page(&handle_down, bounds, kind, direction, y);
                        window.refresh();
                        let (state_repeat, handle_repeat) = (state_down.clone(), handle_down.clone());
                        let repeat = window.spawn(cx, async move |cx| {
                            let mut delay = TRACK_REPEAT_FIRST;
                            loop {
                                cx.background_executor().timer(delay).await;
                                delay = TRACK_REPEAT;
                                let more = cx.update(|window, cx| {
                                    let Some((direction, y)) = state_repeat.read(cx).track else { return false };
                                    if page(&handle_repeat, bounds, kind, direction, y) {
                                        window.refresh();
                                    }
                                    true
                                });
                                if !matches!(more, Ok(true)) {
                                    break;
                                }
                            }
                        });
                        state_down.update(cx, |s, _| {
                            s.track = Some((direction, y));
                            s.repeat = Some(repeat);
                        });
                    });
                },
            )
            .size_full(),
        )
    }
}

/// A scrolling body (`body.settings-window { overflow-y: auto }`): the content in a vertical
/// scroll area and, while it overflows, the 8 px bar beside it, taking its width from the
/// content as Chromium's classic scrollbar does (the Settings list is 512 wide, not 520).
///
/// Whether it overflows is known only after layout: `overflow` is the caller's current belief
/// (start with what the content usually does) and `on_measured` is called, during the frame's
/// prepaint, when the layout says otherwise; the caller stores the new value and re-renders.
pub fn scroll_area(id: impl Into<ElementId>, handle: &ScrollHandle, overflow: bool, content: impl IntoElement, on_measured: impl Fn(bool, &mut App) + 'static) -> Div {
    let id: ElementId = id.into();
    let probe_handle = handle.clone();
    div()
        .size_full()
        .relative()
        .flex()
        .flex_row()
        .child(div().id(id.clone()).flex_1().min_w_0().h_full().overflow_y_scroll().track_scroll(handle).child(div().flex_none().w_full().child(content)))
        .when(overflow, |body| body.child(Scrollbar::new((id.clone(), "scrollbar"), handle, ScrollbarKind::Window)))
        // After the scroll area's prepaint (its maximum offset is known): a sibling painted later.
        .child(
            canvas(
                move |_, _, cx| {
                    let laid_out = probe_handle.bounds().size.height > px(0.0);
                    let over = probe_handle.max_offset().y > px(0.5);
                    if laid_out && over != overflow {
                        on_measured(over, cx);
                    }
                },
                |_, _, _, _| {},
            )
            .absolute()
            .size_0(),
        )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn thumb_like_chromium() {
        assert_eq!(thumb_span(280.0, 280.0, 280.0, 0.0, 10.0), None);
        assert_eq!(thumb_span(280.0, 280.0, 560.0, 0.0, 10.0), Some((0.0, 140.0)));
        assert_eq!(thumb_span(280.0, 280.0, 560.0, 280.0, 10.0), Some((140.0, 140.0)));
        assert_eq!(thumb_span(280.0, 280.0, 280_000.0, 0.0, 10.0).unwrap().1, 10.0);
        // The length rounded, the position truncated, and 1 once scrolled at all.
        assert_eq!(thumb_span(282.0, 282.0, 1018.0, 0.0, 10.0), Some((0.0, 78.0)));
        assert_eq!(thumb_span(282.0, 282.0, 1018.0, 0.5, 10.0), Some((1.0, 78.0)));
        assert_eq!(thumb_span(282.0, 282.0, 1018.0, 100.0, 10.0), Some((27.0, 78.0)));
        // The Settings body: 850 of 931.
        assert_eq!(thumb_span(850.0, 850.0, 931.0, 0.0, 8.0), Some((0.0, 776.0)));
        assert_eq!(page_step(282.0), 246.0);
        assert_eq!(page_step(850.0), 743.0);
        assert_eq!(dragged_offset(0.0, 10.0, 282.0, 78.0, 736.0), 36.07843);
    }
}
