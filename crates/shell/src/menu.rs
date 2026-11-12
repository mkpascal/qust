// Native popup menus: the player's right-click menus, the Settings choices (checked items), and the
// edit menu of text fields. Win32 menus as muda showed them (TrackPopupMenu, left-aligned at the
// point, the owner brought to the front first), in the app's theme.
//
// Two rules keep the windows drawing while a menu is open:
// - no TPM_NONOTIFY: the owner then gets WM_ENTERMENULOOP, on which GPUI starts the timer that
//   keeps it running and painting inside the menu's modal loop;
// - the menu is never shown from inside a GPUI callback (the modal loop would re-enter GPUI while
//   it is borrowed): `popup` schedules it with a zero timer, which the message loop runs between
//   two events (what menu.rs did for Tauri), and hands the choice to `done` afterwards.

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Item {
    Action { id: u32, text: String, enabled: bool, checked: bool, accelerator: Option<String> },
    Separator,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Menu {
    pub items: Vec<Item>,
}

impl Menu {
    pub fn new() -> Menu {
        Menu::default()
    }

    pub fn item(mut self, id: u32, text: &str) -> Menu {
        self.items.push(Item::Action { id, text: text.to_string(), enabled: true, checked: false, accelerator: None });
        self
    }

    // `accelerator` is the text after the tab ("Ctrl+Alt+Ins"); see `Binding::menu_text`.
    pub fn action(mut self, id: u32, text: &str, enabled: bool, checked: bool, accelerator: Option<&str>) -> Menu {
        self.items.push(Item::Action { id, text: text.to_string(), enabled, checked, accelerator: accelerator.map(str::to_string) });
        self
    }

    pub fn separator(mut self) -> Menu {
        self.items.push(Item::Separator);
        self
    }

    // Without any accelerator text (what the page fell back to when one didn't parse).
    pub fn without_accelerators(mut self) -> Menu {
        for item in &mut self.items {
            if let Item::Action { accelerator, .. } = item {
                *accelerator = None;
            }
        }
        self
    }

    // The label Win32 gets: the text, then a tab and the accelerator (muda's format).
    pub fn label(text: &str, accelerator: Option<&str>) -> String {
        match accelerator {
            Some(accelerator) => format!("{text}\t{accelerator}"),
            None => text.to_string(),
        }
    }
}

// Where the menu opens.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Position {
    // At the mouse cursor (right-click menus).
    Cursor,
    // At a point of the owner's client area, logical pixels (Settings combos: the combo's
    // bottom-left + 2).
    Client { x: f64, y: f64 },
}

// The edit menu of text fields: muda's predefined items, always enabled.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EditCommand {
    Cut,
    Copy,
    Paste,
    SelectAll,
}

// The seam with the text input (qust-fluent's text box): what the edit menu acts on. The menu
// replaced muda's simulated Ctrl+X / C / V / A with direct calls.
pub trait EditTarget {
    fn cut(&mut self);
    fn copy(&mut self);
    fn paste(&mut self);
    fn select_all(&mut self);
}

impl EditCommand {
    pub const IDS: [(u32, EditCommand); 4] = [(0xE001, EditCommand::Cut), (0xE002, EditCommand::Copy), (0xE003, EditCommand::Paste), (0xE004, EditCommand::SelectAll)];

    pub fn from_id(id: u32) -> Option<EditCommand> {
        EditCommand::IDS.iter().find(|(other, _)| *other == id).map(|(_, command)| *command)
    }

    pub fn apply(self, target: &mut dyn EditTarget) {
        match self {
            EditCommand::Cut => target.cut(),
            EditCommand::Copy => target.copy(),
            EditCommand::Paste => target.paste(),
            EditCommand::SelectAll => target.select_all(),
        }
    }
}

// Cut / Copy / Paste / separator / Select All, with muda's mnemonics and accelerator text.
pub fn edit_menu() -> Menu {
    Menu::new()
        .action(0xE001, "Cu&t", true, false, Some("Ctrl+X"))
        .action(0xE002, "&Copy", true, false, Some("Ctrl+C"))
        .action(0xE003, "&Paste", true, false, Some("Ctrl+V"))
        .separator()
        .action(0xE004, "Select &All", true, false, Some("Ctrl+A"))
}

// The read-only values' menu (Stream information): Copy alone.
pub fn copy_menu() -> Menu {
    Menu::new().action(0xE002, "&Copy", true, false, Some("Ctrl+C"))
}

#[cfg(windows)]
mod imp {
    use super::*;
    use crate::theme::Theme;
    use crate::win::{self, PreferredAppMode};
    use std::cell::RefCell;
    use windows::core::PCWSTR;
    use windows::Win32::Foundation::{HWND, LPARAM, POINT, WPARAM};
    use windows::Win32::Graphics::Gdi::ClientToScreen;
    use windows::Win32::UI::WindowsAndMessaging::*;

    pub(crate) fn apply_theme(theme: Theme) {
        win::set_preferred_app_mode(match theme {
            Theme::System => PreferredAppMode::AllowDark,
            Theme::Light => PreferredAppMode::ForceLight,
            Theme::Dark => PreferredAppMode::ForceDark,
        });
    }

    pub fn dark_menus_supported() -> bool {
        win::dark_menus_supported()
    }

    type Done = Box<dyn FnOnce(Option<u32>)>;

    struct Pending {
        owner: isize,
        menu: Menu,
        position: Position,
        flags: TRACK_POPUP_MENU_FLAGS,
        foreground: bool,
        done: Done,
    }

    thread_local! {
        static PENDING: RefCell<Vec<Pending>> = const { RefCell::new(Vec::new()) };
    }

    // Shows `menu` for `owner` (a window of this thread) once the current event is handled, then
    // calls `done` with the chosen item's id (None when dismissed). Call it on the owner's thread
    // (GPUI's main thread), from anywhere, including a GPUI event handler. `done` runs outside
    // GPUI too: post its result to the UI.
    pub fn popup(owner: isize, menu: Menu, position: Position, done: impl FnOnce(Option<u32>) + 'static) {
        schedule(Pending { owner, menu, position, flags: TPM_LEFTALIGN | TPM_RETURNCMD, foreground: true, done: Box::new(done) });
    }

    // The edit menu, calling `on_command` with the choice.
    pub fn popup_edit(owner: isize, position: Position, on_command: impl FnOnce(EditCommand) + 'static) {
        popup(owner, edit_menu(), position, move |chosen| {
            if let Some(command) = chosen.and_then(EditCommand::from_id) {
                on_command(command);
            }
        });
    }

    // Tests: extra flags (TPM_NONOTIFY to show the freeze it causes) and no foreground change.
    #[doc(hidden)]
    pub fn popup_for_test(owner: isize, menu: Menu, position: Position, no_notify: bool, done: impl FnOnce(Option<u32>) + 'static) {
        let mut flags = TPM_LEFTALIGN | TPM_RETURNCMD;
        if no_notify {
            flags |= TPM_NONOTIFY;
        }
        schedule(Pending { owner, menu, position, flags, foreground: false, done: Box::new(done) });
    }

    // Tests: shows the menu right now, from wherever this is called (inside a GPUI callback, to
    // see what happens when `popup`'s deferral is skipped).
    #[doc(hidden)]
    pub fn show_now_for_test(owner: isize, menu: Menu, position: Position, no_notify: bool) -> Option<u32> {
        let mut flags = TPM_LEFTALIGN | TPM_RETURNCMD;
        if no_notify {
            flags |= TPM_NONOTIFY;
        }
        show(&Pending { owner, menu, position, flags, foreground: false, done: Box::new(|_| {}) })
    }

    fn schedule(pending: Pending) {
        PENDING.with(|queue| queue.borrow_mut().push(pending));
        unsafe {
            SetTimer(None, 0, 0, Some(run_pending));
        }
    }

    // Run by the message loop itself, between two events.
    unsafe extern "system" fn run_pending(_: HWND, _: u32, timer: usize, _: u32) {
        let _ = KillTimer(None, timer);
        loop {
            let next = PENDING.with(|queue| {
                let mut queue = queue.borrow_mut();
                (!queue.is_empty()).then(|| queue.remove(0))
            });
            let Some(pending) = next else { break };
            let chosen = show(&pending);
            (pending.done)(chosen);
        }
    }

    fn build(menu: &Menu) -> Option<HMENU> {
        unsafe {
            let handle = CreatePopupMenu().ok()?;
            for item in &menu.items {
                match item {
                    Item::Separator => {
                        let _ = AppendMenuW(handle, MF_SEPARATOR, 0, PCWSTR::null());
                    }
                    Item::Action { id, text, enabled, checked, accelerator } => {
                        let mut flags = MF_STRING;
                        if !enabled {
                            flags |= MF_GRAYED;
                        }
                        if *checked {
                            flags |= MF_CHECKED;
                        }
                        let label = win::wide(&Menu::label(text, accelerator.as_deref()));
                        let _ = AppendMenuW(handle, flags, *id as usize, PCWSTR(label.as_ptr()));
                    }
                }
            }
            Some(handle)
        }
    }

    // Item states as Win32 has them (tests): (id, grayed, checked, label).
    #[doc(hidden)]
    pub fn inspect(menu: &Menu) -> Vec<(u32, bool, bool, String)> {
        let Some(handle) = build(menu) else { return Vec::new() };
        let mut out = Vec::new();
        unsafe {
            for index in 0..GetMenuItemCount(Some(handle)).max(0) {
                let id = GetMenuItemID(handle, index);
                let state = GetMenuState(handle, index as u32, MF_BYPOSITION);
                let mut buffer = [0u16; 256];
                let length = GetMenuStringW(handle, index as u32, Some(&mut buffer), MF_BYPOSITION);
                out.push((id, state & MF_GRAYED.0 != 0, state & MF_CHECKED.0 != 0, String::from_utf16_lossy(&buffer[..length.max(0) as usize])));
            }
            let _ = DestroyMenu(handle);
        }
        out
    }

    fn show(pending: &Pending) -> Option<u32> {
        let handle = build(&pending.menu)?;
        let owner = win::hwnd(pending.owner);
        unsafe {
            let point = match pending.position {
                Position::Cursor => {
                    let mut point = POINT::default();
                    let _ = GetCursorPos(&mut point);
                    point
                }
                Position::Client { x, y } => {
                    let scale = crate::window::scale(pending.owner);
                    let mut point = POINT { x: (x * scale) as i32, y: (y * scale) as i32 };
                    let _ = ClientToScreen(owner, &mut point);
                    point
                }
            };
            win::allow_dark_mode_for_window(owner, true);
            if pending.foreground {
                let _ = SetForegroundWindow(owner);
            }
            let chosen = TrackPopupMenu(handle, pending.flags, point.x, point.y, Some(0), owner, None);
            let _ = DestroyMenu(handle);
            let _ = PostMessageW(Some(owner), WM_NULL, WPARAM(0), LPARAM(0));
            (chosen.0 > 0).then_some(chosen.0 as u32)
        }
    }

    // --- Test support: menus that open off every screen and close themselves -------------------

    thread_local! {
        static HOOK: std::cell::Cell<isize> = const { std::cell::Cell::new(0) };
    }

    // While on, every menu this thread opens is moved off all monitors as it appears (so nothing
    // shows on the user's screen) and closed after `close_after` (EndMenu from a timer the menu's
    // own loop runs). Tests only.
    #[doc(hidden)]
    pub fn test_hide_menus(on: bool, close_after: std::time::Duration) {
        use windows::Win32::System::Threading::GetCurrentThreadId;
        use windows::Win32::UI::Shell::SetWindowSubclass;

        unsafe extern "system" fn offscreen(window: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM, _: usize, _: usize) -> windows::Win32::Foundation::LRESULT {
            if msg == WM_WINDOWPOSCHANGING {
                let position = &mut *(lparam.0 as *mut WINDOWPOS);
                position.x = -32000;
                position.y = -32000;
                position.flags &= !SWP_NOMOVE;
            }
            windows::Win32::UI::Shell::DefSubclassProc(window, msg, wparam, lparam)
        }

        unsafe extern "system" fn cbt(code: i32, wparam: WPARAM, lparam: LPARAM) -> windows::Win32::Foundation::LRESULT {
            if code == HCBT_CREATEWND as i32 {
                let window = HWND(wparam.0 as *mut _);
                let mut class = [0u16; 16];
                let length = GetClassNameW(window, &mut class);
                if String::from_utf16_lossy(&class[..length.max(0) as usize]) == "#32768" {
                    let create = &mut *(*(lparam.0 as *mut CBT_CREATEWNDW)).lpcs;
                    create.x = -32000;
                    create.y = -32000;
                    let _ = SetWindowSubclass(window, Some(offscreen), 0x4D45_4E55, 0);
                }
            }
            CallNextHookEx(None, code, wparam, lparam)
        }

        unsafe extern "system" fn end_menu(_: HWND, _: u32, timer: usize, _: u32) {
            let _ = KillTimer(None, timer);
            let _ = EndMenu();
        }

        unsafe {
            let current = HOOK.with(|hook| hook.get());
            if on && current == 0 {
                if let Ok(hook) = SetWindowsHookExW(WH_CBT, Some(cbt), None, GetCurrentThreadId()) {
                    HOOK.with(|cell| cell.set(hook.0 as isize));
                }
            } else if !on && current != 0 {
                let _ = UnhookWindowsHookEx(HHOOK(current as *mut _));
                HOOK.with(|cell| cell.set(0));
            }
            if on {
                SetTimer(None, 0, close_after.as_millis() as u32, Some(end_menu));
            }
        }
    }
}

#[cfg(windows)]
pub use imp::{dark_menus_supported, inspect, popup, popup_edit, popup_for_test, show_now_for_test, test_hide_menus};
#[cfg(windows)]
pub(crate) use imp::apply_theme;

// macOS: NSMenu (menu_mac.rs).
#[cfg(target_os = "macos")]
#[path = "menu_mac.rs"]
mod mac;
#[cfg(target_os = "macos")]
pub use mac::popup;

// Elsewhere a menu reports "dismissed".
#[cfg(not(any(windows, target_os = "macos")))]
pub fn popup(_owner: isize, _menu: Menu, _position: Position, done: impl FnOnce(Option<u32>) + 'static) {
    done(None);
}

#[cfg(not(windows))]
pub fn popup_edit(owner: isize, position: Position, on_command: impl FnOnce(EditCommand) + 'static) {
    popup(owner, edit_menu(), position, move |chosen| {
        if let Some(command) = chosen.and_then(EditCommand::from_id) {
            on_command(command);
        }
    });
}

#[cfg(not(windows))]
pub fn dark_menus_supported() -> bool {
    false
}

#[cfg(not(windows))]
pub(crate) fn apply_theme(_theme: crate::theme::Theme) {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn labels_and_edit_menu() {
        assert_eq!(Menu::label("Play", Some("Ctrl+Alt+Ins")), "Play\tCtrl+Alt+Ins");
        assert_eq!(Menu::label("Edit…", None), "Edit…");
        let edit = edit_menu();
        assert_eq!(edit.items.len(), 5);
        assert_eq!(edit.items[3], Item::Separator);
        assert_eq!(EditCommand::from_id(0xE004), Some(EditCommand::SelectAll));
        let stripped = Menu::new().action(1, "Play", true, false, Some("Ctrl+Alt+Ins")).without_accelerators();
        assert_eq!(stripped.items[0], Item::Action { id: 1, text: "Play".into(), enabled: true, checked: false, accelerator: None });
    }

    struct Field(Vec<&'static str>);

    impl EditTarget for Field {
        fn cut(&mut self) {
            self.0.push("cut");
        }
        fn copy(&mut self) {
            self.0.push("copy");
        }
        fn paste(&mut self) {
            self.0.push("paste");
        }
        fn select_all(&mut self) {
            self.0.push("select all");
        }
    }

    #[test]
    fn edit_commands_reach_the_field() {
        let mut field = Field(Vec::new());
        for (_, command) in EditCommand::IDS {
            command.apply(&mut field);
        }
        assert_eq!(field.0, ["cut", "copy", "paste", "select all"]);
    }

    #[cfg(windows)]
    #[test]
    fn win32_items() {
        let menu = Menu::new().action(1, "Play", false, false, Some("Ctrl+Alt+Ins")).separator().action(2, "Dark", true, true, None);
        let items = inspect(&menu);
        assert_eq!(items[0], (1, true, false, "Play\tCtrl+Alt+Ins".to_string()));
        assert_eq!(items[1].0, 0);
        assert_eq!(items[2], (2, false, true, "Dark".to_string()));
    }
}
