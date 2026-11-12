//! The player window's part above the drawer: a thin view laying out the five islands (each its
//! own cached view) the way `.app` laid them out, and owning what crosses them (the dialog, the
//! right-click menus).

use crate::cartridges::CartridgesView;
use crate::commands::Commands;
use crate::deck::DeckView;
use crate::dialog::{DialogTarget, DialogView};
use crate::display::DisplayView;
use crate::host::{Panel, PlayerHost};
use crate::live;
use crate::menus::{self, MenuAction};
use crate::sliders::SlidersView;
use crate::transport::TransportView;
use gpui::{
    div, prelude::*, px, AnyWindowHandle, App, Context, Entity, Global, IntoElement, MouseButton, MouseUpEvent, Render, StyleRefinement, WeakEntity, Window,
};
use qust_fluent::Forced;
use qust_ui::{Models, WindowRoot};
use std::cell::{Cell, OnceCell};
use std::collections::HashMap;
use std::rc::Rc;

/// The player window's height above the drawer (`--player-height`).
pub const HEIGHT: f32 = 267.0;
/// The drawer's height below it (`--drawer-height`).
pub const DRAWER_HEIGHT: f32 = 320.0;
/// The key of the slot the view leaves for the drawer (860 × 320 at y 267): the app puts the
/// drawer there as a live layer of the window root.
pub const DRAWER_SLOT: &str = "drawer";
pub const WIDTH: f32 = 860.0;

/// What every part of the view shares.
pub(crate) struct Shared {
    pub models: Models,
    pub commands: Rc<dyn Commands>,
    pub host: Rc<dyn PlayerHost>,
    pub root: WeakEntity<WindowRoot>,
    pub dialog: OnceCell<Entity<DialogView>>,
    pub cartridges: OnceCell<Entity<CartridgesView>>,
    pub deck: OnceCell<Entity<DeckView>>,
    /// The drawer as the Playlist button shows it.
    pub drawer_open: Cell<bool>,
}

impl Shared {
    pub fn dialog(&self) -> Option<&Entity<DialogView>> {
        self.dialog.get()
    }

    pub fn dialog_open(&self, cx: &App) -> bool {
        self.dialog().is_some_and(|dialog| dialog.read(cx).is_open())
    }

    pub fn close_dialog(&self, cx: &mut App) {
        if let Some(dialog) = self.dialog() {
            dialog.update(cx, |dialog, cx| dialog.close(cx));
        }
    }

    pub fn open_dialog(&self, target: DialogTarget, window: &mut Window, cx: &mut App) {
        if let Some(dialog) = self.dialog().cloned() {
            dialog.update(cx, |dialog, cx| dialog.open(target, window, cx));
        }
    }

    /// `openSettings()` / `openInfo()`: the dialog closes first.
    pub fn open_panel(&self, panel: Panel, window: &mut Window, cx: &mut App) {
        self.close_dialog(cx);
        self.host.open_panel(panel, window, cx);
    }

    /// Runs a chosen menu item.
    pub fn run(&self, action: MenuAction, window: &mut Window, cx: &mut App) {
        match action {
            MenuAction::Play => self.commands.play(),
            MenuAction::Pause => self.commands.pause(),
            MenuAction::Stop => self.commands.stop(),
            MenuAction::Info => self.open_panel(Panel::Info, window, cx),
            MenuAction::Settings => self.open_panel(Panel::Settings, window, cx),
            MenuAction::PlayCartridge(slot) => self.commands.select_slot(slot),
            MenuAction::EditCartridge(slot) | MenuAction::LoadCartridge(slot) => self.open_dialog(DialogTarget::Cartridge(slot), window, cx),
            MenuAction::EjectCartridge(slot) => self.commands.eject(slot),
        }
    }

    /// Shows `menu` through the host and runs the choice in `window`.
    pub fn show_menu(self: &Rc<Self>, menu: qust_shell::menu::Menu, window: &mut Window, cx: &mut App) {
        let shared = self.clone();
        let handle: AnyWindowHandle = window.window_handle();
        self.host.show_menu(
            menu,
            window,
            cx,
            Box::new(move |chosen, cx: &mut App| {
                let Some(action) = chosen.and_then(MenuAction::from_id) else { return };
                let _ = handle.update(cx, |_, window, cx| shared.run(action, window, cx));
            }),
        );
    }
}

/// Pseudo-states forced on elements, by the fixtures' selectors (`#playButton`,
/// `#cartridges > .cartridge:nth-child(3) > .cartridge-body`): set by captures, absent in the
/// app.
#[derive(Default, Clone)]
pub struct ForcedStates {
    pub by_selector: HashMap<String, Forced>,
}

impl Global for ForcedStates {}

/// The state forced on `selector` (none in the app).
pub fn forced(cx: &App, selector: &str) -> Forced {
    cx.try_global::<ForcedStates>().and_then(|states| states.by_selector.get(selector).copied()).unwrap_or(Forced::NONE)
}

/// The player window above the drawer.
pub struct PlayerView {
    pub(crate) shared: Rc<Shared>,
    display: Entity<DisplayView>,
    sliders: Entity<SlidersView>,
    transport: Entity<TransportView>,
    cartridges: Entity<CartridgesView>,
    deck: Entity<DeckView>,
    live: Option<live::LiveViews>,
    /// Renders of this view (measurements and tests).
    pub renders: u64,
}

impl PlayerView {
    /// The view, its live layers (analyzer, clock, title, seek row, cartridge badges) put on
    /// `root`, and its dialog ready to open as an overlay of `root`.
    pub fn new(models: &Models, commands: Rc<dyn Commands>, host: Rc<dyn PlayerHost>, root: WeakEntity<WindowRoot>, window: &mut Window, cx: &mut Context<Self>) -> PlayerView {
        let shared = Rc::new(Shared {
            models: models.clone(),
            commands,
            host,
            root: root.clone(),
            dialog: OnceCell::new(),
            cartridges: OnceCell::new(),
            deck: OnceCell::new(),
            drawer_open: Cell::new(models.settings.read(cx).playlist_open),
        });
        let dialog = cx.new(|cx| DialogView::new(shared.clone(), window, cx));
        let _ = shared.dialog.set(dialog);
        let display = cx.new(|cx| DisplayView::new(shared.clone(), cx));
        let sliders = cx.new(|cx| SlidersView::new(shared.clone(), cx));
        let transport = cx.new(|cx| TransportView::new(shared.clone(), cx));
        let cartridges = cx.new(|cx| CartridgesView::new(shared.clone(), cx));
        let _ = shared.cartridges.set(cartridges.clone());
        let deck = cx.new(|cx| DeckView::new(shared.clone(), cx));
        let _ = shared.deck.set(deck.clone());
        let live = live::install(&shared, window, cx);
        PlayerView { shared, display, sliders, transport, cartridges, deck, live, renders: 0 }
    }

    /// Opens the Load / Edit cartridge dialog for `slot` (the drawer's "Edit cartridge…").
    pub fn open_cartridge_dialog(&mut self, slot: usize, window: &mut Window, cx: &mut Context<Self>) {
        self.shared.open_dialog(DialogTarget::Cartridge(slot), window, cx);
    }

    /// Opens the Edit track dialog for playlist entry `id` (the drawer's "Edit…"). Nothing
    /// happens for an id the playlist doesn't have.
    pub fn open_entry_dialog(&mut self, id: u64, window: &mut Window, cx: &mut Context<Self>) {
        self.shared.open_dialog(DialogTarget::Entry(id), window, cx);
    }

    /// Opens the Add URL dialog (the drawer's "Add URL…").
    pub fn open_url_dialog(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.shared.open_dialog(DialogTarget::Url, window, cx);
    }

    /// Closes the dialog (instant, like the page).
    pub fn close_dialog(&mut self, cx: &mut Context<Self>) {
        self.shared.close_dialog(cx);
    }

    /// Whether the dialog is open: while it is, the drawer shows no menu and ignores file drops.
    pub fn dialog_open(&self, cx: &App) -> bool {
        self.shared.dialog_open(cx)
    }

    /// The dialog's view (tests, and the integrator's Esc / focus checks).
    pub fn dialog(&self) -> Option<Entity<DialogView>> {
        self.shared.dialog().cloned()
    }

    pub fn cartridges(&self) -> Entity<CartridgesView> {
        self.cartridges.clone()
    }

    pub fn sliders(&self) -> Entity<SlidersView> {
        self.sliders.clone()
    }

    pub fn display(&self) -> Entity<DisplayView> {
        self.display.clone()
    }

    pub fn transport(&self) -> Entity<TransportView> {
        self.transport.clone()
    }

    pub fn deck(&self) -> Entity<DeckView> {
        self.deck.clone()
    }

    /// The live layers' views (None if the window root was gone when the view was made).
    pub fn live(&self) -> Option<&live::LiveViews> {
        self.live.as_ref()
    }

    /// The drawer opened or closed by itself (a file drag over the window, entries added): the
    /// Playlist button follows. Its own clicks need no call.
    pub fn set_drawer_open(&mut self, open: bool, cx: &mut Context<Self>) {
        if self.shared.drawer_open.replace(open) != open {
            self.deck.update(cx, |_, cx| cx.notify());
        }
    }

    pub fn drawer_open(&self) -> bool {
        self.shared.drawer_open.get()
    }

    /// The player-area menu (Play or Pause, Stop, Stream information, Settings) as the window's
    /// state makes it now; run the chosen id with [`PlayerView::run_menu_item`].
    pub fn player_menu(&self, cx: &App) -> qust_shell::menu::Menu {
        menus::player_menu(self.shared.models.deck.read(cx), &self.shared.models.shortcuts.read(cx).shortcuts)
    }

    /// Runs a menu item of this window's menus (ids in [`menus::ids`]). False for an id it
    /// doesn't know.
    pub fn run_menu_item(&mut self, id: u32, window: &mut Window, cx: &mut Context<Self>) -> bool {
        match MenuAction::from_id(id) {
            Some(action) => {
                self.shared.run(action, window, cx);
                true
            }
            None => false,
        }
    }

    /// Ends a cartridge drag without moving anything (Esc).
    pub fn cancel_drag(&mut self, cx: &mut Context<Self>) {
        self.cartridges.update(cx, |cartridges, cx| cartridges.end_drag(false, cx));
    }
}

impl Render for PlayerView {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        self.renders += 1;
        let shared = self.shared.clone();
        let shared_keys = self.shared.clone();
        let fixed = |w: f32, h: f32| StyleRefinement::default().w(px(w)).h(px(h));
        let player = div()
            .id("player-window")
            .relative()
            .flex()
            .flex_row()
            .gap(px(28.0))
            .pt(px(6.0))
            .px(px(22.0))
            .pb(px(22.0))
            .w(px(WIDTH))
            .h(px(HEIGHT))
            // Chromium kerns; GPUI only when asked (crate::text).
            .font_features(crate::text::kerning())
            // Right-click anywhere outside the cartridges (the cartridges show their own menus):
            // the transport menu. Chromium sends `contextmenu` on the button's release.
            .on_mouse_up(MouseButton::Right, move |_: &MouseUpEvent, window, cx| {
                if shared.dialog_open(cx) {
                    return;
                }
                let menu = menus::player_menu(shared.models.deck.read(cx), &shared.models.shortcuts.read(cx).shortcuts);
                shared.show_menu(menu, window, cx);
            })
            // Esc ends a cartridge drag (the page listened on the document).
            .on_key_down(move |event, _, cx| {
                if event.keystroke.key == "escape" {
                    if let Some(cartridges) = shared_keys.cartridges.get() {
                        let dragging = cartridges.read(cx).dragging();
                        if dragging {
                            cartridges.update(cx, |cartridges, cx| cartridges.end_drag(false, cx));
                        }
                    }
                }
            })
            .child(
                // .player: the three islands, spaced to the cartridge grid's height.
                div()
                    .flex()
                    .flex_col()
                    .justify_between()
                    .w(px(330.0))
                    // The columns are 247 tall: the grid's row is 239 (267 − padding), and the
                    // islands overflow it, as in the page.
                    .h(px(247.0))
                    .flex_none()
                    .child(self.display.clone().cached(fixed(330.0, 91.0)))
                    .child(self.sliders.clone().cached(fixed(330.0, 80.0)))
                    .child(self.transport.clone().cached(fixed(330.0, 52.0))),
            )
            .child(
                // .deck: the cartridges over the deck bar.
                div()
                    .flex()
                    .flex_col()
                    .gap(px(12.0))
                    .w(px(458.0))
                    .h(px(247.0))
                    .flex_none()
                    .child(self.cartridges.clone().cached(fixed(458.0, 183.0)))
                    .child(self.deck.clone().cached(fixed(458.0, 52.0))),
            );
        // The whole window: this part, then the drawer's place, a slot the app fills with the
        // drawer's live layer (qust-ui-playlist), cut off by the window while it is closed.
        div()
            .w(px(WIDTH))
            .h(px(HEIGHT + DRAWER_HEIGHT))
            .flex()
            .flex_col()
            .child(player)
            .child(div().w(px(WIDTH)).h(px(DRAWER_HEIGHT)).flex_none().child(qust_ui::portal_slot(self.shared.root.clone(), DRAWER_SLOT)))
    }
}
