// Qust's system shell: what the operating system provides that the UI toolkit doesn't, and that
// Tauri and its plugins did until now. Global shortcuts, the tray icon and background mode, start
// with Windows, one instance at a time, native menus, owned panels and the title bar theme, the
// drawer's window animation, the file picker, and the updater's schedule.
//
// Nothing here needs GPUI except `gpui_glue` (feature "gpui"): the rest takes window handles and
// callbacks, so it is tested without a UI.

pub mod accelerator;
pub mod background;
#[doc(hidden)]
pub mod diag;
pub mod drawer;
pub mod hotkeys;
pub mod instance;
pub mod menu;
pub mod picker;
pub mod recorder;
pub mod shell;
pub mod shortcuts;
pub mod startup;
pub mod theme;
pub mod tray;
pub mod updates;
pub mod window;

#[cfg(windows)]
mod native;
#[cfg(not(windows))]
#[path = "native_other.rs"]
mod native;
#[cfg(windows)]
mod win;

#[cfg(feature = "gpui")]
pub mod gpui_glue;

pub use native::{Event, NativeThread};
pub use shell::{Shell, Started};

// Gives the process Qust's AppUserModelID (the bundle identifier, which the installer also puts
// on the Start menu and desktop shortcuts), so its windows group with the pinned Qust whether it
// was started from a shortcut, by the helper at sign-in, or by the updater. Call it before the
// first window. (Tauri set none: windows started outside a shortcut fell under the exe's default
// id, apart from the pin.) GPUI's `App::set_app_identity` does the same plus toast registration.
pub fn set_app_identity() -> bool {
    #[cfg(windows)]
    unsafe {
        let id = windows::core::HSTRING::from(instance::IDENTIFIER);
        windows::Win32::UI::Shell::SetCurrentProcessExplicitAppUserModelID(&id).is_ok()
    }
    #[cfg(not(windows))]
    false
}
