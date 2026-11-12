//! Every SVG symbol of the pages (`index.html`, `settings.html`; `info.html` has none), as GPUI
//! assets. The markup is the page's, symbol for symbol; `<use href="#i-speaker">` is inlined and
//! `currentColor` is black (GPUI draws an SVG as a mask in the element's text color, which is
//! what `fill: currentColor` did). The Settings page has its own chevron (stroke 1.2 instead of
//! 1.3) and info icon (r 5.8 instead of 6.2): they are separate icons here.
//!
//! The app registers [`Assets`] (`Application::with_assets(qust_fluent::Assets)`), and an icon is
//! drawn with [`icon`]`(Icon::Play, 16.)`, colored by `text_color`.

use gpui::{px, svg, AssetSource, SharedString, Styled, Svg};
use std::borrow::Cow;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Icon {
    // index.html
    Play,
    Pause,
    Stop,
    Eject,
    Plus,
    Speaker,
    Volume0,
    Volume1,
    Volume2,
    Settings,
    Info,
    Bolt,
    List,
    Chevron,
    Note,
    Link,
    Radio,
    Balance,
    // settings.html
    SettingsChevron,
    Theme,
    Bars,
    Autoplay,
    Keyboard,
    Update,
    Clock,
    Tray,
    Signal,
    SettingsInfo,
}

const SPEAKER: &str = r#"<path d="M7.7 2.7 4.6 5.3H2.8a.8.8 0 0 0-.8.8v3.8c0 .4.4.8.8.8h1.8l3.1 2.6a.5.5 0 0 0 .8-.4V3.1a.5.5 0 0 0-.8-.4z" />"#;

impl Icon {
    pub const ALL: [Icon; 28] = [
        Icon::Play,
        Icon::Pause,
        Icon::Stop,
        Icon::Eject,
        Icon::Plus,
        Icon::Speaker,
        Icon::Volume0,
        Icon::Volume1,
        Icon::Volume2,
        Icon::Settings,
        Icon::Info,
        Icon::Bolt,
        Icon::List,
        Icon::Chevron,
        Icon::Note,
        Icon::Link,
        Icon::Radio,
        Icon::Balance,
        Icon::SettingsChevron,
        Icon::Theme,
        Icon::Bars,
        Icon::Autoplay,
        Icon::Keyboard,
        Icon::Update,
        Icon::Clock,
        Icon::Tray,
        Icon::Signal,
        Icon::SettingsInfo,
    ];

    /// The symbol's id in the page (`#i-…`), as the layout files name it.
    pub fn symbol(self) -> &'static str {
        match self {
            Icon::Play => "i-play",
            Icon::Pause => "i-pause",
            Icon::Stop => "i-stop",
            Icon::Eject => "i-eject",
            Icon::Plus => "i-plus",
            Icon::Speaker => "i-speaker",
            Icon::Volume0 => "i-vol-0",
            Icon::Volume1 => "i-vol-1",
            Icon::Volume2 => "i-vol-2",
            Icon::Settings => "i-settings",
            Icon::Info => "i-info",
            Icon::Bolt => "i-bolt",
            Icon::List => "i-list",
            Icon::Chevron => "i-chevron",
            Icon::Note => "i-note",
            Icon::Link => "i-link",
            Icon::Radio => "i-radio",
            Icon::Balance => "i-balance",
            Icon::SettingsChevron => "i-chevron",
            Icon::Theme => "i-theme",
            Icon::Bars => "i-bars",
            Icon::Autoplay => "i-autoplay",
            Icon::Keyboard => "i-keyboard",
            Icon::Update => "i-update",
            Icon::Clock => "i-clock",
            Icon::Tray => "i-tray",
            Icon::Signal => "i-signal",
            Icon::SettingsInfo => "i-info",
        }
    }

    /// The asset path GPUI loads it by.
    pub fn path(self) -> &'static str {
        match self {
            Icon::Play => "qust/play.svg",
            Icon::Pause => "qust/pause.svg",
            Icon::Stop => "qust/stop.svg",
            Icon::Eject => "qust/eject.svg",
            Icon::Plus => "qust/plus.svg",
            Icon::Speaker => "qust/speaker.svg",
            Icon::Volume0 => "qust/vol-0.svg",
            Icon::Volume1 => "qust/vol-1.svg",
            Icon::Volume2 => "qust/vol-2.svg",
            Icon::Settings => "qust/settings.svg",
            Icon::Info => "qust/info.svg",
            Icon::Bolt => "qust/bolt.svg",
            Icon::List => "qust/list.svg",
            Icon::Chevron => "qust/chevron.svg",
            Icon::Note => "qust/note.svg",
            Icon::Link => "qust/link.svg",
            Icon::Radio => "qust/radio.svg",
            Icon::Balance => "qust/balance.svg",
            Icon::SettingsChevron => "qust/settings-chevron.svg",
            Icon::Theme => "qust/theme.svg",
            Icon::Bars => "qust/bars.svg",
            Icon::Autoplay => "qust/autoplay.svg",
            Icon::Keyboard => "qust/keyboard.svg",
            Icon::Update => "qust/update.svg",
            Icon::Clock => "qust/clock.svg",
            Icon::Tray => "qust/tray.svg",
            Icon::Signal => "qust/signal.svg",
            Icon::SettingsInfo => "qust/settings-info.svg",
        }
    }

    /// The symbol's content, as in the page.
    pub fn body(self) -> Cow<'static, str> {
        let s = |text: &'static str| Cow::Borrowed(text);
        match self {
            Icon::Play => s(r#"<path d="M5 3.3v9.4a.8.8 0 0 0 1.2.7l7.5-4.7a.8.8 0 0 0 0-1.4L6.2 2.6A.8.8 0 0 0 5 3.3z" />"#),
            Icon::Pause => s(r#"<rect x="3.5" y="2.5" width="3" height="11" rx="1" /><rect x="9.5" y="2.5" width="3" height="11" rx="1" />"#),
            Icon::Stop => s(r#"<rect x="3" y="3" width="10" height="10" rx="1.5" />"#),
            Icon::Eject => s(r#"<path d="M7.3 3.4a.9.9 0 0 1 1.4 0l4 4.9a.6.6 0 0 1-.5 1H3.8a.6.6 0 0 1-.5-1z" /><rect x="3" y="10.6" width="10" height="2" rx=".8" />"#),
            Icon::Plus => s(r#"<path d="M8 3v10M3 8h10" fill="none" stroke="black" stroke-width="1.3" stroke-linecap="round" />"#),
            Icon::Speaker => s(SPEAKER),
            Icon::Volume0 => Cow::Owned(format!(r#"{SPEAKER}<path d="m10.5 6 3.5 4m0-4-3.5 4" fill="none" stroke="black" stroke-width="1.2" stroke-linecap="round" />"#)),
            Icon::Volume1 => Cow::Owned(format!(r#"{SPEAKER}<path d="M10.6 5.8a3 3 0 0 1 0 4.4" fill="none" stroke="black" stroke-width="1.2" stroke-linecap="round" />"#)),
            Icon::Volume2 => Cow::Owned(format!(r#"{SPEAKER}<path d="M10.6 5.8a3 3 0 0 1 0 4.4M12.4 4a5.6 5.6 0 0 1 0 8" fill="none" stroke="black" stroke-width="1.2" stroke-linecap="round" />"#)),
            Icon::Settings => s(r#"<circle cx="8" cy="8" r="5.9" fill="none" stroke="black" stroke-width="2" stroke-dasharray="2.317 2.317" /><circle cx="8" cy="8" r="3.9" fill="none" stroke="black" stroke-width="2.2" />"#),
            Icon::Info => s(r#"<circle cx="8" cy="8" r="6.2" fill="none" stroke="black" stroke-width="1.3" /><path d="M8 7.2v4.2" fill="none" stroke="black" stroke-width="1.5" stroke-linecap="round" /><circle cx="8" cy="4.9" r=".95" />"#),
            Icon::Bolt => s(r#"<path d="M9.2 1.6 3.6 9h3.6l-1 5.4L12.4 7H8.8z" />"#),
            Icon::List => s(r#"<path d="M5.5 4h8M5.5 8h8M5.5 12h8" fill="none" stroke="black" stroke-width="1.3" stroke-linecap="round" /><circle cx="2.6" cy="4" r=".95" /><circle cx="2.6" cy="8" r=".95" /><circle cx="2.6" cy="12" r=".95" />"#),
            Icon::Chevron => s(r#"<path d="m4 6 4 4 4-4" fill="none" stroke="black" stroke-width="1.3" stroke-linecap="round" stroke-linejoin="round" />"#),
            Icon::Note => s(r#"<path d="M6.5 11.5V3.6l6-1.3v7.4" fill="none" stroke="black" stroke-width="1.3" stroke-linejoin="round" /><ellipse cx="4.6" cy="11.6" rx="1.9" ry="1.5" /><ellipse cx="10.6" cy="9.8" rx="1.9" ry="1.5" />"#),
            Icon::Link => s(r#"<path d="M6.6 9.4a2.6 2.6 0 0 0 3.7 0l2.4-2.4a2.6 2.6 0 0 0-3.7-3.7l-.9.9M9.4 6.6a2.6 2.6 0 0 0-3.7 0L3.3 9a2.6 2.6 0 0 0 3.7 3.7l.9-.9" fill="none" stroke="black" stroke-width="1.3" stroke-linecap="round" />"#),
            Icon::Radio => s(r#"<circle cx="8" cy="8" r="1.6" /><path d="M5.2 5.2a4 4 0 0 0 0 5.6M10.8 5.2a4 4 0 0 1 0 5.6M3.1 3.1a7 7 0 0 0 0 9.8M12.9 3.1a7 7 0 0 1 0 9.8" fill="none" stroke="black" stroke-width="1.2" stroke-linecap="round" />"#),
            Icon::Balance => s(r#"<path d="M2 8h12" fill="none" stroke="black" stroke-width="1.2" stroke-linecap="round" /><path d="M2.5 5v6M13.5 5v6" fill="none" stroke="black" stroke-width="1.2" stroke-linecap="round" /><circle cx="8" cy="8" r="2.2" />"#),
            Icon::SettingsChevron => s(r#"<path d="m4 6 4 4 4-4" fill="none" stroke="black" stroke-width="1.2" stroke-linecap="round" stroke-linejoin="round" />"#),
            Icon::Theme => s(r#"<circle cx="8" cy="8" r="5.8" fill="none" stroke="black" stroke-width="1.2" /><path d="M8 2.2a5.8 5.8 0 0 1 0 11.6z" />"#),
            Icon::Bars => s(r#"<rect x="2" y="8" width="2.4" height="6" rx=".6" /><rect x="6.8" y="3" width="2.4" height="11" rx=".6" /><rect x="11.6" y="6" width="2.4" height="8" rx=".6" />"#),
            Icon::Autoplay => s(r#"<circle cx="8" cy="8" r="5.8" fill="none" stroke="black" stroke-width="1.2" /><path d="M6.6 5.6v4.8a.4.4 0 0 0 .6.3l3.6-2.4a.4.4 0 0 0 0-.6L7.2 5.3a.4.4 0 0 0-.6.3z" />"#),
            Icon::Keyboard => s(r#"<rect x="1.6" y="3.6" width="12.8" height="8.8" rx="1.6" fill="none" stroke="black" stroke-width="1.2" /><path d="M4 6.4h.01M6.7 6.4h.01M9.3 6.4h.01M12 6.4h.01M4 9.6h8" fill="none" stroke="black" stroke-width="1.3" stroke-linecap="round" />"#),
            Icon::Update => s(r#"<path d="M13.2 6.6A5.4 5.4 0 0 0 3.3 5.2M2.8 9.4a5.4 5.4 0 0 0 9.9 1.4" fill="none" stroke="black" stroke-width="1.2" stroke-linecap="round" /><path d="M3.2 2.4v2.9h2.9M12.8 13.6v-2.9H9.9" fill="none" stroke="black" stroke-width="1.2" stroke-linecap="round" stroke-linejoin="round" />"#),
            Icon::Clock => s(r#"<circle cx="8" cy="8" r="5.8" fill="none" stroke="black" stroke-width="1.2" /><path d="M8 4.8V8l2.2 1.4" fill="none" stroke="black" stroke-width="1.2" stroke-linecap="round" stroke-linejoin="round" />"#),
            Icon::Tray => s(r#"<rect x="2" y="3" width="12" height="10" rx="1.6" fill="none" stroke="black" stroke-width="1.2" /><path d="M2.6 10h3l.9 1.3h3l.9-1.3h3" fill="none" stroke="black" stroke-width="1.2" stroke-linejoin="round" />"#),
            Icon::Signal => s(r#"<circle cx="8" cy="9" r="1.1" /><path d="M5.6 6.6a3.4 3.4 0 0 0 0 4.8M10.4 6.6a3.4 3.4 0 0 1 0 4.8M3.6 4.6a6.2 6.2 0 0 0 0 8.8M12.4 4.6a6.2 6.2 0 0 1 0 8.8" fill="none" stroke="black" stroke-width="1.2" stroke-linecap="round" />"#),
            Icon::SettingsInfo => s(r#"<circle cx="8" cy="8" r="5.8" fill="none" stroke="black" stroke-width="1.2" /><path d="M8 7.3v4" fill="none" stroke="black" stroke-width="1.3" stroke-linecap="round" /><circle cx="8" cy="5.1" r=".85" />"#),
        }
    }

    /// A whole SVG document.
    pub fn svg(self) -> String {
        format!(r#"<svg xmlns="http://www.w3.org/2000/svg" width="16" height="16" viewBox="0 0 16 16" fill="black">{}</svg>"#, self.body())
    }

    fn by_path(path: &str) -> Option<Icon> {
        Icon::ALL.into_iter().find(|icon| icon.path() == path)
    }
}

/// An icon at `size` CSS px (the page's `svg { width: 16px; height: 16px }`, or the class's
/// size), drawn in the current text color unless `text_color` is set on it.
pub fn icon(icon: Icon, size: f32) -> Svg {
    svg().path(icon.path()).size(px(size)).flex_none()
}

/// The icons as a GPUI asset source.
pub struct Assets;

impl AssetSource for Assets {
    fn load(&self, path: &str) -> gpui::Result<Option<Cow<'static, [u8]>>> {
        Ok(Icon::by_path(path).map(|icon| Cow::Owned(icon.svg().into_bytes())))
    }

    fn list(&self, path: &str) -> gpui::Result<Vec<SharedString>> {
        Ok(Icon::ALL.iter().filter(|icon| icon.path().starts_with(path)).map(|icon| SharedString::from(icon.path())).collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Every symbol of the pages is here, with the page's markup.
    #[test]
    fn every_symbol_of_the_pages() {
        let root = concat!(env!("CARGO_MANIFEST_DIR"), "/../../../reference/page/");
        for (page, icons) in [("index.html", &Icon::ALL[..18]), ("settings.html", &Icon::ALL[18..])] {
            let html = std::fs::read_to_string(format!("{root}{page}")).expect(page);
            let symbols = html.matches("<symbol id=\"").count();
            assert_eq!(symbols, icons.len(), "{page}");
            for icon in icons {
                let start = html.find(&format!("<symbol id=\"{}\"", icon.symbol())).unwrap_or_else(|| panic!("{} in {page}", icon.symbol()));
                let body_start = start + html[start..].find('>').unwrap() + 1;
                let body = &html[body_start..body_start + html[body_start..].find("</symbol>").unwrap()];
                let expected = body.replace("currentColor", "black").replace(r##"<use href="#i-speaker" />"##, SPEAKER);
                assert_eq!(icon.body(), expected, "{}", icon.symbol());
            }
        }
    }
}
