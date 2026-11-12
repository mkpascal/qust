//! Qust's playlist drawer: the player window's lower part (y 267 to 587, 860 × 320), shown by
//! growing the window. The list island (the loaded cartridges pinned at the top, the divider,
//! the tracks or the empty hint, the scrollbar), the status line, and every interaction of the
//! page's playlist (BEHAVIOUR.md 2.7, 2.9): selection, keys, reordering by drag, files dropped
//! from Explorer, the right-click menu.
//!
//! # Wiring it (the integrator's part)
//!
//! ```ignore
//! let drawer = cx.new(|cx| Drawer::new(&bridge.models, Rc::new(bridge.player.clone()), host, open_at_start, cx));
//! // Mount it at the window root, not inside the player's content view (see below): two live
//! // layers, "drawer" and the playing row's equalizer above it. Add them before any overlay
//! // that must cover them (the dialog, the cartridge drag ghost).
//! qust_ui_playlist::mount(&drawer, &root, cx);
//! // …and in the player's content view, at x 0, y 267, 860 × 320: `portal_slot(root, "drawer")`.
//! // The Playlist button:            drawer.update(cx, |d, cx| d.set_open(open, window, cx));
//! // Add URL's dialog answered ids:  drawer.update(cx, |d, cx| d.select_added(ids, window, cx));
//! // Add file's picker chose paths:  drawer.update(cx, |d, cx| d.add_paths(paths, None, window, cx));
//! ```
//!
//! **What it needs** ([`DrawerHost`], called deferred so it may update the drawer):
//! `edit_cartridge(slot)`, `edit_entry(id)`, `add_url()`, `add_files()` (the dialog and the
//! picker), `open_panel(Panel)` (Stream information / Settings from the menu), `show_menu(menu,
//! chosen)` (a `qust_shell::menu::Menu` to show at the cursor through `qust_shell::menu::popup`;
//! `chosen(id)` back on the UI thread), `open_drawer()` (the drawer opened itself: the Playlist
//! button, `playlist-open`, the window's height animation), `dialog_open()` (drops and the menu
//! are ignored then). The player commands go through [`PlaylistCommands`], implemented for
//! `qust_ui::Player`.
//!
//! **What it offers**: the [`Drawer`] view (renders at whatever 860 × 320 box it is given,
//! independent of the window's height), [`Drawer::set_open`] / [`Drawer::is_open`],
//! [`Drawer::select_added`], [`Drawer::add_paths`], [`Drawer::focus_handle`] (Tab order: after
//! Add URL).
//!
//! **External file drops need no wiring.** GPUI turns a platform file drag into an internal drag
//! of [`gpui::ExternalPaths`] plus mouse moves (the button held) and a release on drop; an
//! element's `on_drag_move::<ExternalPaths>` sees every move window-wide, and a window-level
//! release listener sees the drop wherever it lands (over the player too: the end of the list).
//! The drawer registers both itself, also while closed (a drag over the window opens it), and
//! checks `dialog_open()` on each event.
//!
//! **Mount it at the window root.** In GPUI a dirty cached view re-renders its whole subtree
//! (`view.rs`, `prepaint_view` renders with `refreshing` set), and a notify marks every ancestor
//! dirty. The drawer re-renders on selection, scroll and list changes; as a child of the player's
//! content it would re-render the whole player each time. As a live layer of
//! `qust_ui::WindowRoot` (a sibling of the content), it re-renders alone. The playing row's
//! equalizer, which animates at the frame rate, is a second live layer ([`EqualizerLayer`]) that
//! reads its place from the drawer and paints clipped to the list, so its frames re-render
//! neither the drawer nor the player. The root paints the window background over the full
//! height, so the drawer area is never unpainted while the window grows or shrinks.
//!
//! **Right-to-left titles**: what a row draws of its title and detail is `qust_text::shown` of
//! them, computed when the cartridge or the entry changes (`Drawer::row_element` reads it from a
//! cache); tooltips and accessibility labels keep the logical text.
//!
//! # Modules
//!
//! - [`list`]: the arithmetic layout, the selection, keys, drop index, reveal, texts (no GPUI).
//! - [`menu`]: the right-click menu and its actions.
//! - [`host`]: the two seams.
//! - [`drawer`]: the view, [`mount`]; the playing row's [`EqualizerLayer`] is
//!   `qust_fluent::EqualizerLayer`.
//! - `harness` (feature `capture`): the reference fixtures' actions, for captures and tests.

pub mod drawer;
#[cfg(feature = "capture")]
pub mod harness;
pub mod host;
pub mod list;
pub mod menu;

pub use drawer::{mount, Drawer, Stats};
pub use qust_fluent::equalizer::{EqualizerLayer, EqualizerPlacement};
pub use host::{DrawerHost, Panel, Pending, PlaylistCommands};
pub use list::Item;
