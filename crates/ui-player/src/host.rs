//! What the player window needs from the rest of the app (the page's `invoke` calls that weren't
//! player commands, and the native UI), as a trait the app implements. Nothing here opens OS UI
//! by itself: the app does, through `qust-shell` (and tests record the calls instead).

use gpui::{App, Window};
use qust_shell::menu::Menu;

/// `open_panel(name)`
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Panel {
    Settings,
    Info,
}

/// What a native menu hands back: the chosen item's id (None when dismissed), on the UI thread.
pub type MenuDone = Box<dyn FnOnce(Option<u32>, &mut App)>;

pub trait PlayerHost: 'static {
    /// Opens (or brings to the front) Settings or Stream information. The view has already
    /// closed its dialog.
    fn open_panel(&self, panel: Panel, window: &mut Window, cx: &mut App);

    /// The Playlist button: open or close the drawer (`set_playlist_open`: save `playlist-open`
    /// and animate the window height). The view has already flipped its button.
    fn set_drawer(&self, open: bool, window: &mut Window, cx: &mut App);

    /// Add file (`playlist_add_files`): the native picker ("Add to playlist"), then add what was
    /// picked, then select and reveal it in the drawer, opening it if needed. Never call the
    /// picker from inside this call: it is a GPUI event handler (see `qust_shell::menu`).
    fn add_files(&self, window: &mut Window, cx: &mut App);

    /// Add URL succeeded: the drawer selects and reveals these entries (`selectAdded`), opening
    /// itself if closed.
    fn added(&self, ids: Vec<u64>, window: &mut Window, cx: &mut App);

    /// Shows a native menu at the mouse cursor, then calls `done` with the choice on the UI
    /// thread. Implement it with `qust_shell::menu::popup(hwnd, menu, Position::Cursor, ..)`,
    /// which defers the menu until GPUI's event handling is over (never show it from here).
    fn show_menu(&self, menu: Menu, window: &mut Window, cx: &mut App, done: MenuDone);
}
