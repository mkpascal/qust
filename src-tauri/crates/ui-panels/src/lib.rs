//! Qust's two secondary windows on GPUI: **Settings** (settings.html / settings.js) and **Stream
//! information** (info.html / info.js), rebuilt from the frozen pages in `reference/`.
//!
//! # Opening a panel (the app)
//!
//! ```ignore
//! let services: Rc<dyn PanelServices> = Rc::new(AppServices { .. });   // see below
//! let options = WindowOptions {
//!     window_bounds: Some(WindowBounds::Windowed(Bounds::new(position, size(px(settings::SIZE.0), px(settings::SIZE.1))))),
//!     titlebar: Some(TitlebarOptions { title: Some(settings::TITLE.into()), .. }),
//!     is_resizable: false, is_minimizable: false, show: false, focus: true, ..Default::default()
//! };
//! cx.open_window(options, |window, cx| settings_window(&models, services.clone(), window, cx).0)
//! ```
//!
//! Then, as for any panel: `set_owner(panel, player)`, `set_minimizable(panel, false)`, the
//! position from `panel_position(player, SIZE)`, `follow_theme`, `close_on_escape`, show. Both
//! views also close on Esc themselves through [`PanelServices::close_window`] (where the shell's
//! Esc hook is missing, as on macOS); while a shortcut is recorded Esc cancels the recording,
//! in both paths. When Settings closes, the app resumes the global shortcuts
//! (`shortcuts.set_paused(false)`); the view also does it when it is released mid-recording.
//!
//! | window | constructor | client size | title |
//! |---|---|---|---|
//! | Settings | [`settings_window`] → (`WindowRoot`, [`SettingsView`]) | 520 × 850 ([`settings::SIZE`]) | "Qust settings" |
//! | Stream information | [`info_window`] → (`WindowRoot`, [`InfoView`]) | 580 × 590 ([`info::SIZE`]) | "Stream information" |
//!
//! # What the views read and what they ask for
//!
//! They read `qust_ui::Models` and re-render when it changes: `settings` (the initial choices,
//! and the analyzer colors changed from the player), `shortcuts`, `update`, `background`
//! (Settings) and `status` (Stream information). The app keeps those current: the bridge does
//! `status` and `settings`; the app applies `Shortcuts::describe()` / `on_change`,
//! `UpdateService`'s `update-status` and `background::describe()` (as `qust_ui::Shortcuts`,
//! `UpdateStatus`, `Background`).
//!
//! Everything else goes through one trait, [`PanelServices`] (see its methods: settings writes
//! and broadcasts, the theme, the shortcut recorder, the background helper, the updater, native
//! menus, the clipboard, stream details, closing). The app implements it with `qust-shell` and the
//! bridge; [`fake::FakeServices`] is the recording fake the captures and tests use.
//!
//! **Bridge note.** `qust_ui::Bridge::start` doesn't install the store's watcher, so a setting
//! the Settings window writes (`visualizer-colors`, `reconnect-minutes`, `autoplay`) reaches the
//! player only if the app does what the Tauri app's main.rs did:
//! `store.watch(move |key, value| { let (key, value) = (key.to_string(), value.cloned()); player.ask(move |p| p.setting_changed(&key, value)); })`
//! (tests/end_to_end.rs does exactly that).
//!
//! **Before opening Settings**, apply `models.update`, `models.background` and
//! `models.shortcuts`: their defaults read as "not installed", "helper unavailable" and "no
//! shortcuts" (the page asked for them as it opened).
//!
//! # Verifying
//!
//! - `cargo run -p qust-ui-panels --features capture --example panels_capture -- [names] [--themes dark,light]`:
//!   every `settings-*` / `info-*` fixture rendered in a hidden window and compared with its
//!   reference render (`harness` replays the fixtures' actions).
//! - `cargo test -p qust-ui-panels --features capture --test behaviour`: every interaction against
//!   the recording fake. `--test end_to_end`: the real player on the null output.
//!
//! # Fidelity notes
//!
//! - Text is laid out as Chromium laid it out (kerning, baseline, ellipsis, letter-spacing,
//!   fallback fonts) by `qust_fluent::text`; single-line texts are `qust_fluent::Line`s.
//! - Right-to-left text: what the Stream information window shows of the stream (station, title,
//!   every value) and the error texts of Settings that can carry another program's message go
//!   through `qust_text::shown` when they change; tooltips, the clipboard and accessibility
//!   labels keep the logical text.
//!
//! - A Stream information value is selected whole by a right-click (then Copy, or Ctrl+C); the
//!   page also allowed selecting part of a value with the mouse, which plain GPUI text can't.
//! - The Reconnect menu lists "Until stopped" first, as the page's menu did (JavaScript orders an
//!   object's integer keys numerically), not last as BEHAVIOUR.md 3.2 lists it.

pub mod fake;
pub mod format;
#[cfg(feature = "capture")]
pub mod harness;
pub mod info;
pub mod services;
pub mod settings;

pub use info::{info_window, InfoTexts, InfoView};
pub use services::{InfoRequest, MenuDone, PanelServices};
pub use settings::{settings_window, ComboKind, SettingsView};
