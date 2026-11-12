//! What the panels need from outside: the OS, the shell and the player, behind one trait the app
//! implements (with `qust-shell` and `qust_ui::Bridge`) and the tests implement with a recording
//! fake ([`crate::fake::FakeServices`]).
//!
//! Everything else the windows show comes from `qust_ui::Models` (`settings`, `shortcuts`,
//! `update`, `background`, `status`): the app keeps those current and the views re-render when
//! they change. The methods mirror the commands the old pages invoked (named in each method).

use gpui::{App, Window};
use qust_shell::menu::{Menu, Position};
use qust_shell::recorder::Recorded;
use qust_ui::{Background, StreamInfo};
use serde_json::Value;
use std::any::Any;
use std::future::Future;
use std::pin::Pin;

/// What a pending stream-details request resolves to (None when the player went away).
pub type InfoRequest = Pin<Box<dyn Future<Output = Option<StreamInfo>>>>;

/// Called with the id of the chosen menu item (None when the menu was dismissed), on the UI
/// thread with the app.
pub type MenuDone = Box<dyn FnOnce(Option<u32>, &mut App)>;

/// The services both panels use. Every method is called on the UI thread, from inside GPUI's event
/// handling: none may block for long or open OS UI synchronously (menus go through
/// `qust_shell::menu::popup`, which defers them).
pub trait PanelServices {
    /// Seconds since 1970 (`Date.now()`), for "Last checked … ago".
    fn now(&self) -> u64;

    /// The app's version ("1.1.5"), for the "Qust 1.1.5" title.
    fn version(&self) -> String;

    /// `saveData(key, value)`: writes a setting to the store; the player follows the keys it
    /// watches (`visualizer-colors`, `reconnect-minutes`, `autoplay`): with the bridge,
    /// `player.write_setting(key, Some(value))`, the store's watcher calling
    /// `qust_player::Player::setting_changed`.
    fn save_setting(&self, key: &str, value: Value, cx: &mut App);

    /// `emit('settings-changed', { key, value })` for the other windows: with the bridge,
    /// `models.apply_setting(key, &value, cx)`. Keys as settings.js sent them: `colors`,
    /// `autoplay`, `reconnectMinutes`.
    fn broadcast_setting(&self, key: &str, value: Value, cx: &mut App);

    /// `set_theme`: saves `theme` ("system", "light", "dark") and applies it to every window
    /// (`qust_ui::theme::apply`, `qust_shell::theme::set_app_theme`).
    fn set_theme(&self, theme: &str, cx: &mut App);

    /// `pause_shortcuts`: unregisters every global shortcut while one is recorded (`true`), and
    /// registers them again afterwards (`false`). Keep `models.shortcuts` current (the shell's
    /// `Shortcuts::on_change` reports the resume).
    fn pause_shortcuts(&self, paused: bool, cx: &mut App);

    /// `set_shortcut(action, accelerator)`: `action` is "play", "stop" or "toggle"; an empty
    /// accelerator resets to the default. The error is the text Settings shows
    /// (`qust_shell::shortcuts::Shortcuts::set_shortcut`'s). Keep `models.shortcuts` current.
    fn set_shortcut(&self, action: &str, accelerator: &str, cx: &mut App) -> Result<(), String>;

    /// Starts reading key presses from this window's messages
    /// (`qust_shell::window::KeyCapture::start(hwnd, on_key)`) and returns what keeps the capture
    /// running: Settings drops it to stop. `on_key` may be called from inside the window
    /// procedure; it only queues the key. None where there is no native capture (the view then
    /// records from GPUI's key events, `qust_shell::recorder::record_gpui`).
    fn capture_keys(&self, window: &Window, on_key: Box<dyn FnMut(Recorded)>) -> Option<Box<dyn Any>>;

    /// `set_background(enabled)`: the new state (`qust_shell::background::set_background`), or
    /// the error text Settings shows. Keep `models.background` current.
    fn set_background(&self, enabled: bool, cx: &mut App) -> Result<Background, String>;

    /// `check_for_updates`: starts a check (on a background thread); the status arrives through
    /// `models.update`.
    fn check_for_updates(&self, cx: &mut App);

    /// `install_update`: installs the downloaded update (on a background thread) and quits.
    fn install_update(&self, cx: &mut App);

    /// Shows a native popup menu for `window` at `position` (`qust_shell::menu::popup`, which
    /// shows it once the current event is handled) and calls `done` with the choice.
    fn popup_menu(&self, window: &Window, menu: Menu, position: Position, done: MenuDone, cx: &mut App);

    /// Puts `text` on the clipboard (Copy in the Stream information menu).
    fn copy_text(&self, text: String, cx: &mut App);

    /// `stream_info(url)`: the details of what plays, by its bare URL or path (with the bridge,
    /// `player.stream_info(url).get()`).
    fn stream_info(&self, url: String, cx: &mut App) -> InfoRequest;

    /// Closes the panel (Esc). With the bridge: `window.remove_window()`; the app's close handler
    /// runs as for the close button.
    fn close_window(&self, window: &mut Window, cx: &mut App);
}
