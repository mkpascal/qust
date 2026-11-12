// System-wide hotkeys: RegisterHotKey on the shell thread's window (WM_HOTKEY comes back there as
// `Event::Hotkey(id)`), as global-hotkey did for tauri-plugin-global-shortcut: MOD_NOREPEAT, and
// the same virtual keys. macOS: Carbon's RegisterEventHotKey (see `mac`).

use crate::accelerator::Accelerator;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HotkeyError {
    // Another program (or this one, under another id) holds these keys.
    AlreadyRegistered,
    // The key has no virtual key code here.
    UnknownKey,
    Os(String),
}

// What `Shortcuts` registers through, so its rules are tested without the OS.
pub trait HotkeyBackend: Send + Sync {
    fn register(&self, id: i32, accelerator: &Accelerator) -> Result<(), HotkeyError>;
    fn unregister(&self, id: i32);
}

#[cfg(windows)]
mod imp {
    use super::*;
    use crate::native::NativeThread;
    use crate::win;
    use std::cell::RefCell;
    use windows::Win32::Foundation::{ERROR_HOTKEY_ALREADY_REGISTERED, HWND};
    use windows::Win32::UI::Input::KeyboardAndMouse::{RegisterHotKey, UnregisterHotKey, HOT_KEY_MODIFIERS, MOD_ALT, MOD_CONTROL, MOD_NOREPEAT, MOD_SHIFT, MOD_WIN};

    thread_local! {
        // The ids registered on this (the shell) thread, to unregister them when it stops.
        static REGISTERED: RefCell<Vec<i32>> = const { RefCell::new(Vec::new()) };
    }

    pub struct Hotkeys {
        native: NativeThread,
    }

    impl Hotkeys {
        pub fn new(native: &NativeThread) -> Self {
            Hotkeys { native: native.clone() }
        }
    }

    impl HotkeyBackend for Hotkeys {
        fn register(&self, id: i32, accelerator: &Accelerator) -> Result<(), HotkeyError> {
            let vk = accelerator.vk();
            if vk == 0 {
                return Err(HotkeyError::UnknownKey);
            }
            let mut mods = MOD_NOREPEAT;
            if accelerator.mods.shift {
                mods |= MOD_SHIFT;
            }
            if accelerator.mods.meta {
                mods |= MOD_WIN;
            }
            if accelerator.mods.alt {
                mods |= MOD_ALT;
            }
            if accelerator.mods.ctrl {
                mods |= MOD_CONTROL;
            }
            let window = self.native.hwnd();
            self.native
                .call(move || register_on_thread(win::hwnd(window), id, mods, vk))
                .unwrap_or_else(|| Err(HotkeyError::Os("the shell thread has stopped".into())))
        }

        fn unregister(&self, id: i32) {
            let window = self.native.hwnd();
            self.native.call(move || unsafe {
                let _ = UnregisterHotKey(Some(win::hwnd(window)), id);
                REGISTERED.with(|ids| ids.borrow_mut().retain(|other| *other != id));
            });
        }
    }

    fn register_on_thread(window: HWND, id: i32, mods: HOT_KEY_MODIFIERS, vk: u16) -> Result<(), HotkeyError> {
        unsafe {
            match RegisterHotKey(Some(window), id, mods, vk as u32) {
                Ok(()) => {
                    REGISTERED.with(|ids| ids.borrow_mut().push(id));
                    Ok(())
                }
                Err(error) if error.code() == ERROR_HOTKEY_ALREADY_REGISTERED.to_hresult() => Err(HotkeyError::AlreadyRegistered),
                Err(error) => Err(HotkeyError::Os(error.message())),
            }
        }
    }

    pub(crate) fn unregister_all_on_thread(window: HWND) {
        let ids = REGISTERED.with(|ids| std::mem::take(&mut *ids.borrow_mut()));
        for id in ids {
            unsafe {
                let _ = UnregisterHotKey(Some(window), id);
            }
        }
    }
}

#[cfg(windows)]
pub use imp::Hotkeys;
#[cfg(windows)]
pub(crate) use imp::unregister_all_on_thread;

// Elsewhere (macOS is best effort, see the report): nothing can be registered, which Settings
// shows as "Another app is using this shortcut".
#[cfg(not(windows))]
pub struct Hotkeys;

#[cfg(not(windows))]
impl Hotkeys {
    pub fn new(_native: &crate::NativeThread) -> Self {
        Hotkeys
    }
}

#[cfg(not(windows))]
impl HotkeyBackend for Hotkeys {
    fn register(&self, _id: i32, _accelerator: &Accelerator) -> Result<(), HotkeyError> {
        Err(HotkeyError::Os("global shortcuts aren't supported on this platform yet".into()))
    }

    fn unregister(&self, _id: i32) {}
}
