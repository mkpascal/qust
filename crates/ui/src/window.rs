//! Every window's root: a thin view that stacks layers, so what animates at 60 Hz never makes
//! the window's content re-render.
//!
//! GPUI re-renders, on every frame it draws, the window's root view and every view marked dirty;
//! a view that notifies marks all its ancestor views dirty too. Views embedded with
//! `.cached(..)` that aren't dirty are replayed from the last frame. So [`WindowRoot`] renders
//! only:
//! 1. the window background (`--bg`),
//! 2. the content view, cached (the window crate's real UI: islands, cartridges, rows…),
//! 3. **live layers**: small views that change often (the analyzer at 60 Hz, the marquee, the
//!    clock, the seek row), each drawn at the bounds of a [`portal_slot`] that the content left
//!    for it. They are siblings of the content, not descendants: when one notifies, only it and
//!    this root re-render, never the content,
//! 4. overlays (the dialog with its backdrop, a drag ghost), above everything.
//!
//! A slot records where it was laid out; when that changes (a resize, a scale change) the
//! root redraws once more with the layer at the new place.
//!
//! **Focus.** In the browser a fresh window (or one whose focused control went away) has the
//! document focused, and the first Tab moves to the first control. In GPUI nothing is focused
//! then, and a key event only reaches the root node of the dispatch tree, which has no Tab
//! handler: the root takes the focus itself whenever nothing has it (its handle is not a tab
//! stop), so Tab and Shift+Tab start from the beginning and the end, as in the browser.

use gpui::{
    canvas, div, px, AnyView, App, AppContext, Bounds, Context, Entity, FocusHandle, InteractiveElement, IntoElement, ParentElement, Pixels, Render,
    SharedString, StyleRefinement, Styled, WeakEntity, Window,
};
use qust_fluent::theme;

struct Live {
    key: SharedString,
    view: AnyView,
    bounds: Option<Bounds<Pixels>>,
}

pub struct WindowRoot {
    content: AnyView,
    live: Vec<Live>,
    overlays: Vec<(SharedString, AnyView)>,
    /// Focused whenever nothing else is (see the module docs).
    focus: Option<FocusHandle>,
    /// Renders of this root (measurements and tests).
    pub renders: u64,
}

impl WindowRoot {
    pub fn new(content: impl Into<AnyView>) -> WindowRoot {
        WindowRoot { content: content.into(), live: Vec::new(), overlays: Vec::new(), focus: None, renders: 0 }
    }

    pub fn content(&self) -> &AnyView {
        &self.content
    }

    /// Replaces the content view (live layers and overlays stay).
    pub fn set_content(&mut self, content: impl Into<AnyView>, cx: &mut Context<Self>) {
        self.content = content.into();
        cx.notify();
    }

    /// Adds (or replaces) the live layer `key`, drawn at the slot of the same key.
    pub fn set_live(&mut self, key: impl Into<SharedString>, view: impl Into<AnyView>, cx: &mut Context<Self>) {
        let key = key.into();
        let view = view.into();
        match self.live.iter_mut().find(|live| live.key == key) {
            Some(live) => live.view = view,
            None => self.live.push(Live { key, view, bounds: None }),
        }
        cx.notify();
    }

    pub fn remove_live(&mut self, key: &str, cx: &mut Context<Self>) {
        self.live.retain(|live| live.key != key);
        cx.notify();
    }

    /// Shows (or replaces) overlay `key` above everything (the dialog: give it the whole
    /// window; it draws its own backdrop).
    pub fn set_overlay(&mut self, key: impl Into<SharedString>, view: impl Into<AnyView>, cx: &mut Context<Self>) {
        let key = key.into();
        let view = view.into();
        match self.overlays.iter_mut().find(|(k, _)| *k == key) {
            Some((_, current)) => *current = view,
            None => self.overlays.push((key, view)),
        }
        cx.notify();
    }

    pub fn remove_overlay(&mut self, key: &str, cx: &mut Context<Self>) {
        let before = self.overlays.len();
        self.overlays.retain(|(k, _)| k != key);
        if self.overlays.len() != before {
            cx.notify();
        }
    }

    pub fn has_overlay(&self, key: &str) -> bool {
        self.overlays.iter().any(|(k, _)| k == key)
    }

    fn slot_moved(&mut self, key: &SharedString, bounds: Bounds<Pixels>, cx: &mut Context<Self>) {
        if let Some(live) = self.live.iter_mut().find(|live| &live.key == key) {
            if live.bounds != Some(bounds) {
                live.bounds = Some(bounds);
                cx.notify();
            }
        }
    }
}

impl Render for WindowRoot {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.renders += 1;
        let focus = self.focus.get_or_insert_with(|| cx.focus_handle()).clone();
        if window.focused(cx).is_none() {
            let root = focus.clone();
            window.defer(cx, move |window, cx| {
                if window.focused(cx).is_none() {
                    root.focus(window, cx);
                }
            });
        }
        let theme = theme(cx);
        // Tab from the root itself starts at the first tab stop, Shift+Tab at the last (the root
        // isn't one, and GPUI would go on from where its handle sorts).
        let (next, prev) = (focus.clone(), focus.clone());
        qust_fluent::focus::tab_navigation(
            div()
                .id("window-root")
                .track_focus(&focus)
                .capture_action(move |_: &qust_fluent::focus::FocusNext, window, cx| {
                    if next.is_focused(window) {
                        window.blur(cx);
                    }
                })
                .capture_action(move |_: &qust_fluent::focus::FocusPrev, window, cx| {
                    if prev.is_focused(window) {
                        window.blur(cx);
                    }
                })
                .relative()
                .size_full()
                .bg(theme.bg)
                .child(self.content.clone().cached(StyleRefinement::default().size_full()))
                .children(self.live.iter().filter_map(|live| {
                    let bounds = live.bounds?;
                    Some(
                        div()
                            .absolute()
                            .left(bounds.origin.x)
                            .top(bounds.origin.y)
                            .w(bounds.size.width)
                            .h(bounds.size.height)
                            .child(live.view.clone().cached(StyleRefinement::default().size_full())),
                    )
                }))
                .children(self.overlays.iter().map(|(_, view)| div().absolute().inset_0().child(view.clone().cached(StyleRefinement::default().size_full())))),
        )
    }
}

/// Where the live layer `key` goes: put it in the content where the page had the element, sized
/// like it. It draws nothing itself.
pub fn portal_slot(root: WeakEntity<WindowRoot>, key: impl Into<SharedString>) -> impl IntoElement {
    let key: SharedString = key.into();
    canvas(
        move |bounds, _, cx| {
            if let Some(root) = root.upgrade() {
                root.update(cx, |root, cx| root.slot_moved(&key, bounds, cx));
            }
        },
        |_, _, _, _| {},
    )
    .size_full()
}

/// The analyzer as a live layer: observes the frame model and paints only itself.
pub struct AnalyzerView {
    model: Entity<crate::models::AnalyzerFrame>,
    /// Renders (measurements and tests).
    pub renders: u64,
}

impl AnalyzerView {
    pub fn new(model: &Entity<crate::models::AnalyzerFrame>, cx: &mut Context<Self>) -> AnalyzerView {
        cx.observe(model, |_, _, cx| cx.notify()).detach();
        AnalyzerView { model: model.clone(), renders: 0 }
    }

    /// The analyzer's size in CSS px at this window's scale (for its slot).
    pub fn css_size(window: &Window) -> (Pixels, Pixels) {
        let (w, h) = qust_fluent::analyzer::css_size(window.scale_factor());
        (px(w), px(h))
    }
}

impl Render for AnalyzerView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.renders += 1;
        let theme = theme(cx);
        let model = self.model.read(cx);
        qust_fluent::Analyzer::new(model.frame, model.colors, theme.accent_light, theme.display)
    }
}

/// Builds a window's root around `content`.
pub fn root<V: 'static + Render>(content: Entity<V>, cx: &mut App) -> Entity<WindowRoot> {
    cx.new(|_| WindowRoot::new(content))
}

/// The analyzer's slot in the content: reserves its size and hands its place to the root.
pub fn analyzer_slot(root: WeakEntity<WindowRoot>, window: &Window) -> impl IntoElement {
    let (w, h) = AnalyzerView::css_size(window);
    div().w(w).h(h).flex_none().child(portal_slot(root, "analyzer"))
}
