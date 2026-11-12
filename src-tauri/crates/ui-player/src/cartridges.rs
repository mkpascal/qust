//! The six cartridges (`#cartridges`, BEHAVIOUR.md 2.5): every look (empty, loaded, active,
//! playing with the equalizer, the instant switch bolts), click to play or to open the dialog,
//! the eject strip, right-click menus, and drag to reorder (the ghost, the drop target outline,
//! Esc, the swallowed click after a drag, the "dropped" scale-in after a move).
//!
//! **Scale.** CSS scales a whole cartridge with `transform: scale()` ("dropped" 0.94 → 1, the
//! ghost 1.04). GPUI has no transforms on elements, so [`cartridge`] lays the cartridge out at
//! scale `k`: every length (box, padding, radii, font sizes, icons, shadows) times `k`, centred
//! on its cell. Text then shapes at the scaled size, as Chromium rasterised it.
//!
//! **Opacity.** The dragged cartridge fades to 0.35 as a group in CSS; GPUI applies opacity to
//! each primitive, which would let the fill show through the text. So the source is drawn with
//! its colors flattened instead ([`Fade`]): the fill at 35 %, and what sits on the fill (text,
//! icons, the divider) as the opaque color the group gives over the window background.
//!
//! **Live parts.** The equalizer and the pulsing bolt animate for as long as something plays,
//! so they are live layers of the window root ([`BadgeView`], one per slot) drawn in a slot
//! each body leaves at top 8 / right 8: the grid itself re-renders only when the deck changes.

use crate::dialog::{host, DialogTarget};
use crate::menus;
use crate::view::{forced, Shared};
use gpui::{
    canvas, div, fill, point, prelude::*, px, size, AnyElement, App, Bounds, Context, Corners, DispatchPhase, Entity, FocusHandle, IntoElement, MouseButton,
    MouseDownEvent, MouseMoveEvent, MouseUpEvent, Pixels, Point, Render, SharedString, Window,
};
use qust_fluent::motion::{dropped_scale, equalizer_scale, motion, preloading_opacity, timing};
use qust_fluent::paint::css_shadow;
use qust_fluent::{icon, theme, Color, Forced, Icon, TextStyles, Theme, TooltipExt};
use qust_ui::models::Deck;
use qust_ui::{portal_slot, Cartridge, Preload};
use std::cell::Cell;
use std::rc::Rc;
use std::time::Instant;

pub const SLOTS: usize = 6;
pub const GRID_WIDTH: f32 = 458.0;
pub const GRID_HEIGHT: f32 = 183.0;
pub const GAP: f32 = 10.0;
/// A cell of the 3 × 2 grid (146 × 86.5).
pub const CELL_WIDTH: f32 = (GRID_WIDTH - 2.0 * GAP) / 3.0;
pub const CELL_HEIGHT: f32 = (GRID_HEIGHT - GAP) / 2.0;
/// Pixels the pointer moves (Euclidean) before a press becomes a drag.
pub const DRAG_THRESHOLD: f32 = 5.0;
/// The ghost's `scale(1.04)`.
pub const GHOST_SCALE: f32 = 1.04;
/// `.cartridge.is-dragging { opacity: 0.35 }`
pub const DRAG_OPACITY: f32 = 0.35;

/// A cell's top-left corner in the grid.
pub fn cell_origin(slot: usize) -> (f32, f32) {
    ((slot % 3) as f32 * (CELL_WIDTH + GAP), if slot < 3 { 0.0 } else { ROW_HEIGHTS[0] + GAP })
}

/// The rows as Chromium painted them: laid out 86.5 tall at 0 and 96.5, painted with their
/// edges rounded half up (6 + 86.5 = 92.5 → 93, 102.5 → 103). GPUI snaps element edges
/// rounding halves toward zero, which would draw every edge of the second row (and the first
/// row's eject strips) a pixel higher; the rows get the heights Chromium's pixels have.
pub const ROW_HEIGHTS: [f32; 2] = [87.0, 86.0];

/// A cell's height in the grid.
pub fn cell_height(slot: usize) -> f32 {
    ROW_HEIGHTS[(slot / 3).min(1)]
}

/// How one cartridge looks.
#[derive(Clone, Debug, PartialEq)]
pub struct Look {
    pub slot: usize,
    pub cartridge: Option<Cartridge>,
    pub active: bool,
    /// Active and not stopped or paused (connecting and buffering too).
    pub playing: bool,
}

impl Look {
    pub fn of(deck: &Deck, slot: usize) -> Look {
        let cartridge = deck.cartridges.get(slot).cloned().flatten();
        let active = cartridge.is_some() && deck.active_slot == Some(slot);
        Look { slot, active, playing: active && deck.state.is_running(), cartridge }
    }

    pub fn empty(&self) -> bool {
        self.cartridge.is_none()
    }

    pub fn preload(&self) -> Option<Preload> {
        self.cartridge.as_ref().and_then(|c| c.preload)
    }
}

/// The group opacity of the dragged cartridge, flattened (see the module docs).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Fade {
    pub opacity: f32,
    /// The opaque window background behind the cartridge.
    pub bg: Color,
}

impl Fade {
    /// A layer painted straight on the background (fill, ring, border).
    fn layer(fade: Option<Fade>, color: Color) -> Color {
        match fade {
            Some(f) => color.mix(f.opacity),
            None => color,
        }
    }

    /// Something painted on `fill` (text, icons, the divider): the color the group gives.
    fn on(fade: Option<Fade>, color: Color, fill: Color) -> Color {
        match fade {
            Some(f) => color.over(fill).mix(f.opacity).over(f.bg),
            None => color,
        }
    }
}

/// Where a cartridge is drawn and how.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Mode {
    /// In the grid: buttons, focus, the badge slot for the live layer.
    Grid,
    /// The drag ghost: `--dialog` (or the accent) fill, the drop shadow, badges drawn inline,
    /// no interaction. `since`: when the clone appeared (its animations start there).
    Ghost { since: Instant },
}

/// The colors of one look.
struct Palette {
    text: Color,
    host: Color,
    eject: Color,
    divider: Color,
    fill: Option<Color>,
    /// The fill goes under the border (`background` shorthand: `background-clip` reset).
    fill_border_box: bool,
    ring: Option<Color>,
    border: Option<Color>,
    dashed: bool,
    hover: Color,
    pressed: Color,
    focus: Color,
    shadow: bool,
}

fn palette(look: &Look, mode: Mode, theme: &Theme) -> Palette {
    let ghost = matches!(mode, Mode::Ghost { .. });
    if look.empty() {
        return Palette {
            text: theme.text_3,
            host: theme.text_3,
            eject: theme.text_2,
            divider: theme.card_stroke,
            fill: None,
            fill_border_box: false,
            ring: None,
            border: Some(theme.empty_stroke),
            dashed: true,
            hover: theme.subtle_hover,
            pressed: theme.subtle_pressed,
            focus: theme.focus,
            shadow: false,
        };
    }
    if look.active {
        // `.cartridge.is-active` (0,2,0) wins over `.cartridge-ghost` (0,1,0) for the shadow:
        // the active ghost has none.
        return Palette {
            text: theme.on_accent,
            host: theme.on_accent.mix(0.7),
            eject: theme.on_accent.mix(0.7),
            divider: theme.on_accent.mix(0.15),
            fill: Some(theme.accent),
            fill_border_box: true,
            ring: None,
            border: None,
            dashed: false,
            hover: theme.on_accent.mix(0.08),
            pressed: theme.on_accent.mix(0.04),
            focus: theme.on_accent,
            shadow: false,
        };
    }
    Palette {
        text: theme.text,
        host: theme.text_3,
        eject: theme.text_2,
        divider: theme.card_stroke,
        fill: Some(if ghost { theme.dialog } else { theme.card }),
        fill_border_box: ghost,
        ring: if ghost { None } else { Some(theme.card_edge) },
        border: Some(theme.card_stroke),
        dashed: false,
        hover: theme.subtle_hover,
        pressed: theme.subtle_pressed,
        focus: theme.focus,
        shadow: ghost,
    }
}

/// What the grid adds to a cartridge (the ghost gets none).
pub struct Interaction {
    pub body_focus: FocusHandle,
    pub eject_focus: FocusHandle,
    pub body_forced: Forced,
    pub eject_forced: Forced,
    pub body_ring: bool,
    pub eject_ring: bool,
    pub drop_target: bool,
    pub fade: Option<Fade>,
    pub root: gpui::WeakEntity<qust_ui::WindowRoot>,
    pub on_body: Box<dyn Fn(&mut Window, &mut App)>,
    pub on_eject: Box<dyn Fn(&mut Window, &mut App)>,
    pub on_press: Box<dyn Fn(&MouseDownEvent, &mut Window, &mut App)>,
}

/// One cartridge at scale `k`, `width` × `height` before scaling, positioned in its cell
/// (centred, as `transform-origin: center`), or at the ghost's place by the caller.
pub fn cartridge(look: &Look, k: f32, mode: Mode, interaction: Option<Interaction>, theme: &Theme, window: &Window, cx: &App) -> AnyElement {
    let colors = palette(look, mode, theme);
    let fade = interaction.as_ref().and_then(|i| i.fade);
    let natural = match mode {
        Mode::Grid => cell_height(look.slot),
        Mode::Ghost { .. } => CELL_HEIGHT,
    };
    let (width, height) = (CELL_WIDTH * k, natural * k);
    let s = |v: f32| px(v * k);
    let radius = 8.0 * k;
    let inner = (8.0 - 1.0) * k;
    let under_text = colors.fill.unwrap_or(Color::TRANSPARENT);
    let text = Fade::on(fade, colors.text, under_text);
    let host_color = Fade::on(fade, colors.host, under_text);
    let eject_color = Fade::on(fade, colors.eject, under_text);
    let divider = Fade::on(fade, colors.divider, under_text);
    let empty = look.empty();
    let slot = look.slot;

    let mut frame: Vec<AnyElement> = Vec::new();
    // The layers sit in the 1 px layout border: −1 reaches the border box.
    let layer = |inset: f32, r: f32| div().absolute().top(px(inset)).left(px(inset)).right(px(inset)).bottom(px(inset)).rounded(px(r));
    if let Some(fill_color) = colors.fill {
        let fill_color = Fade::layer(fade, fill_color);
        frame.push(if colors.fill_border_box { layer(-1.0, radius) } else { layer(0.0, inner) }.bg(fill_color).into_any_element());
    }
    if let Some(ring) = colors.ring {
        frame.push(layer(0.0, inner).border_1().border_color(Fade::layer(fade, ring)).into_any_element());
    }
    if let Some(border) = colors.border {
        let border = Fade::layer(fade, border);
        frame.push(layer(-1.0, radius).border_1().border_color(border).when(colors.dashed, |l| l.border_dashed()).into_any_element());
    }

    // The badges: a slot for the live layer in the grid, drawn inline on the ghost.
    let badge: AnyElement = match (&mode, &interaction) {
        (Mode::Grid, Some(interaction)) => div().absolute().top(s(8.0)).right(s(8.0)).w(s(11.0)).h(s(11.0)).child(portal_slot(interaction.root.clone(), badge_key(slot))).into_any_element(),
        (Mode::Ghost { since }, _) => {
            let since = *since;
            let accent = theme.accent;
            let playing = look.playing;
            let preload = look.preload();
            let bars_color = colors.text;
            let elapsed = motion(cx).elapsed(since, None);
            div()
                .absolute()
                .top(s(8.0))
                .right(s(8.0))
                .w(s(11.0))
                .h(s(11.0))
                .child(canvas(|_, _, _| {}, move |bounds, _, window, cx| paint_badge(bounds, playing.then_some(elapsed), preload.map(|p| (p, elapsed)), bars_color, accent, 1.0, window, cx)).size_full())
                .into_any_element()
        }
        _ => div().into_any_element(),
    };

    let mut body = div()
        .id(SharedString::from(format!("cartridge-body-{slot}")))
        .relative()
        .flex()
        .flex_1()
        .items_center()
        .justify_center()
        .min_h_0()
        .py(s(4.0))
        .px(s(12.0))
        .text_color(text);
    body = if empty { body.flex_row().gap(s(6.0)).rounded(px(inner)) } else { body.flex_col().rounded_t(px(inner)) };
    body = if empty {
        body.child(icon(Icon::Plus, 16.0 * k).text_color(text)).child(div().font_text().regular().type_size(13.0 * k, 18.0 * k).whitespace_nowrap().child("Load"))
    } else {
        let cartridge = look.cartridge.clone().unwrap_or_default();
        // max-width 100% of the body's content box, cut with Chromium's ellipsis.
        let available = (CELL_WIDTH - 2.0 - 24.0) * k;
        let name_font = crate::text::page_font(true, gpui::FontWeight::SEMIBOLD, crate::text::kerning());
        let host_font = crate::text::page_font(false, gpui::FontWeight::NORMAL, crate::text::kerning());
        let (name, name_cut) = crate::text::ellipsized(&crate::text::shown(&cartridge.title), &name_font, 15.0 * k, available, window);
        let (host_text, host_cut) = crate::text::ellipsized(&crate::text::shown(&host(&cartridge.url)), &host_font, 11.0 * k, available, window);
        // A cut text's box is the whole max-width, the text on its left (not centred).
        body.child(div().whitespace_nowrap().when(name_cut, |d| d.w(px(available))).font_display().semibold().type_size(15.0 * k, 20.0 * k).child(name))
            .child(
                div()
                    .whitespace_nowrap()
                    .when(host_cut, |d| d.w(px(available)))
                    .font_text()
                    .regular()
                    .type_size(11.0 * k, 14.0 * k)
                    .text_color(host_color)
                    // On Chromium's baseline (crate::text::baseline_shift): 1 px up for 11/14.
                    .relative()
                    .top(px(crate::text::baseline_shift(11.0 * k, 14.0 * k, false, window)))
                    .child(host_text),
            )
    };
    body = body.child(badge);

    let mut eject = (!empty).then(|| {
        div()
            .id(SharedString::from(format!("cartridge-eject-{slot}")))
            .relative()
            .flex()
            .flex_none()
            .items_center()
            .justify_center()
            .h(s(22.0))
            .border_t_1()
            .border_color(divider)
            .rounded_b(px(inner))
            // In the second row Chromium centred the icon at a half pixel (170.5) and drew it
            // from 171; GPUI's snapping would draw it from 170.
            .child(icon(Icon::Eject, 14.0 * k).text_color(eject_color).when(slot >= 3 && matches!(mode, Mode::Grid) && k == 1.0, |i| i.mt(px(1.0))))
    });

    let mut outlines: Vec<AnyElement> = Vec::new();
    if let Some(interaction) = interaction {
        let label = match &look.cartridge {
            Some(c) => format!("Play {}", c.title),
            None => format!("Load cartridge {}", slot + 1),
        };
        // Hover and pressed fills, the body and the strip separately. The pointer decides,
        // unless a fixture forces one.
        let (hover, pressed) = (colors.hover, colors.pressed);
        let fill_of = |forced: Forced| if forced.active { Some(pressed) } else if forced.hover { Some(hover) } else { None };
        let body_fixed = fill_of(interaction.body_forced);
        let eject_fixed = fill_of(interaction.eject_forced);
        body = body
            .role(gpui::Role::Button)
            .aria_label(SharedString::from(label))
            .track_focus(&interaction.body_focus)
            .when_some(body_fixed, |b, fill| b.bg(fill))
            .when(body_fixed.is_none(), |b| b.hover(move |s| s.bg(hover)).active(move |s| s.bg(pressed)));
        let on_press = interaction.on_press;
        body = body.on_mouse_down(MouseButton::Left, move |event, window, cx| on_press(event, window, cx));
        let on_body = interaction.on_body;
        body = body.on_click(move |_, window, cx| on_body(window, cx));
        if interaction.body_ring {
            let ring = div().absolute().inset_0().border_2().border_color(colors.focus);
            body = body.child(if empty { ring.rounded(px(inner)) } else { ring.rounded_t(px(inner)) });
        }
        if let Some(strip) = eject.take() {
            let on_eject = interaction.on_eject;
            let mut strip = strip
                .role(gpui::Role::Button)
                .aria_label("Edit or eject")
                .native_tooltip("Edit or eject")
                .track_focus(&interaction.eject_focus)
                .when_some(eject_fixed, |b, fill| b.bg(fill))
                .when(eject_fixed.is_none(), |b| b.hover(move |s| s.bg(hover)).active(move |s| s.bg(pressed)))
                // A press on the strip never starts a drag.
                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                .on_click(move |_, window, cx| on_eject(window, cx));
            if interaction.eject_ring {
                strip = strip.child(div().absolute().top(px(-1.0)).left_0().right_0().bottom_0().border_2().border_color(colors.focus).rounded_b(px(inner)));
            }
            eject = Some(strip);
        }
        if interaction.drop_target {
            // outline: 2px solid accent; outline-offset: 2px (radius 8 + 4 outside).
            let out = 4.0 * k + 1.0;
            outlines.push(div().absolute().top(px(-out)).left(px(-out)).right(px(-out)).bottom(px(-out)).border(s(2.0)).border_color(theme.accent).rounded(px(radius + 4.0 * k)).into_any_element());
        }
    }

    let shadow = colors.shadow.then(|| {
        vec![css_shadow(0.0, 2.0 * k, 8.0 * k, 0.0, Color::rgba(0, 0, 0, 64.0 / 255.0)), css_shadow(0.0, 16.0 * k, 32.0 * k, 0.0, Color::rgba(0, 0, 0, 89.0 / 255.0))]
    });

    let mut boxed = div()
        .relative()
        .w(px(width))
        .h(px(height))
        .flex()
        .flex_col()
        .border_1()
        .border_color(Color::TRANSPARENT)
        .rounded(px(radius))
        .when_some(shadow, |b, shadow| b.shadow(shadow))
        .children(frame)
        .child(body);
    if let Some(strip) = eject {
        boxed = boxed.child(strip);
    }
    boxed.children(outlines).into_any_element()
}

/// Paints the badges in their slot (11 × 11 at scale 1): the equalizer (three 2 × 10 bars at
/// right 9, `scaleY` from the bottom) and the bolt (11 × 11 at right 8) over it, as the DOM had
/// them.
#[allow(clippy::too_many_arguments)]
fn paint_badge(bounds: Bounds<Pixels>, equalizer: Option<std::time::Duration>, bolt: Option<(Preload, std::time::Duration)>, bars: Color, accent: Color, opacity: f32, window: &mut Window, cx: &mut App) {
    let k = bounds.size.width.as_f32() / 11.0;
    let (x0, y0) = (bounds.origin.x.as_f32(), bounds.origin.y.as_f32());
    let reduced = motion(cx).reduced;
    if let Some(elapsed) = equalizer {
        for bar in 0..3 {
            let scale = if reduced { 1.0 } else { equalizer_scale(elapsed, bar) as f32 };
            let h = 10.0 * k * scale;
            let rect = Bounds::new(point(px(x0 + 4.0 * bar as f32 * k), px(y0 + 10.0 * k - h)), size(px(2.0 * k), px(h)));
            window.paint_quad(fill(rect, bars.alpha(bars.a * opacity)).corner_radii(Corners::all(px(k.min(h / 2.0)))));
        }
    }
    if let Some((preload, elapsed)) = bolt {
        let alpha = match preload {
            Preload::Live => 1.0,
            Preload::Loading if reduced => 1.0,
            Preload::Loading => preloading_opacity(elapsed) as f32,
        };
        let color = accent.alpha(accent.a * alpha * opacity);
        let _ = window.paint_svg(bounds, Icon::Bolt.path().into(), None, gpui::TransformationMatrix::unit(), color.into(), cx);
    }
}

pub fn badge_key(slot: usize) -> String {
    format!("badge-{slot}")
}

/// One slot's badges as a live layer.
pub struct BadgeView {
    shared: Rc<Shared>,
    slot: usize,
    playing_since: Option<Instant>,
    preload: Option<(Preload, Instant)>,
    faded: bool,
    pub renders: u64,
}

impl BadgeView {
    pub(crate) fn new(shared: Rc<Shared>, slot: usize, cx: &mut Context<Self>) -> BadgeView {
        cx.observe(&shared.models.deck, |this: &mut BadgeView, deck, cx| {
            let look = Look::of(deck.read(cx), this.slot);
            if this.follow(&look) {
                cx.notify();
            }
        })
        .detach();
        let look = Look::of(shared.models.deck.read(cx), slot);
        let mut view = BadgeView { shared, slot, playing_since: None, preload: None, faded: false, renders: 0 };
        view.follow(&look);
        view
    }

    // The CSS animations (re)start when their class comes on.
    fn follow(&mut self, look: &Look) -> bool {
        let mut changed = false;
        if look.playing != self.playing_since.is_some() {
            self.playing_since = look.playing.then(Instant::now);
            changed = true;
        }
        let preload = look.preload();
        if preload != self.preload.map(|(p, _)| p) {
            // Live → loading restarts the pulse; loading → live stops it.
            self.preload = preload.map(|p| (p, self.preload.filter(|(old, _)| *old == p).map_or_else(Instant::now, |(_, at)| at)));
            changed = true;
        }
        changed
    }

    pub fn set_faded(&mut self, faded: bool, cx: &mut Context<Self>) {
        if self.faded != faded {
            self.faded = faded;
            cx.notify();
        }
    }
}

impl Render for BadgeView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.renders += 1;
        let theme = *theme(cx);
        let motion = motion(cx);
        let look = Look::of(self.shared.models.deck.read(cx), self.slot);
        let animating = !motion.reduced && (self.playing_since.is_some() || matches!(self.preload, Some((Preload::Loading, _))));
        if animating && motion.wants_frames() {
            window.request_animation_frame();
        }
        let equalizer = self.playing_since.map(|since| motion.elapsed(since, None));
        let bolt = self.preload.map(|(p, since)| (p, motion.elapsed(since, None)));
        let bars = palette(&look, Mode::Grid, &theme).text;
        let accent = theme.accent;
        let opacity = if self.faded { DRAG_OPACITY } else { 1.0 };
        canvas(|_, _, _| {}, move |bounds, _, window, cx| paint_badge(bounds, equalizer, bolt, bars, accent, opacity, window, cx)).size_full()
    }
}

/// A drag in progress (or a press that may become one).
#[derive(Clone, Debug, PartialEq)]
pub struct Drag {
    pub slot: usize,
    /// Where the button went down, in window coordinates.
    pub start: Point<f32>,
    pub active: bool,
    /// The press point from the cartridge's top-left corner.
    pub offset: Point<f32>,
    pub pointer: Point<f32>,
    pub target: Option<usize>,
}

/// The drag ghost, an overlay of the window root.
pub struct GhostView {
    pub look: Option<Look>,
    /// Its scaled box's centre is the pointer minus the grab offset plus half the cartridge.
    pub pointer: Point<f32>,
    pub offset: Point<f32>,
    since: Instant,
    pub renders: u64,
}

impl GhostView {
    /// Where the ghost's border box is (window coordinates, scaled about its centre).
    pub fn bounds(&self) -> Bounds<Pixels> {
        let left = self.pointer.x - self.offset.x + CELL_WIDTH * (1.0 - GHOST_SCALE) / 2.0;
        let top = self.pointer.y - self.offset.y + CELL_HEIGHT * (1.0 - GHOST_SCALE) / 2.0;
        Bounds::new(point(px(left), px(top)), size(px(CELL_WIDTH * GHOST_SCALE), px(CELL_HEIGHT * GHOST_SCALE)))
    }
}

impl Render for GhostView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.renders += 1;
        let theme = *theme(cx);
        let Some(look) = self.look.clone() else { return div().into_any_element() };
        let motion = motion(cx);
        if (look.playing || look.preload() == Some(Preload::Loading)) && motion.wants_frames() && !motion.reduced {
            window.request_animation_frame();
        }
        let bounds = self.bounds();
        div()
            .size_full()
            .child(div().absolute().left(bounds.origin.x).top(bounds.origin.y).child(cartridge(&look, GHOST_SCALE, Mode::Ghost { since: self.since }, None, &theme, window, cx)))
            .into_any_element()
    }
}

pub struct CartridgesView {
    shared: Rc<Shared>,
    pub body_focus: [FocusHandle; SLOTS],
    pub eject_focus: [FocusHandle; SLOTS],
    drag: Option<Drag>,
    suppress_click: bool,
    dropped: [Option<Instant>; SLOTS],
    /// Where the grid was laid out (window coordinates), for mapping the pointer to a slot.
    origin: Rc<Cell<Option<Point<f32>>>>,
    ghost: Entity<GhostView>,
    badges: Vec<Entity<BadgeView>>,
    pub renders: u64,
}

impl CartridgesView {
    pub(crate) fn new(shared: Rc<Shared>, cx: &mut Context<Self>) -> CartridgesView {
        cx.observe(&shared.models.deck, |_, _, cx| cx.notify()).detach();
        let ghost = cx.new(|_| GhostView { look: None, pointer: point(0.0, 0.0), offset: point(0.0, 0.0), since: Instant::now(), renders: 0 });
        let badges = (0..SLOTS).map(|slot| {
            let shared = shared.clone();
            cx.new(|cx| BadgeView::new(shared, slot, cx))
        });
        let badges = badges.collect();
        CartridgesView {
            shared,
            body_focus: std::array::from_fn(|_| qust_fluent::focus::tab_stop(cx)),
            eject_focus: std::array::from_fn(|_| qust_fluent::focus::tab_stop(cx)),
            drag: None,
            suppress_click: false,
            dropped: [None; SLOTS],
            origin: Rc::new(Cell::new(None)),
            ghost,
            badges,
            renders: 0,
        }
    }

    pub(crate) fn badges(&self) -> &[Entity<BadgeView>] {
        &self.badges
    }

    pub fn ghost(&self) -> &Entity<GhostView> {
        &self.ghost
    }

    pub fn drag(&self) -> Option<&Drag> {
        self.drag.as_ref()
    }

    /// A drag is showing its ghost.
    pub fn dragging(&self) -> bool {
        self.drag.as_ref().is_some_and(|d| d.active)
    }

    /// The slot whose cell holds window point `p`.
    pub fn slot_at(&self, p: Point<f32>) -> Option<usize> {
        let origin = self.origin.get()?;
        (0..SLOTS).find(|&slot| {
            let (x, y) = cell_origin(slot);
            let (x, y) = (origin.x + x, origin.y + y);
            p.x >= x && p.x < x + CELL_WIDTH && p.y >= y && p.y < y + cell_height(slot)
        })
    }

    /// When a "dropped" animation of `slot` started, if it runs.
    pub fn dropped_since(&self, slot: usize) -> Option<Instant> {
        self.dropped.get(slot).copied().flatten()
    }

    fn press(&mut self, slot: usize, event: &MouseDownEvent, cx: &mut Context<Self>) {
        // pointerdown: the click after the last drag is no longer swallowed.
        self.suppress_click = false;
        let loaded = self.shared.models.deck.read(cx).cartridges.get(slot).is_some_and(Option::is_some);
        if !loaded {
            return;
        }
        let p = point(event.position.x.as_f32(), event.position.y.as_f32());
        self.drag = Some(Drag { slot, start: p, active: false, offset: point(0.0, 0.0), pointer: p, target: None });
    }

    fn pointer_moved(&mut self, p: Point<f32>, cx: &mut Context<Self>) {
        let Some(mut drag) = self.drag.clone() else { return };
        if !drag.active {
            let (dx, dy) = (p.x - drag.start.x, p.y - drag.start.y);
            if (dx * dx + dy * dy).sqrt() < DRAG_THRESHOLD {
                return;
            }
            let Some(origin) = self.origin.get() else { return };
            let (x, y) = cell_origin(drag.slot);
            drag.active = true;
            drag.offset = point(drag.start.x - (origin.x + x), drag.start.y - (origin.y + y));
            let look = Look::of(self.shared.models.deck.read(cx), drag.slot);
            let (offset, pointer) = (drag.offset, p);
            self.ghost.update(cx, |ghost, cx| {
                ghost.look = Some(look);
                ghost.offset = offset;
                ghost.pointer = pointer;
                ghost.since = Instant::now();
                cx.notify();
            });
            if let Some(root) = self.shared.root.upgrade() {
                let ghost = self.ghost.clone();
                root.update(cx, |root, cx| root.set_overlay("cartridge-ghost", ghost, cx));
            }
            if let Some(badge) = self.badges.get(drag.slot) {
                badge.update(cx, |badge, cx| badge.set_faded(true, cx));
            }
            cx.notify();
        }
        drag.pointer = p;
        self.ghost.update(cx, |ghost, cx| {
            ghost.pointer = p;
            cx.notify();
        });
        let target = self.slot_at(p).filter(|&slot| slot != drag.slot);
        if target != drag.target {
            drag.target = target;
            cx.notify();
        }
        self.drag = Some(drag);
    }

    /// `endDrag(drop)`: the ghost goes, the source comes back; a drop over a target moves.
    pub fn end_drag(&mut self, drop: bool, cx: &mut Context<Self>) {
        let Some(drag) = self.drag.take() else { return };
        if !drag.active {
            return;
        }
        if let Some(root) = self.shared.root.upgrade() {
            root.update(cx, |root, cx| root.remove_overlay("cartridge-ghost", cx));
        }
        self.ghost.update(cx, |ghost, _| ghost.look = None);
        if let Some(badge) = self.badges.get(drag.slot) {
            badge.update(cx, |badge, cx| badge.set_faded(false, cx));
        }
        self.suppress_click = true;
        if let (true, Some(target)) = (drop, drag.target) {
            self.move_cartridge(drag.slot, target, cx);
        }
        cx.notify();
    }

    /// `moveCartridge(from, to)`: the move, then both slots play "dropped" once it is done.
    pub fn move_cartridge(&mut self, from: usize, to: usize, cx: &mut Context<Self>) {
        let answer = self.shared.commands.move_cartridge(from, to);
        cx.spawn(async move |this, cx| {
            if answer.await.is_none() {
                return;
            }
            let _ = this.update(cx, |this, cx| {
                let now = Instant::now();
                for slot in [from, to] {
                    if let Some(at) = this.dropped.get_mut(slot) {
                        *at = Some(now);
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn click_body(&mut self, slot: usize, window: &mut Window, cx: &mut Context<Self>) {
        // The click that ends a drag (released over the cartridge it started on).
        if std::mem::take(&mut self.suppress_click) || self.dragging() {
            return;
        }
        let loaded = self.shared.models.deck.read(cx).cartridges.get(slot).is_some_and(Option::is_some);
        if loaded {
            self.shared.commands.select_slot(slot);
        } else {
            self.shared.open_dialog(DialogTarget::Cartridge(slot), window, cx);
        }
    }

    fn click_eject(&mut self, slot: usize, window: &mut Window, cx: &mut Context<Self>) {
        if std::mem::take(&mut self.suppress_click) || self.dragging() {
            return;
        }
        self.shared.open_dialog(DialogTarget::Cartridge(slot), window, cx);
    }
}

fn cartridge_selector(slot: usize, part: &str) -> String {
    format!("#cartridges > .cartridge:nth-child({}) > .{part}", slot + 1)
}

impl Render for CartridgesView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.renders += 1;
        let theme = *theme(cx);
        let motion = motion(cx);
        let deck = self.shared.models.deck.read(cx).clone();
        let this = cx.entity().downgrade();
        let drag = self.drag.clone().filter(|d| d.active);

        // The "dropped" scale-in of moved cartridges.
        let mut animating = false;
        let scales: Vec<f32> = (0..SLOTS)
            .map(|slot| match self.dropped[slot] {
                Some(_) if motion.reduced => {
                    self.dropped[slot] = None;
                    1.0
                }
                Some(start) => {
                    let elapsed = motion.elapsed(start, Some(timing::DROPPED));
                    if motion.is_live() && elapsed >= timing::DROPPED {
                        self.dropped[slot] = None;
                        1.0
                    } else {
                        animating = true;
                        dropped_scale(elapsed) as f32
                    }
                }
                None => 1.0,
            })
            .collect();
        if animating && motion.wants_frames() {
            window.request_animation_frame();
        }

        let cell = |slot: usize, cx: &mut Context<Self>| -> AnyElement {
            let look = Look::of(&deck, slot);
            let k = scales[slot];
            let faded = drag.as_ref().is_some_and(|d| d.slot == slot);
            let target = drag.as_ref().is_some_and(|d| d.target == Some(slot));
            let body_forced = forced(cx, &cartridge_selector(slot, "cartridge-body"));
            let eject_forced = forced(cx, &cartridge_selector(slot, "cartridge-eject"));
            let body_ring = body_forced.focus_visible || qust_fluent::focus::is_focus_visible(&self.body_focus[slot], window, cx);
            let eject_ring = eject_forced.focus_visible || qust_fluent::focus::is_focus_visible(&self.eject_focus[slot], window, cx);
            let (press, body, eject) = (this.clone(), this.clone(), this.clone());
            let interaction = Interaction {
                body_focus: self.body_focus[slot].clone(),
                eject_focus: self.eject_focus[slot].clone(),
                body_forced,
                eject_forced,
                body_ring,
                eject_ring,
                drop_target: target,
                fade: faded.then_some(Fade { opacity: DRAG_OPACITY, bg: theme.bg }),
                root: self.shared.root.clone(),
                on_body: Box::new(move |window, cx| {
                    let _ = body.update(cx, |view, cx| view.click_body(slot, window, cx));
                }),
                on_eject: Box::new(move |window, cx| {
                    let _ = eject.update(cx, |view, cx| view.click_eject(slot, window, cx));
                }),
                on_press: Box::new(move |event, _, cx| {
                    let _ = press.update(cx, |view, cx| view.press(slot, event, cx));
                }),
            };
            let shared = self.shared.clone();
            let offset_x = CELL_WIDTH * (1.0 - k) / 2.0;
            let offset_y = cell_height(slot) * (1.0 - k) / 2.0;
            div()
                .id(SharedString::from(format!("cartridge-{slot}")))
                .relative()
                .flex_1()
                .min_w_0()
                .h_full()
                // Right-click on a cartridge: its own menu.
                .on_mouse_up(MouseButton::Right, move |_: &MouseUpEvent, window, cx| {
                    cx.stop_propagation();
                    if shared.dialog_open(cx) {
                        return;
                    }
                    let loaded = shared.models.deck.read(cx).cartridges.get(slot).is_some_and(Option::is_some);
                    shared.show_menu(menus::cartridge_menu(slot, loaded), window, cx);
                })
                .child(div().absolute().left(px(offset_x)).top(px(offset_y)).child(cartridge(&look, k, Mode::Grid, Some(interaction), &theme, window, cx)))
                .into_any_element()
        };
        let cells: Vec<AnyElement> = (0..SLOTS).map(|slot| cell(slot, cx)).collect();
        let mut cells = cells.into_iter();
        let row0: Vec<AnyElement> = cells.by_ref().take(3).collect();
        let row1: Vec<AnyElement> = cells.collect();

        // Where the grid is, and the drag's window-wide pointer listeners (on every frame: they
        // check the drag when the event comes).
        let origin = self.origin.clone();
        let moved = this.clone();
        let released = this.clone();
        let tracker = canvas(
            move |bounds, _, _| origin.set(Some(point(bounds.origin.x.as_f32(), bounds.origin.y.as_f32()))),
            move |_, _, window, _| {
                let moved = moved.clone();
                window.on_mouse_event(move |event: &MouseMoveEvent, phase, _, cx| {
                    if phase != DispatchPhase::Bubble {
                        return;
                    }
                    let p = point(event.position.x.as_f32(), event.position.y.as_f32());
                    let _ = moved.update(cx, |view, cx| {
                        if view.drag.is_some() {
                            if event.pressed_button == Some(MouseButton::Left) || view.dragging() {
                                view.pointer_moved(p, cx);
                            }
                        }
                    });
                });
                let released = released.clone();
                window.on_mouse_event(move |event: &MouseUpEvent, phase, _, cx| {
                    if phase != DispatchPhase::Bubble || event.button != MouseButton::Left {
                        return;
                    }
                    let _ = released.update(cx, |view, cx| {
                        if view.drag.is_some() {
                            if view.dragging() {
                                view.end_drag(true, cx);
                            } else {
                                view.drag = None;
                            }
                        }
                    });
                });
            },
        )
        .absolute()
        .inset_0();

        div()
            .id("cartridges")
            .relative()
            .size_full()
            .flex()
            .flex_col()
            .gap(px(GAP))
            .child(div().flex().flex_row().gap(px(GAP)).flex_none().h(px(ROW_HEIGHTS[0])).children(row0))
            .child(div().flex().flex_row().gap(px(GAP)).flex_none().h(px(ROW_HEIGHTS[1])).children(row1))
            // Last, so its release listener runs before the cartridges' clicks.
            .child(tracker)
    }
}
