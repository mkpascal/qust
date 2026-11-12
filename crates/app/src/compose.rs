//! The player window's content, composed from the window crates: the `PlayerView` (860 × 587,
//! its part at the top and an empty slot for the drawer below) as the `WindowRoot`'s content,
//! then the drawer mounted as the root's live layers ("drawer" and the playing row's equalizer),
//! before any overlay can be shown, so the dialog and the cartridge drag ghost cover it. One
//! [`WindowHost`] is the host of both: what one asks of the other goes through it, and what
//! leaves the window goes to a [`HostEnv`] (the app, or a capture's stand-in).

use gpui::{App, AppContext, Entity, WeakEntity, Window};
use qust_shell::menu::Menu;
use qust_ui::{Models, WindowRoot};
use qust_ui_player::{Commands, MenuDone, Panel as PlayerPanel, PlayerHost, PlayerView};
use qust_ui_playlist::{Drawer, DrawerHost, Panel as DrawerPanel, PlaylistCommands};
use std::cell::OnceCell;
use std::path::PathBuf;
use std::rc::Rc;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PanelKind {
    Settings,
    Info,
}

/// What the player window needs from outside it.
pub trait HostEnv: 'static {
    /// Opens Settings or Stream information (or brings it to the front).
    fn open_panel(&self, panel: PanelKind, cx: &mut App);
    /// The drawer opened or closed: save `playlist-open`, grow or shrink the window.
    fn drawer_moved(&self, open: bool, window: &mut Window, cx: &mut App);
    /// A native menu at the cursor; `done` with the choice on the UI thread, later.
    fn show_menu(&self, menu: Menu, window: &mut Window, done: MenuDone, cx: &mut App);
    /// The Add file picker; `done` with the paths on the UI thread, later (nothing when
    /// cancelled).
    fn pick_files(&self, window: &mut Window, done: Box<dyn FnOnce(Vec<PathBuf>, &mut App)>, cx: &mut App);
}

/// The host of the player view and the drawer.
pub struct WindowHost {
    env: Rc<dyn HostEnv>,
    view: OnceCell<WeakEntity<PlayerView>>,
    drawer: OnceCell<WeakEntity<Drawer>>,
}

impl WindowHost {
    fn view(&self) -> Option<Entity<PlayerView>> {
        self.view.get().and_then(WeakEntity::upgrade)
    }

    fn drawer(&self) -> Option<Entity<Drawer>> {
        self.drawer.get().and_then(WeakEntity::upgrade)
    }

    // Add file (the Playlist bar's button and the drawer's menu): the picker, then the drawer
    // adds what was picked, selects and reveals it, opening itself if it was closed.
    fn add_files(&self, window: &mut Window, cx: &mut App) {
        let drawer = self.drawer.get().cloned();
        let handle = window.window_handle();
        self.env.pick_files(
            window,
            Box::new(move |paths, cx| {
                if paths.is_empty() {
                    return;
                }
                let Some(drawer) = drawer.and_then(|d| d.upgrade()) else { return };
                let _ = handle.update(cx, |_, window, cx| drawer.update(cx, |drawer, cx| drawer.add_paths(paths, None, window, cx)));
            }),
            cx,
        );
    }
}

impl PlayerHost for WindowHost {
    fn open_panel(&self, panel: PlayerPanel, _window: &mut Window, cx: &mut App) {
        let kind = match panel {
            PlayerPanel::Settings => PanelKind::Settings,
            PlayerPanel::Info => PanelKind::Info,
        };
        let env = self.env.clone();
        // Opening a window from inside the click's handling: after it.
        cx.defer(move |cx| env.open_panel(kind, cx));
    }

    fn set_drawer(&self, open: bool, window: &mut Window, cx: &mut App) {
        if let Some(drawer) = self.drawer() {
            drawer.update(cx, |drawer, cx| drawer.set_open(open, window, cx));
        }
        self.env.drawer_moved(open, window, cx);
    }

    fn add_files(&self, window: &mut Window, cx: &mut App) {
        WindowHost::add_files(self, window, cx);
    }

    fn added(&self, ids: Vec<u64>, window: &mut Window, cx: &mut App) {
        if let Some(drawer) = self.drawer() {
            drawer.update(cx, |drawer, cx| drawer.select_added(ids, window, cx));
        }
    }

    fn show_menu(&self, menu: Menu, window: &mut Window, cx: &mut App, done: MenuDone) {
        self.env.show_menu(menu, window, done, cx);
    }
}

impl DrawerHost for WindowHost {
    fn edit_cartridge(&self, slot: usize, window: &mut Window, cx: &mut App) {
        if let Some(view) = self.view() {
            view.update(cx, |view, cx| view.open_cartridge_dialog(slot, window, cx));
        }
    }

    fn edit_entry(&self, id: u64, window: &mut Window, cx: &mut App) {
        if let Some(view) = self.view() {
            view.update(cx, |view, cx| view.open_entry_dialog(id, window, cx));
        }
    }

    fn add_url(&self, window: &mut Window, cx: &mut App) {
        if let Some(view) = self.view() {
            view.update(cx, |view, cx| view.open_url_dialog(window, cx));
        }
    }

    fn add_files(&self, window: &mut Window, cx: &mut App) {
        WindowHost::add_files(self, window, cx);
    }

    fn open_panel(&self, panel: DrawerPanel, _window: &mut Window, cx: &mut App) {
        // The dialog closes first, as openSettings() / openInfo() did.
        if let Some(view) = self.view() {
            view.update(cx, |view, cx| view.close_dialog(cx));
        }
        let kind = match panel {
            DrawerPanel::Settings => PanelKind::Settings,
            DrawerPanel::StreamInformation => PanelKind::Info,
        };
        let env = self.env.clone();
        cx.defer(move |cx| env.open_panel(kind, cx));
    }

    fn show_menu(&self, menu: Menu, chosen: Box<dyn FnOnce(Option<u32>, &mut App)>, window: &mut Window, cx: &mut App) {
        self.env.show_menu(menu, window, chosen, cx);
    }

    fn open_drawer(&self, window: &mut Window, cx: &mut App) {
        if let Some(view) = self.view() {
            view.update(cx, |view, cx| view.set_drawer_open(true, cx));
        }
        self.env.drawer_moved(true, window, cx);
    }

    fn dialog_open(&self, cx: &App) -> bool {
        self.view().is_some_and(|view| view.read(cx).dialog_open(cx))
    }
}

/// The player window's parts.
#[derive(Clone)]
pub struct Composed {
    pub root: Entity<WindowRoot>,
    pub view: Entity<PlayerView>,
    pub drawer: Entity<Drawer>,
    pub host: Rc<WindowHost>,
}

/// Builds the player window's root (call it in `open_window`'s builder). The drawer starts open
/// or closed as `models.settings` says (`playlist-open`), like the Playlist button.
pub fn player_window(models: &Models, commands: Rc<dyn Commands>, playlist: Rc<dyn PlaylistCommands>, env: Rc<dyn HostEnv>, window: &mut Window, cx: &mut App) -> Composed {
    let placeholder: gpui::AnyView = cx.new(|_| gpui::EmptyView).into();
    let root = cx.new(|_| WindowRoot::new(placeholder));
    let host = Rc::new(WindowHost { env, view: OnceCell::new(), drawer: OnceCell::new() });
    let player_host: Rc<dyn PlayerHost> = host.clone();
    let view = cx.new(|cx| PlayerView::new(models, commands, player_host, root.downgrade(), window, cx));
    root.update(cx, |root, cx| root.set_content(view.clone(), cx));
    let open = models.settings.read(cx).playlist_open;
    let drawer_host: Rc<dyn DrawerHost> = host.clone();
    let drawer = cx.new(|cx| Drawer::new(models, playlist, drawer_host, open, cx));
    // Before any overlay: the dialog and the drag ghost must cover the drawer.
    qust_ui_playlist::mount(&drawer, &root, cx);
    let _ = host.view.set(view.downgrade());
    let _ = host.drawer.set(drawer.downgrade());
    Composed { root, view, drawer, host }
}
