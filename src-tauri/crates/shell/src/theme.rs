// The app's theme (`theme` in the store: system / light / dark) as far as the OS is concerned:
// whether the system uses dark mode, the window background, the title bars of the windows that
// follow it (`window::follow_theme`), and the native menus.
//
// Menus: with "system" they follow the system's dark mode, as Tauri's did (uxtheme AllowDark).
// With a forced theme they follow the app (ForceDark / ForceLight); the Tauri app's menus ignored
// a forced theme and followed the system (see the report).

use std::sync::atomic::{AtomicU8, Ordering};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Theme {
    #[default]
    System,
    Light,
    Dark,
}

impl Theme {
    // The stored value; anything else is "system", as stored_theme() read it.
    pub fn from_setting(value: Option<&str>) -> Theme {
        match value {
            Some("light") => Theme::Light,
            Some("dark") => Theme::Dark,
            _ => Theme::System,
        }
    }

    pub fn from_store(store: &qust_player::Store) -> Theme {
        Theme::from_setting(store.get("theme").as_ref().and_then(|value| value.as_str()))
    }

    pub fn as_setting(self) -> &'static str {
        match self {
            Theme::System => "system",
            Theme::Light => "light",
            Theme::Dark => "dark",
        }
    }

    pub fn is_dark(self) -> bool {
        match self {
            Theme::System => system_dark(),
            Theme::Light => false,
            Theme::Dark => true,
        }
    }

    fn to_u8(self) -> u8 {
        match self {
            Theme::System => 0,
            Theme::Light => 1,
            Theme::Dark => 2,
        }
    }

    fn from_u8(value: u8) -> Theme {
        match value {
            1 => Theme::Light,
            2 => Theme::Dark,
            _ => Theme::System,
        }
    }
}

static APP_THEME: AtomicU8 = AtomicU8::new(0);

pub fn app_theme() -> Theme {
    Theme::from_u8(APP_THEME.load(Ordering::Relaxed))
}

// Applies a theme to everything here: the menus, and the title bars of the windows following it.
// Call it at startup with the stored theme, and when Settings changes it (after saving it).
pub fn set_app_theme(theme: Theme) {
    APP_THEME.store(theme.to_u8(), Ordering::Relaxed);
    crate::menu::apply_theme(theme);
    crate::window::retheme_all();
}

// The page's background (--bg), also the window's own background while the drawer uncovers it:
// #202020 dark, #f3f3f3 light.
pub fn background(dark: bool) -> (u8, u8, u8) {
    if dark {
        (0x20, 0x20, 0x20)
    } else {
        (0xf3, 0xf3, 0xf3)
    }
}

// Whether apps should be dark: AppsUseLightTheme = 0, as tao read it, and never in high contrast.
#[cfg(windows)]
pub fn system_dark() -> bool {
    use windows::Win32::UI::Accessibility::{HCF_HIGHCONTRASTON, HIGHCONTRASTW};
    use windows::Win32::UI::WindowsAndMessaging::{SystemParametersInfoW, SPI_GETHIGHCONTRAST, SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS};
    let light = windows_registry::CURRENT_USER
        .open(r"Software\Microsoft\Windows\CurrentVersion\Themes\Personalize")
        .and_then(|key| key.get_u32("AppsUseLightTheme"))
        .map(|value| value != 0)
        .unwrap_or(true);
    let mut contrast = HIGHCONTRASTW { cbSize: std::mem::size_of::<HIGHCONTRASTW>() as u32, ..Default::default() };
    let high_contrast = unsafe { SystemParametersInfoW(SPI_GETHIGHCONTRAST, contrast.cbSize, Some(&mut contrast as *mut _ as *mut _), SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS(0)) }.is_ok()
        && contrast.dwFlags.contains(HCF_HIGHCONTRASTON);
    !light && !high_contrast
}

#[cfg(not(windows))]
pub fn system_dark() -> bool {
    // macOS: GPUI's window appearance tells; there is nothing to read here without AppKit.
    false
}

// The user's accent (base, light, dark), as `window.__QUST__.accent` had it.
pub fn accent() -> Option<serde_json::Value> {
    qust_player::accent()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn settings() {
        assert_eq!(Theme::from_setting(Some("light")), Theme::Light);
        assert_eq!(Theme::from_setting(Some("dark")), Theme::Dark);
        assert_eq!(Theme::from_setting(Some("system")), Theme::System);
        assert_eq!(Theme::from_setting(Some("blue")), Theme::System);
        assert_eq!(Theme::from_setting(None), Theme::System);
        assert!(Theme::Dark.is_dark());
        assert!(!Theme::Light.is_dark());
        assert_eq!(background(true), (0x20, 0x20, 0x20));
        assert_eq!(background(false), (0xf3, 0xf3, 0xf3));
        for theme in [Theme::System, Theme::Light, Theme::Dark] {
            assert_eq!(Theme::from_u8(theme.to_u8()), theme);
            assert_eq!(Theme::from_setting(Some(theme.as_setting())), theme);
        }
    }
}
