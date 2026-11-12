//! Qust's player window above the playlist drawer (y < 267): the display island, the sliders,
//! the transport, the six cartridges, the deck bar, the dialog and the right-click menus, as
//! `reference/` describes them (BEHAVIOUR.md sections 2.1–2.6, 2.8–2.10).
//!
//! # Putting it in a window
//!
//! ```ignore
//! let root = cx.new(|_| WindowRoot::new(placeholder));
//! let player = cx.new(|cx| PlayerView::new(&models, commands, host, root.downgrade(), window, cx));
//! // The window's content: the player (860 × 267) over the drawer (860 × 320).
//! root.update(cx, |root, cx| root.set_content(app_content_with(player.clone(), drawer), cx));
//! ```
//!
//! `PlayerView::new` puts its live layers on the root (keys `analyzer`, `clock`, `title`, `seek`,
//! `badge-0` … `badge-5`, see [`live`]) and shows its overlays there while needed (`dialog`,
//! `cartridge-ghost`). The view is the window's content: 860 × 587, its part at the top
//! (y < 267) and below it an empty 860 × 320 [`portal_slot`](qust_ui::portal_slot) keyed
//! [`DRAWER_SLOT`] (`"drawer"`), where the app mounts the drawer as a live layer of the root
//! (`root.set_live("drawer", drawer_view)`). Add the drawer's layer before any overlay is shown
//! (the root draws live layers below overlays, so the dialog and the drag ghost cover it). The
//! view assumes nothing about the drawer beyond that slot.
//!
//! # What the view needs from outside
//!
//! - [`Commands`]: the player. `qust_ui::Player` implements it (`Rc::new(bridge.player.clone())`).
//! - [`PlayerHost`]: the app. `open_panel(Panel)`, `set_drawer(open)` (the Playlist button;
//!   the view flips its button at once), `add_files()` (picker, add, select in the drawer),
//!   `added(ids)` (Add URL succeeded: the drawer selects them, opening itself), and
//!   `show_menu(menu, window, cx, done)` (a `qust_shell::menu::Menu` at the cursor, through
//!   `qust_shell::menu::popup`, then `done(choice, cx)` on the UI thread).
//! - The models (`qust_ui::Models`): `now`, `clock`, `deck`, `levels`, `seek`, `analyzer`,
//!   `shortcuts` (tooltips and menu accelerators follow them), `update` (the dot on Settings),
//!   `playlist` (the Edit track dialog), `settings` (`playlist_open` at start).
//!
//! # What the view offers
//!
//! - [`PlayerView::open_cartridge_dialog`]`(slot)`, [`PlayerView::open_entry_dialog`]`(id)`,
//!   [`PlayerView::open_url_dialog`]: the drawer's "Edit cartridge…", "Edit…", "Add URL…".
//! - [`PlayerView::dialog_open`]: while true the drawer shows no menu and ignores file drops.
//!   [`PlayerView::close_dialog`].
//! - [`PlayerView::set_drawer_open`]`(open)`: the drawer opened or closed by itself.
//! - [`PlayerView::player_menu`] and [`PlayerView::run_menu_item`]`(id)`: the player-area menu,
//!   and running any item id of [`menus::ids`] (the drawer's menus end with
//!   [`menus::with_panels`]: Stream information / Settings).
//! - [`PlayerView::cancel_drag`]: Esc during a cartridge drag (the view handles Esc itself when
//!   the key reaches it).
//!
//! # Testing
//!
//! Feature `capture`: [`testing`] (a recording stand-in for both traits, the fixture
//! interpreter), the example `player_capture` (every player fixture, compared with
//! `compare.mjs`), the tests `behaviour` and `end_to_end`.

pub mod cartridges;
pub mod commands;
pub mod deck;
pub mod dialog;
pub mod display;
pub mod host;
pub mod live;
pub mod menus;
pub mod sliders;
pub mod text;
pub mod transport;
pub mod view;

pub use commands::{Answer, Commands};
pub use dialog::DialogTarget;
pub use host::{MenuDone, Panel, PlayerHost};
pub use view::{forced, ForcedStates, PlayerView, DRAWER_HEIGHT, DRAWER_SLOT, HEIGHT, WIDTH};

#[cfg(feature = "capture")]
pub mod testing;
