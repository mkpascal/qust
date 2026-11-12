//! The window's live layers: what changes while something plays, drawn by the window root at the
//! slots the islands leave, so it re-renders alone (`qust_ui::window`):
//!
//! | key | view | changes |
//! |---|---|---|
//! | `analyzer` | `qust_ui::AnalyzerView` | every frame from the player (≤ 60 Hz) |
//! | `clock` | [`ClockView`] | once a second while playing, on state changes |
//! | `title` | `qust_fluent::Marquee` | on title changes; animates while it scrolls |
//! | `seek` | [`crate::sliders::SeekView`] | four times a second while a file plays |
//! | `badge-0` … `badge-5` | [`crate::cartridges::BadgeView`] | the equalizer and the pulsing bolt animate |

use crate::display::{clock_text, CLOCK_IDLE};
use crate::view::Shared;
use gpui::{div, prelude::*, px, App, Context, Entity, IntoElement, Render, Window};
use qust_fluent::{icon, theme, Color, Icon, Marquee, TextStyles};
use qust_ui::models::Clock;
use qust_ui::{AnalyzerView, Playback};
use std::rc::Rc;

/// The clock and its state icon (`#clock`).
pub struct ClockView {
    clock: Entity<Clock>,
    pub renders: u64,
}

impl ClockView {
    pub fn new(clock: &Entity<Clock>, cx: &mut Context<Self>) -> ClockView {
        cx.observe(clock, |_, _, cx| cx.notify()).detach();
        ClockView { clock: clock.clone(), renders: 0 }
    }
}

impl Render for ClockView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.renders += 1;
        let theme = theme(cx);
        let clock = self.clock.read(cx);
        let idle = clock.state == Playback::Stopped;
        let glyph = match clock.state {
            Playback::Stopped => Icon::Stop,
            Playback::Paused => Icon::Pause,
            _ => Icon::Play,
        };
        // The text is white on the screen in both themes; the icon is --accent-light in both.
        let (text, glyph_color) = if idle { (CLOCK_IDLE, CLOCK_IDLE) } else { (Color::WHITE, theme.accent_light) };
        div()
            .size_full()
            .flex()
            .items_center()
            .gap(px(5.0))
            .font_display()
            .semibold()
            .type_size(22.0, 24.0)
            .tabular_nums()
            .whitespace_nowrap()
            .text_color(text)
            .child(icon(glyph, 10.0).text_color(glyph_color))
            .child(clock_text(clock.state, clock.seconds))
    }
}

/// The views the live layers show (tests count their renders).
#[derive(Clone)]
pub struct LiveViews {
    pub analyzer: Entity<AnalyzerView>,
    pub clock: Entity<ClockView>,
    pub title: Entity<Marquee>,
    pub seek: Entity<crate::sliders::SeekView>,
    pub badges: Vec<Entity<crate::cartridges::BadgeView>>,
}

/// The title row's backdrop: the island's fill over the window background (the marquee fades
/// its ends into it).
fn title_colors(cx: &App) -> ((Color, Color), Color) {
    let theme = theme(cx);
    ((theme.text, theme.text_3), theme.card.over(theme.bg))
}

pub(crate) fn install(shared: &Rc<Shared>, _: &mut Window, cx: &mut App) -> Option<LiveViews> {
    let models = &shared.models;
    let analyzer = cx.new(|cx| AnalyzerView::new(&models.analyzer, cx));
    let clock = cx.new(|cx| ClockView::new(&models.clock, cx));
    let (colors, backdrop) = title_colors(cx);
    let now = models.now.read(cx).clone();
    let title = cx.new(|_| Marquee::new(crate::text::shown(&now.title), now.title_dim, colors, backdrop));
    let marquee = title.clone();
    cx.observe(&models.now, move |now, cx| {
        let (text, dim) = {
            let now = now.read(cx);
            (now.title.clone(), now.title_dim)
        };
        marquee.update(cx, |m, cx| m.set(crate::text::shown(&text), dim, cx));
    })
    .detach();
    let marquee = title.clone();
    cx.observe_global::<qust_fluent::Theme>(move |cx| {
        let (colors, backdrop) = title_colors(cx);
        marquee.update(cx, |m, cx| m.set_colors(colors, backdrop, cx));
    })
    .detach();
    let seek = crate::sliders::seek_layer(shared, cx);
    let badges = shared.cartridges.get().map(|c| c.read(cx).badges().to_vec()).unwrap_or_default();
    let views = LiveViews { analyzer, clock, title, seek, badges };
    let root = shared.root.upgrade()?;
    let layers = views.clone();
    root.update(cx, |root, cx| {
        root.set_live("analyzer", layers.analyzer, cx);
        root.set_live("clock", layers.clock, cx);
        root.set_live("title", layers.title, cx);
        root.set_live("seek", layers.seek, cx);
        for (slot, badge) in layers.badges.into_iter().enumerate() {
            root.set_live(crate::cartridges::badge_key(slot), badge, cx);
        }
    });
    Some(views)
}
