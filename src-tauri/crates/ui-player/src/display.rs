//! The display island (`.display`): the black screen (clock over the analyzer), the station line,
//! the title, the kbps / kHz line and the MONO / STEREO badges (BEHAVIOUR.md 2.2).
//!
//! The clock, the analyzer and the title are live layers ([`crate::live`]): this view leaves
//! slots for them and re-renders only when the station or the specs change, or when the clock
//! gains or loses a digit (the screen is as wide as the clock or the canvas, whichever is wider).

use crate::view::Shared;
use gpui::{div, prelude::*, px, App, Context, FontWeight, HighlightStyle, IntoElement, Render, Window};
use qust_fluent::paint::css_inset_shadow;
use qust_fluent::section::channel_badge;
use qust_fluent::text::{font, line, tabular};
use qust_fluent::{surface, theme, Color, Frame, TextStyles, TooltipExt};
use qust_ui::{analyzer_slot, format, portal_slot, Playback};
use std::rc::Rc;

/// The clock's text for a number of seconds as the clock model holds it.
pub fn clock_text(state: Playback, seconds: u64) -> String {
    format::clock(if state == Playback::Stopped { 0 } else { seconds })
}

/// `String(number)` for the numbers the specs show (integers stay integers).
pub fn js_number(value: f64) -> String {
    if value.fract() == 0.0 && value.abs() < 1e15 {
        format!("{}", value as i64)
    } else {
        format!("{value}")
    }
}

/// The clock's dim color while stopped: `rgba(255,255,255,.35)`, as Chromium quantized it.
pub const CLOCK_IDLE: Color = Color::rgba(255, 255, 255, 89.0 / 255.0);

/// The width of the clock's text at 22 px Display semibold with tabular figures. Every digit has
/// the same advance, so only the pattern ("00:00", "0:00:00") matters.
pub fn clock_text_width(text: &str, window: &Window) -> f32 {
    let pattern: String = text.chars().map(|c| if c.is_ascii_digit() { '0' } else { c }).collect();
    qust_fluent::text::line_width(&pattern, &font(true, FontWeight::SEMIBOLD, tabular()), 22.0, window)
}

pub struct DisplayView {
    shared: Rc<Shared>,
    /// The station line as drawn (`crate::text::shown`), for the station it was made from.
    station: (String, gpui::SharedString),
    /// The clock's text length (its width class), the only thing of the clock this view shows.
    clock_len: usize,
    pub renders: u64,
}

impl DisplayView {
    pub(crate) fn new(shared: Rc<Shared>, cx: &mut Context<Self>) -> DisplayView {
        cx.observe(&shared.models.now, |_, _, cx| cx.notify()).detach();
        cx.observe(&shared.models.clock, |this: &mut DisplayView, clock, cx| {
            let clock = clock.read(cx);
            let len = clock_text(clock.state, clock.seconds).len();
            if len != this.clock_len {
                this.clock_len = len;
                cx.notify();
            }
        })
        .detach();
        let clock = shared.models.clock.read(cx);
        let clock_len = clock_text(clock.state, clock.seconds).len();
        DisplayView { shared, clock_len, station: (String::new(), gpui::SharedString::default()), renders: 0 }
    }
}

/// "<b>value</b> unit": the number semibold in `--text`, the unit in the line's color.
fn spec(value: String, unit: &str, cx: &App) -> impl IntoElement {
    let theme = theme(cx);
    let text = format!("{value} {unit}");
    let bold = HighlightStyle { color: Some(theme.text.into()), font_weight: Some(FontWeight::SEMIBOLD), ..Default::default() };
    line(text).tabular_nums().highlight(0..value.len(), bold)
}

impl Render for DisplayView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.renders += 1;
        let theme = *theme(cx);
        let now = self.shared.models.now.read(cx).clone();
        let clock = self.shared.models.clock.read(cx).clone();
        let commands = self.shared.commands.clone();
        // .clock is as wide as the screen's content: the clock row or the 75 px canvas.
        let clock_width = 15.0 + clock_text_width(&clock_text(clock.state, clock.seconds), window);
        let (canvas_width, _) = qust_ui::AnalyzerView::css_size(window);
        let screen_content = clock_width.max(canvas_width.as_f32());
        let root = self.shared.root.clone();

        let screen = div()
            .id("screen")
            .relative()
            .flex()
            .flex_none()
            .flex_col()
            .justify_between()
            .gap(px(7.0))
            .pt(px(10.0))
            .px(px(12.0))
            .pb(px(12.0))
            .rounded(px(6.0))
            .bg(theme.display)
            // box-shadow: inset 0 0 0 1px rgba(255,255,255,.06), inset 0 1px 3px rgba(0,0,0,.4):
            // the first listed on top (GPUI paints its list in order).
            .shadow(vec![css_inset_shadow(0.0, 1.0, 3.0, 0.0, Color::rgba(0, 0, 0, 102.0 / 255.0)), css_inset_shadow(0.0, 0.0, 0.0, 1.0, Color::rgba(255, 255, 255, 15.0 / 255.0))])
            .native_tooltip("Click to change the analyzer colors")
            .on_click(move |_, _, _| commands.cycle_colors())
            .child(div().w(px(screen_content)).h(px(24.0)).child(portal_slot(root.clone(), "clock")))
            .child(analyzer_slot(root.clone(), window));

        let kbps = match now.bitrate {
            Some(b) if b != 0.0 => js_number(b),
            _ => "--".to_string(),
        };
        let khz = match now.samplerate {
            Some(rate) if rate != 0.0 => format::khz(rate),
            _ => "--".to_string(),
        };
        let channels = now.channels.unwrap_or(0.0);
        if self.station.0 != now.station {
            self.station = (now.station.clone(), crate::text::shown(&now.station));
        }
        let station = self.station.1.clone();
        // The column (58 px of content) is centred against the screen (53 px + the canvas): at
        // scale 1 that is 5.5 px down, which Chromium paints from 6 (boxes and the text's
        // baseline round halves up) and GPUI's snapping from 5. The offset is given rounded the
        // way Chromium rounds it, so everything in the column lands on Chromium's pixels.
        let (_, canvas_height) = qust_ui::AnalyzerView::css_size(window);
        let free = (53.0 + canvas_height.as_f32() - 58.0) / 2.0;
        let column_top = qust_fluent::paint::snap_half_up(free, window.scale_factor());
        let now_playing = div()
            .flex()
            .flex_1()
            .flex_col()
            .self_start()
            .mt(px(column_top))
            .gap(px(3.0))
            .min_w_0()
            .font_text()
            // As wide as the now-playing column, cut with Chromium's ellipsis.
            .child(line(station).ellipsis().w_full().type_size(11.0, 14.0).text_color(theme.text_2))
            .child(div().w_full().h(px(20.0)).child(portal_slot(root, "title")))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(8.0))
                    .mt(px(2.0))
                    .type_size(11.0, 16.0)
                    .text_color(theme.text_2)
                    .whitespace_nowrap()
                    .child(spec(kbps, "kbps", cx))
                    .child(spec(khz, "kHz", cx))
                    .child(div().flex().gap(px(3.0)).ml_auto().child(channel_badge("Mono", channels == 1.0, &theme)).child(channel_badge("Stereo", channels >= 2.0, &theme))),
            );

        surface(Frame::island(&theme)).size_full().flex().items_center().gap(px(14.0)).p(px(10.0)).child(screen).child(now_playing)
    }
}
