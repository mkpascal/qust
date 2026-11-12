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

// macOS: Carbon's RegisterEventHotKey, what global-hotkey did for tauri-plugin-global-shortcut
// there. Untested (see CLAUDE.md). Registered on the main thread (the app calls in from there);
// the hot key events arrive through the application event target while the main run loop runs.
#[cfg(target_os = "macos")]
mod mac {
    use super::*;
    use crate::native::{Event, NativeThread};
    use std::collections::HashMap;
    use std::ffi::c_void;
    use std::sync::{Mutex, OnceLock};

    type OSStatus = i32;

    #[repr(C)]
    #[derive(Clone, Copy)]
    struct EventHotKeyID {
        signature: u32,
        id: u32,
    }

    #[repr(C)]
    struct EventTypeSpec {
        event_class: u32,
        event_kind: u32,
    }

    type Handler = extern "C" fn(*mut c_void, *mut c_void, *mut c_void) -> OSStatus;

    #[link(name = "Carbon", kind = "framework")]
    unsafe extern "C" {
        fn GetApplicationEventTarget() -> *mut c_void;
        fn InstallEventHandler(target: *mut c_void, handler: Handler, count: u32, types: *const EventTypeSpec, user_data: *mut c_void, out: *mut *mut c_void) -> OSStatus;
        fn RegisterEventHotKey(key_code: u32, modifiers: u32, id: EventHotKeyID, target: *mut c_void, options: u32, out: *mut *mut c_void) -> OSStatus;
        fn UnregisterEventHotKey(hot_key: *mut c_void) -> OSStatus;
        fn GetEventParameter(event: *mut c_void, name: u32, desired_type: u32, actual_type: *mut u32, size: usize, actual_size: *mut usize, data: *mut c_void) -> OSStatus;
    }

    const fn four(code: &[u8; 4]) -> u32 {
        u32::from_be_bytes(*code)
    }

    const SIGNATURE: u32 = four(b"Qust");
    const K_EVENT_CLASS_KEYBOARD: u32 = four(b"keyb");
    const K_EVENT_HOT_KEY_PRESSED: u32 = 5;
    const K_EVENT_PARAM_DIRECT_OBJECT: u32 = four(b"----");
    const TYPE_EVENT_HOT_KEY_ID: u32 = four(b"hkid");
    const EVENT_HOT_KEY_EXISTS_ERR: OSStatus = -9878;
    const CMD_KEY: u32 = 0x0100;
    const SHIFT_KEY: u32 = 0x0200;
    const OPTION_KEY: u32 = 0x0800;
    const CONTROL_KEY: u32 = 0x1000;

    // The registered hot keys' refs by id (as usize: the refs are only used on the main thread).
    static REGISTERED: Mutex<Option<HashMap<i32, usize>>> = Mutex::new(None);
    static NATIVE: OnceLock<NativeThread> = OnceLock::new();

    extern "C" fn on_hot_key(_: *mut c_void, event: *mut c_void, _: *mut c_void) -> OSStatus {
        let mut id = EventHotKeyID { signature: 0, id: 0 };
        let status = unsafe {
            GetEventParameter(event, K_EVENT_PARAM_DIRECT_OBJECT, TYPE_EVENT_HOT_KEY_ID, std::ptr::null_mut(), std::mem::size_of::<EventHotKeyID>(), std::ptr::null_mut(), &mut id as *mut _ as *mut c_void)
        };
        if status == 0 && id.signature == SIGNATURE {
            if let Some(native) = NATIVE.get() {
                native.emit(Event::Hotkey(id.id as i32));
            }
        }
        0
    }

    pub struct Hotkeys;

    impl Hotkeys {
        pub fn new(native: &NativeThread) -> Self {
            if NATIVE.set(native.clone()).is_ok() {
                let spec = EventTypeSpec { event_class: K_EVENT_CLASS_KEYBOARD, event_kind: K_EVENT_HOT_KEY_PRESSED };
                unsafe {
                    InstallEventHandler(GetApplicationEventTarget(), on_hot_key, 1, &spec, std::ptr::null_mut(), std::ptr::null_mut());
                }
            }
            Hotkeys
        }
    }

    impl HotkeyBackend for Hotkeys {
        fn register(&self, id: i32, accelerator: &Accelerator) -> Result<(), HotkeyError> {
            let Some(key) = accelerator.mac_key_code() else { return Err(HotkeyError::UnknownKey) };
            let mut mods = 0;
            if accelerator.mods.meta {
                mods |= CMD_KEY;
            }
            if accelerator.mods.shift {
                mods |= SHIFT_KEY;
            }
            if accelerator.mods.alt {
                mods |= OPTION_KEY;
            }
            if accelerator.mods.ctrl {
                mods |= CONTROL_KEY;
            }
            let mut hot_key = std::ptr::null_mut();
            let status = unsafe { RegisterEventHotKey(key as u32, mods, EventHotKeyID { signature: SIGNATURE, id: id as u32 }, GetApplicationEventTarget(), 0, &mut hot_key) };
            match status {
                0 => {
                    REGISTERED.lock().unwrap_or_else(|poisoned| poisoned.into_inner()).get_or_insert_with(HashMap::new).insert(id, hot_key as usize);
                    Ok(())
                }
                EVENT_HOT_KEY_EXISTS_ERR => Err(HotkeyError::AlreadyRegistered),
                status => Err(HotkeyError::Os(format!("RegisterEventHotKey failed ({status})"))),
            }
        }

        fn unregister(&self, id: i32) {
            let hot_key = REGISTERED.lock().unwrap_or_else(|poisoned| poisoned.into_inner()).as_mut().and_then(|map| map.remove(&id));
            if let Some(hot_key) = hot_key {
                unsafe {
                    UnregisterEventHotKey(hot_key as *mut c_void);
                }
            }
        }
    }
}

#[cfg(target_os = "macos")]
pub use mac::Hotkeys;

// Elsewhere nothing can be registered, which Settings shows as "Another app is using this
// shortcut".
#[cfg(not(any(windows, target_os = "macos")))]
pub struct Hotkeys;

#[cfg(not(any(windows, target_os = "macos")))]
impl Hotkeys {
    pub fn new(_native: &crate::NativeThread) -> Self {
        Hotkeys
    }
}

#[cfg(not(any(windows, target_os = "macos")))]
impl HotkeyBackend for Hotkeys {
    fn register(&self, _id: i32, _accelerator: &Accelerator) -> Result<(), HotkeyError> {
        Err(HotkeyError::Os("global shortcuts aren't supported on this platform yet".into()))
    }

    fn unregister(&self, _id: i32) {}
}
