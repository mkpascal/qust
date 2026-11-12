//! The page's styled scrollbars (`::-webkit-scrollbar`): the playlist's (10 px wide, a
//! `--text-3` thumb inside a 3 px transparent border: 4 px visible, radius 5 on the outer box)
//! and the Settings / Stream information body's (8 px, 2 px border, radius 4). No track, no
//! arrows, no hover style.
//!
//! A [`Scrollbar`] draws the thumb for a GPUI `ScrollHandle` and lets it be dragged. Classic
//! scrollbars take layout space: place the bar in a column of its width at the right of the
//! scroll area, inside its border (the page's list content is narrower by the bar's width when
//! it scrolls).
//!
//! Thumb length: `track × visible / content`, at least the bar's thickness (Chromium's custom
//! scrollbar minimum); position `track_room × offset / max_offset`.

use crate::theme::theme;
use gpui::prelude::FluentBuilder;
use gpui::{
    canvas, div, fill, point, px, size, App, Bounds, DispatchPhase, ElementId, InteractiveElement, IntoElement, MouseButton, MouseDownEvent, MouseMoveEvent,
    MouseUpEvent, ParentElement, Pixels, RenderOnce, ScrollHandle, Styled, Window,
};

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

    fn border(self) -> f32 {
        match self {
            ScrollbarKind::Playlist => 3.0,
            ScrollbarKind::Window => 2.0,
        }
    }

    fn radius(self) -> f32 {
        match self {
            ScrollbarKind::Playlist => 5.0,
            ScrollbarKind::Window => 4.0,
        }
    }
}

/// The thumb's box (the whole thumb, border included) along a track, or None when nothing
/// scrolls: (start, length).
pub fn thumb_span(track: f32, visible: f32, content: f32, offset: f32, thickness: f32) -> Option<(f32, f32)> {
    if content <= visible + 0.5 || track <= 0.0 {
        return None;
    }
    let length = (track * visible / content).max(thickness).min(track);
    let max_offset = content - visible;
    let start = (track - length) * (offset / max_offset).clamp(0.0, 1.0);
    Some((start, length))
}

struct DragState {
    // pointer y and scroll offset at the press
    from: Option<(f32, f32)>,
}

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

impl RenderOnce for Scrollbar {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let color = theme(cx).text_3;
        let kind = self.kind;
        let handle = self.handle.clone();
        let drag = window.use_keyed_state((self.id.clone(), "drag"), cx, |_, _| DragState { from: None });
        let viewport = handle.bounds();
        let max = handle.max_offset().y.as_f32();
        let offset = -handle.offset().y.as_f32();
        let visible = viewport.size.height.as_f32();
        let content = visible + max;
        div().id(self.id).w(px(kind.width())).h_full().flex_none().when(max > 0.5, |bar| {
            bar.child(
                canvas(
                    |_, _, _| {},
                    move |bounds: Bounds<Pixels>, _, window, _| {
                        let track = bounds.size.height.as_f32();
                        let Some((start, length)) = thumb_span(track, visible, content, offset, kind.width()) else { return };
                        let b = kind.border();
                        // background: var(--text-3) padding-box inside a transparent border.
                        let thumb = Bounds::new(
                            point(bounds.origin.x + px(b), bounds.origin.y + px(start + b)),
                            size(px(kind.width() - 2.0 * b), px((length - 2.0 * b).max(0.0))),
                        );
                        window.paint_quad(fill(thumb, color).corner_radii(px((kind.radius() - b).max(0.0))));
                        let drag_move = drag.clone();
                        let handle_move = handle.clone();
                        window.on_mouse_event(move |event: &MouseMoveEvent, phase, window, cx| {
                            if phase != DispatchPhase::Bubble {
                                return;
                            }
                            let Some((y0, offset0)) = drag_move.read(cx).from else { return };
                            let room = (track - length).max(1.0);
                            let next = (offset0 + (event.position.y.as_f32() - y0) * (content - visible) / room).clamp(0.0, content - visible);
                            handle_move.set_offset(point(handle_move.offset().x, px(-next)));
                            window.refresh();
                        });
                        let drag_up = drag.clone();
                        window.on_mouse_event(move |_: &MouseUpEvent, phase, _, cx| {
                            if phase == DispatchPhase::Bubble {
                                drag_up.update(cx, |d, _| d.from = None);
                            }
                        });
                        let drag_down = drag.clone();
                        let thumb_hit = Bounds::new(point(bounds.origin.x, bounds.origin.y + px(start)), size(bounds.size.width, px(length)));
                        window.on_mouse_event(move |event: &MouseDownEvent, phase, _, cx| {
                            if phase == DispatchPhase::Bubble && event.button == MouseButton::Left && thumb_hit.contains(&event.position) {
                                drag_down.update(cx, |d, _| d.from = Some((event.position.y.as_f32(), offset)));
                                cx.stop_propagation();
                            }
                        });
                    },
                )
                .size_full(),
            )
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn thumb() {
        assert_eq!(thumb_span(280.0, 280.0, 280.0, 0.0, 10.0), None);
        let (start, length) = thumb_span(280.0, 280.0, 560.0, 0.0, 10.0).unwrap();
        assert_eq!((start, length), (0.0, 140.0));
        let (start, _) = thumb_span(280.0, 280.0, 560.0, 280.0, 10.0).unwrap();
        assert_eq!(start, 140.0);
        assert_eq!(thumb_span(280.0, 280.0, 280_000.0, 0.0, 10.0).unwrap().1, 10.0);
    }
}
