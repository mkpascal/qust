//! The drawer's right-click menu (BEHAVIOUR.md 2.9), built per click as script.js built it:
//!
//! | where | items |
//! |---|---|
//! | a cartridge row | Play · Edit cartridge… · sep · Add file… · Add URL… · Clear playlist |
//! | a track | Play · Edit… · Remove (or "Remove N tracks") · sep · Add file… · Add URL… · Clear playlist |
//! | elsewhere in the drawer | Add file… · Add URL… · Clear playlist |
//!
//! then a separator, Stream information, Settings. "Clear playlist" is disabled with no entries.
//! No item has an accelerator.

use crate::list::Item;
use qust_shell::menu::Menu;

/// What a chosen item does. Remove carries the tracks selected when the menu was built; Clear
/// playlist takes the entries as they are when chosen (the page's closures did the same).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Action {
    Play(Item),
    EditCartridge(usize),
    EditEntry(u64),
    Remove(Vec<u64>),
    AddFile,
    AddUrl,
    ClearPlaylist,
    StreamInformation,
    Settings,
}

/// The menu for a right-click on `target` (None: not on a row) with `tracks` selected and
/// `entries` in the playlist, and the action of each item id.
pub fn build(target: Option<Item>, tracks: &[u64], entries: usize) -> (Menu, Vec<(u32, Action)>) {
    let mut menu = Menu::new();
    let mut actions = Vec::new();
    let mut add = |menu: Menu, text: &str, enabled: bool, action: Action| {
        let id = actions.len() as u32 + 1;
        actions.push((id, action));
        menu.action(id, text, enabled, false, None)
    };
    match target {
        Some(Item::Cartridge(slot)) => {
            menu = add(menu, "Play", true, Action::Play(Item::Cartridge(slot)));
            menu = add(menu, "Edit cartridge\u{2026}", true, Action::EditCartridge(slot));
            menu = menu.separator();
        }
        Some(Item::Entry(id)) => {
            menu = add(menu, "Play", true, Action::Play(Item::Entry(id)));
            menu = add(menu, "Edit\u{2026}", true, Action::EditEntry(id));
            let remove = if tracks.len() > 1 { format!("Remove {} tracks", tracks.len()) } else { "Remove".to_string() };
            menu = add(menu, &remove, true, Action::Remove(tracks.to_vec()));
            menu = menu.separator();
        }
        None => {}
    }
    menu = add(menu, "Add file\u{2026}", true, Action::AddFile);
    menu = add(menu, "Add URL\u{2026}", true, Action::AddUrl);
    menu = add(menu, "Clear playlist", entries > 0, Action::ClearPlaylist);
    menu = menu.separator();
    menu = add(menu, "Stream information", true, Action::StreamInformation);
    menu = add(menu, "Settings", true, Action::Settings);
    (menu, actions)
}

#[cfg(test)]
mod tests {
    use super::*;
    use qust_shell::menu::Item as MenuItem;

    // The items as the layout files record them: (text, enabled) or None for a separator.
    fn shape(menu: &Menu) -> Vec<Option<(String, bool)>> {
        menu.items
            .iter()
            .map(|item| match item {
                MenuItem::Action { text, enabled, accelerator, .. } => {
                    assert!(accelerator.is_none());
                    Some((text.clone(), *enabled))
                }
                MenuItem::Separator => None,
            })
            .collect()
    }

    fn recorded(name: &str) -> Vec<Option<(String, bool)>> {
        let path = qust_ui::fixture::reference_dir().join("layout/player").join(format!("{name}.json"));
        let layout: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(path).expect("layout")).expect("json");
        layout["menus"][0]
            .as_array()
            .expect("a menu")
            .iter()
            .map(|item| item.get("text").map(|text| (text.as_str().unwrap_or("").to_string(), item["enabled"].as_bool().unwrap_or(true))))
            .collect()
    }

    #[test]
    fn menus_match_the_recorded_ones() {
        assert_eq!(shape(&build(None, &[], 0).0), recorded("menu-drawer-empty"));
        assert_eq!(shape(&build(Some(Item::Cartridge(1)), &[], 6).0), recorded("menu-row-cartridge"));
        assert_eq!(shape(&build(Some(Item::Entry(3)), &[3], 6).0), recorded("menu-row-track"));
        assert_eq!(shape(&build(Some(Item::Entry(2)), &[1, 2, 3], 6).0), recorded("menu-row-tracks"));
    }

    #[test]
    fn ids_map_to_actions() {
        let (menu, actions) = build(Some(Item::Entry(2)), &[1, 2], 6);
        let ids: Vec<u32> = menu.items.iter().filter_map(|item| if let MenuItem::Action { id, .. } = item { Some(*id) } else { None }).collect();
        assert_eq!(ids, actions.iter().map(|(id, _)| *id).collect::<Vec<_>>());
        assert_eq!(actions[2].1, Action::Remove(vec![1, 2]));
        assert_eq!(actions.last().map(|(_, a)| a.clone()), Some(Action::Settings));
    }
}
