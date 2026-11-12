//! Which theme is shown: the stored `theme` setting (`system` / `light` / `dark`), the system's
//! light/dark mode when it is `system`, and the system accent.
//!
//! [`apply`] installs `qust_fluent::Theme` for the current choice. [`follow`] makes a window keep
//! it current: when the system theme changes (GPUI's appearance observer) and when the window is
//! activated (the accent is re-read then too, since an accent change alone doesn't always change
//! the appearance GPUI reports).
//!
//! **Seam for `qust-shell` (the title bar).** GPUI sets each window's DWM dark mode from the
//! *system* theme when it creates the window and when the system theme changes; with a forced
//! theme the shell has to override it. Observe the theme global and apply it to every window:
//! `cx.observe_global::<qust_fluent::Theme>(|cx| { let dark = qust_fluent::theme(cx).dark; … })`
//! (fires on every `Theme::install`, including accent changes), and also right after a
//! window opens and after GPUI's own appearance change (use [`follow`]'s callback,
//! [`on_applied`]). The shell can also call [`refresh_accent`] on `WM_SETTINGCHANGE`
//! ("ImmersiveColorSet") / `WM_DWMCOLORIZATIONCOLORCHANGED`, which it sees and GPUI doesn't
//! surface. On macOS, a forced theme is applied to the window chrome here through
//! `App::set_window_appearance`.

use gpui::{App, Context, Global, Subscription, Window, WindowAppearance};
use qust_fluent::{Accent, Theme};

/// The `theme` setting.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ThemeChoice {
    #[default]
    System,
    Light,
    Dark,
}

impl ThemeChoice {
    pub fn parse(value: &str) -> ThemeChoice {
        match value {
            "light" => ThemeChoice::Light,
            "dark" => ThemeChoice::Dark,
            _ => ThemeChoice::System,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            ThemeChoice::System => "system",
            ThemeChoice::Light => "light",
            ThemeChoice::Dark => "dark",
        }
    }
}

#[derive(Default)]
struct Choice {
    choice: ThemeChoice,
    /// A fixed accent (captures), instead of the system's.
    accent: Option<Accent>,
    listeners: Vec<Box<dyn Fn(&mut App)>>,
}

impl Global for Choice {}

/// The system accent: `AccentPalette` entries 3 / 1 / 4 on Windows (what `qust_player::accent()`
/// reads), else the CSS fallback.
pub fn system_accent() -> Accent {
    qust_player::accent()
        .and_then(|value| Accent::parse(value["base"].as_str()?, value["light"].as_str()?, value["dark"].as_str()?))
        .unwrap_or_else(Accent::css_fallback)
}

fn is_dark(appearance: WindowAppearance) -> bool {
    matches!(appearance, WindowAppearance::Dark | WindowAppearance::VibrantDark)
}

/// Installs the theme for `choice` (and remembers it).
pub fn apply(choice: ThemeChoice, cx: &mut App) {
    cx.default_global::<Choice>().choice = choice;
    install(cx);
}

/// Pins the accent (the capture harness uses the reference's), or follows the system again.
pub fn set_accent(accent: Option<Accent>, cx: &mut App) {
    cx.default_global::<Choice>().accent = accent;
    install(cx);
}

/// The current choice.
pub fn choice(cx: &App) -> ThemeChoice {
    cx.try_global::<Choice>().map_or(ThemeChoice::System, |c| c.choice)
}

/// Re-reads the system theme and accent and reinstalls the theme if anything changed.
pub fn refresh_accent(cx: &mut App) {
    install(cx);
}

/// Called after every install (the shell's title bar hook).
pub fn on_applied(listener: impl Fn(&mut App) + 'static, cx: &mut App) {
    cx.default_global::<Choice>().listeners.push(Box::new(listener));
}

fn install(cx: &mut App) {
    let state = cx.default_global::<Choice>();
    let choice = state.choice;
    let accent = state.accent.unwrap_or_else(system_accent);
    let dark = match choice {
        ThemeChoice::System => is_dark(cx.window_appearance()),
        ThemeChoice::Light => false,
        ThemeChoice::Dark => true,
    };
    #[cfg(target_os = "macos")]
    cx.set_window_appearance(match choice {
        ThemeChoice::System => None,
        ThemeChoice::Light => Some(WindowAppearance::Light),
        ThemeChoice::Dark => Some(WindowAppearance::Dark),
    });
    let theme = Theme::new(dark, accent);
    if cx.try_global::<Theme>() != Some(&theme) {
        theme.install(cx);
    }
    let listeners = std::mem::take(&mut cx.default_global::<Choice>().listeners);
    for listener in &listeners {
        listener(cx);
    }
    let state = cx.default_global::<Choice>();
    let added = std::mem::take(&mut state.listeners);
    state.listeners = listeners;
    state.listeners.extend(added);
}

/// Keeps `window` on the current theme: system theme changes and activation re-check it.
/// Keep the subscriptions with the window's view.
pub fn follow<V: 'static>(window: &mut Window, cx: &mut Context<V>) -> [Subscription; 2] {
    [
        cx.observe_window_appearance(window, |_, _, cx| install(cx)),
        cx.observe_window_activation(window, |_, window, cx| {
            if window.is_window_active() {
                install(cx)
            }
        }),
    ]
}
