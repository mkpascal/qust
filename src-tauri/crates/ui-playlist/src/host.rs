//! The drawer's two seams: what it asks of the window around it ([`DrawerHost`]) and the
//! player commands it sends ([`PlaylistCommands`], implemented for `qust_ui::Player`; tests
//! give a recording stand-in).

use gpui::{App, Window};
use qust_shell::menu::Menu;
use std::future::Future;
use std::path::PathBuf;
use std::pin::Pin;

/// The panels the drawer's menu opens (its last two items, as on every menu of the window).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Panel {
    StreamInformation,
    Settings,
}

/// What the drawer needs from the window it lives in. The drawer calls these **deferred**
/// (`Window::defer`), never from inside its own update, so an implementation may update the
/// drawer entity (e.g. `Drawer::set_open`) without a double borrow.
pub trait DrawerHost: 'static {
    /// "Edit cartridge…" (and nothing else: pinned rows don't eject): the dialog for `slot`.
    fn edit_cartridge(&self, slot: usize, window: &mut Window, cx: &mut App);
    /// "Edit…" on a track: the Edit track dialog for entry `id`.
    fn edit_entry(&self, id: u64, window: &mut Window, cx: &mut App);
    /// "Add URL…": the Add URL dialog. When it adds, hand the new ids to `Drawer::select_added`.
    fn add_url(&self, window: &mut Window, cx: &mut App);
    /// "Add file…": the file picker (`qust_shell::picker::pick_files`, off the UI thread); hand
    /// the chosen paths to `Drawer::add_paths(paths, None, ..)`.
    fn add_files(&self, window: &mut Window, cx: &mut App);
    /// "Stream information" / "Settings" (closing the dialog first, as the page did).
    fn open_panel(&self, panel: Panel, window: &mut Window, cx: &mut App);
    /// Shows a native menu at the cursor (`qust_shell::menu::popup(hwnd, menu,
    /// Position::Cursor, ..)`), then calls `chosen` on the UI thread with the item's id (None
    /// when dismissed). Never show it from inside a GPUI handler: `popup` defers it already.
    fn show_menu(&self, menu: Menu, chosen: Box<dyn FnOnce(Option<u32>, &mut App)>, window: &mut Window, cx: &mut App);
    /// The drawer opens itself (files dragged over the window, entries just added): update the
    /// Playlist button, save `playlist-open`, grow the window (`qust_shell::drawer::animate`).
    /// The drawer already counts itself open; calling `Drawer::set_open(true)` back is harmless.
    fn open_drawer(&self, window: &mut Window, cx: &mut App);
    /// Whether the dialog is open (file drops and the right-click menu are ignored then).
    fn dialog_open(&self, cx: &App) -> bool;
}

/// A future resolved on the UI thread.
pub type Pending<T> = Pin<Box<dyn Future<Output = Option<T>>>>;

/// The player commands the drawer sends (`qust_ui::Player`'s, by the names the page invoked).
pub trait PlaylistCommands: 'static {
    /// `cartridge_play(slot)`.
    fn play_cartridge(&self, slot: usize);
    /// `playlist_play(id)`.
    fn play_entry(&self, id: u64);
    /// `playlist_remove(ids)`.
    fn remove_entries(&self, ids: Vec<u64>);
    /// `playlist_move(ids, index)`: before `index` in the list as it is.
    fn move_entries(&self, ids: Vec<u64>, index: usize);
    /// `playlist_drop(paths, index)`: expanded off the UI thread; the ids added, after the
    /// `player-playlist` that shows them.
    fn add_paths(&self, paths: Vec<PathBuf>, index: Option<usize>) -> Pending<Vec<u64>>;
}

impl PlaylistCommands for qust_ui::Player {
    fn play_cartridge(&self, slot: usize) {
        self.select_slot(slot);
    }

    fn play_entry(&self, id: u64) {
        self.select_entry(id);
    }

    fn remove_entries(&self, ids: Vec<u64>) {
        qust_ui::Player::remove_entries(self, ids);
    }

    fn move_entries(&self, ids: Vec<u64>, index: usize) {
        // The answer only says the deck shows it; the playlist event carries the change.
        let _ = qust_ui::Player::move_entries(self, ids, index);
    }

    fn add_paths(&self, paths: Vec<PathBuf>, index: Option<usize>) -> Pending<Vec<u64>> {
        let reply = qust_ui::Player::add_paths(self, paths, index);
        Box::pin(reply.get())
    }
}
