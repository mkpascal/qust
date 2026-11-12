//! The player window's right-click menus (BEHAVIOUR.md 2.9), built per click like the page's
//! `contextmenu` handler built them, as `qust_shell::menu::Menu`s. The view hands them to
//! `PlayerHost::show_menu` and runs the chosen item ([`crate::PlayerView`]).
//!
//! Accelerator text is the global shortcut's, only when it is registered, in the form the old
//! menus displayed (`qust_shell::accelerator::menu_text`: "Ctrl+Alt+Ins"); when one of them can't
//! be shown that way the whole menu goes without accelerators, as the page rebuilt it.

use qust_shell::menu::{Item, Menu};
use qust_ui::models::Deck;
use qust_ui::{Binding, Shortcuts};

/// Item ids. The drawer's menus can reuse [`ids::INFO`] and [`ids::SETTINGS`] and hand them to
/// [`crate::PlayerView::run_menu_item`].
pub mod ids {
    pub const PLAY: u32 = 1;
    pub const PAUSE: u32 = 2;
    pub const STOP: u32 = 3;
    pub const INFO: u32 = 4;
    pub const SETTINGS: u32 = 5;
    /// Cartridge items: `CARTRIDGE + slot * 4 + n` (n: 0 play, 1 edit, 2 eject, 3 load).
    pub const CARTRIDGE: u32 = 100;
}

/// What a menu item does.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MenuAction {
    Play,
    Pause,
    Stop,
    Info,
    Settings,
    PlayCartridge(usize),
    EditCartridge(usize),
    EjectCartridge(usize),
    LoadCartridge(usize),
}

impl MenuAction {
    pub fn from_id(id: u32) -> Option<MenuAction> {
        Some(match id {
            ids::PLAY => MenuAction::Play,
            ids::PAUSE => MenuAction::Pause,
            ids::STOP => MenuAction::Stop,
            ids::INFO => MenuAction::Info,
            ids::SETTINGS => MenuAction::Settings,
            id if (ids::CARTRIDGE..ids::CARTRIDGE + 24).contains(&id) => {
                let slot = ((id - ids::CARTRIDGE) / 4) as usize;
                match (id - ids::CARTRIDGE) % 4 {
                    0 => MenuAction::PlayCartridge(slot),
                    1 => MenuAction::EditCartridge(slot),
                    2 => MenuAction::EjectCartridge(slot),
                    _ => MenuAction::LoadCartridge(slot),
                }
            }
            _ => return None,
        })
    }
}

fn cartridge_id(slot: usize, n: u32) -> u32 {
    ids::CARTRIDGE + slot as u32 * 4 + n
}

/// `shortcutLabel(action)`'s accelerator, as the menu shows it: Some(Some(text)) to show,
/// Some(None) for none, None when it can't be shown (the menu then drops every accelerator).
fn accelerator(binding: &Binding) -> Option<Option<String>> {
    if !(binding.assigned() && binding.registered) {
        return Some(None);
    }
    qust_shell::accelerator::menu_text(&binding.accelerator).map(Some)
}

/// The active cartridge or entry exists (`activeItem() !== null`).
pub fn has_active(deck: &Deck) -> bool {
    match deck.active_slot {
        Some(slot) if deck.cartridges.get(slot).is_some_and(Option::is_some) => true,
        _ => deck.active_id.is_some(),
    }
}

/// The menu for the player area (display, sliders, transport, deck bar, outside cartridges):
/// Pause / Stop while playing, connecting or buffering; Play / Stop otherwise.
pub fn player_menu(deck: &Deck, shortcuts: &Shortcuts) -> Menu {
    let play = accelerator(&shortcuts.play);
    let stop = accelerator(&shortcuts.stop);
    let mut menu = Menu::new();
    if deck.state.is_running() {
        menu = menu.item(ids::PAUSE, "Pause");
    } else {
        menu = menu.action(ids::PLAY, "Play", has_active(deck), false, play.clone().flatten().as_deref());
    }
    menu = menu.action(ids::STOP, "Stop", deck.state != qust_ui::Playback::Stopped, false, stop.clone().flatten().as_deref());
    let menu = with_panels(menu);
    // Only the accelerators the items carry matter: Pause has none.
    let used = if deck.state.is_running() { [stop].to_vec() } else { [play, stop].to_vec() };
    if used.iter().any(Option::is_none) {
        menu.without_accelerators()
    } else {
        menu
    }
}

/// A loaded cartridge: Play / Edit… / Eject; an empty one: Load cartridge….
pub fn cartridge_menu(slot: usize, loaded: bool) -> Menu {
    let menu = if loaded {
        Menu::new().item(cartridge_id(slot, 0), "Play").item(cartridge_id(slot, 1), "Edit…").item(cartridge_id(slot, 2), "Eject")
    } else {
        Menu::new().item(cartridge_id(slot, 3), "Load cartridge…")
    };
    with_panels(menu)
}

/// Every menu of the window ends with a separator (if anything precedes) and Stream information,
/// Settings.
pub fn with_panels(menu: Menu) -> Menu {
    let menu = if menu.items.is_empty() { menu } else { menu.separator() };
    menu.item(ids::INFO, "Stream information").item(ids::SETTINGS, "Settings")
}

/// The menu as `(text, enabled, accelerator)` rows (None for a separator), for tests.
pub fn rows(menu: &Menu) -> Vec<Option<(String, bool, Option<String>)>> {
    menu.items
        .iter()
        .map(|item| match item {
            Item::Action { text, enabled, accelerator, .. } => Some((text.clone(), *enabled, accelerator.clone())),
            Item::Separator => None,
        })
        .collect()
}
