// One Qust at a time: what tauri-plugin-single-instance 2.5 did, with the same names and message,
// so a launch of either version hands over to whichever runs. The first instance holds the named
// mutex "<id>-sim" and a hidden window (class "<id>-sic", title "<id>-siw"); a second launch finds
// the mutex, lets the first come to the front (AllowSetForegroundWindow), sends it its working
// directory and arguments in a WM_COPYDATA (dwData 1543, NUL-separated), and exits.
//
// Debug builds use another id (`app_id`), so a development build never hands over to the
// installed Qust running in the tray (the Tauri app skipped single-instance in debug for that).

// The bundle identifier (tauri.conf.json), which names the mutex and window.
pub const IDENTIFIER: &str = "com.pascalmuangkhot.qust";

pub fn app_id() -> String {
    if cfg!(debug_assertions) {
        format!("{IDENTIFIER}.debug")
    } else {
        IDENTIFIER.to_string()
    }
}

// The payload: cwd, then each argument, separated by NUL (no terminator). The legacy format (1542)
// joined them with '|' and ended with a NUL.
const DATA_NUL: usize = 1543;
const DATA_LEGACY: usize = 1542;

pub fn encode(cwd: &str, args: &[String]) -> Vec<u8> {
    let mut data = cwd.to_string();
    for arg in args {
        data.push('\0');
        data.push_str(arg);
    }
    data.into_bytes()
}

// (args, cwd) from a WM_COPYDATA payload, or None for another kind of message.
pub fn decode(kind: usize, bytes: &[u8]) -> Option<(Vec<String>, String)> {
    let separator = match kind {
        DATA_NUL => '\0',
        DATA_LEGACY => '|',
        _ => return None,
    };
    let bytes = if separator == '|' { bytes.split(|byte| *byte == 0).next().unwrap_or_default() } else { bytes };
    let text = String::from_utf8_lossy(bytes);
    let mut parts = text.split(separator);
    let cwd = parts.next().unwrap_or_default().to_string();
    Some((parts.map(str::to_string).collect(), cwd))
}

pub enum Instance {
    // This is the first instance; keep the lock for as long as the app runs.
    Primary(InstanceLock),
    // Another instance runs and has been given our arguments (`delivered` false when its window
    // couldn't be reached within a second). The caller exits.
    Secondary { delivered: bool },
}

#[cfg(windows)]
mod imp {
    use super::*;
    use crate::native::{emit, Event, NativeThread};
    use crate::win::{self, HiddenWindow};
    use std::cell::RefCell;
    use windows::core::PCWSTR;
    use windows::Win32::Foundation::{CloseHandle, GetLastError, ERROR_ALREADY_EXISTS, HANDLE, HWND, LPARAM, LRESULT, WPARAM};
    use windows::Win32::System::DataExchange::COPYDATASTRUCT;
    use windows::Win32::System::Threading::{CreateMutexW, ReleaseMutex};
    use windows::Win32::UI::WindowsAndMessaging::*;

    const FORWARD_TIMEOUT_MS: u32 = 10_000;

    thread_local! {
        // The listening window, on the shell thread.
        static WINDOW: RefCell<Option<HiddenWindow>> = const { RefCell::new(None) };
    }

    pub struct InstanceLock {
        id: String,
        mutex: isize,
        native: Option<NativeThread>,
    }

    impl InstanceLock {
        // Starts receiving second launches, as `Event::Launched`, on the shell thread.
        pub fn listen(&mut self, native: &NativeThread) -> bool {
            let class = format!("{}-sic", self.id);
            let title = format!("{}-siw", self.id);
            self.native = Some(native.clone());
            native
                .call(move || {
                    let window = HiddenWindow::with_proc(&class, Some(&title), proc).ok();
                    let listening = window.is_some();
                    WINDOW.with(|cell| *cell.borrow_mut() = window);
                    listening
                })
                .unwrap_or(false)
        }

        // Lets the next launch be the first instance (before running the updater's installer,
        // which starts Qust again). Also done on drop.
        pub fn release(mut self) {
            self.release_now();
        }

        fn release_now(&mut self) {
            if let Some(native) = self.native.take() {
                native.call(stop_listening_on_thread);
            }
            if self.mutex != 0 {
                unsafe {
                    let handle = HANDLE(self.mutex as *mut _);
                    let _ = ReleaseMutex(handle);
                    let _ = CloseHandle(handle);
                }
                self.mutex = 0;
            }
        }
    }

    impl Drop for InstanceLock {
        fn drop(&mut self) {
            self.release_now();
        }
    }

    pub(crate) fn stop_listening_on_thread() {
        WINDOW.with(|cell| cell.borrow_mut().take());
    }

    unsafe extern "system" fn proc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
        if msg == WM_COPYDATA {
            let data = &*(lparam.0 as *const COPYDATASTRUCT);
            let bytes = if data.lpData.is_null() { &[][..] } else { std::slice::from_raw_parts(data.lpData as *const u8, data.cbData as usize) };
            return match decode(data.dwData, bytes) {
                Some((args, cwd)) => {
                    emit(Event::Launched { args, cwd });
                    LRESULT(1)
                }
                None => LRESULT(1),
            };
        }
        DefWindowProcW(hwnd, msg, wparam, lparam)
    }

    // Takes the instance lock for `id`, or hands this launch's arguments to the instance holding
    // it.
    pub fn acquire(id: &str) -> Instance {
        acquire_with(id, &std::env::args_os().map(|arg| arg.to_string_lossy().into_owned()).collect::<Vec<_>>())
    }

    pub fn acquire_with(id: &str, args: &[String]) -> Instance {
        let name = win::wide(&format!("{id}-sim"));
        let mutex = unsafe { CreateMutexW(None, true, PCWSTR(name.as_ptr())) };
        let existed = unsafe { GetLastError() } == ERROR_ALREADY_EXISTS;
        let mutex = match mutex {
            Ok(handle) => handle,
            // Without a mutex there is no lock to take; run as the first instance.
            Err(_) => return Instance::Primary(InstanceLock { id: id.to_string(), mutex: 0, native: None }),
        };
        if !existed {
            return Instance::Primary(InstanceLock { id: id.to_string(), mutex: mutex.0 as isize, native: None });
        }
        unsafe {
            let _ = CloseHandle(mutex);
        }
        Instance::Secondary { delivered: forward(id, args) }
    }

    fn forward(id: &str, args: &[String]) -> bool {
        let class = win::wide(&format!("{id}-sic"));
        let title = win::wide(&format!("{id}-siw"));
        unsafe {
            // A first instance launched at the same moment has the mutex before its window.
            let mut window = None;
            for _ in 0..40 {
                if let Ok(found) = FindWindowW(PCWSTR(class.as_ptr()), PCWSTR(title.as_ptr())) {
                    if !found.is_invalid() {
                        window = Some(found);
                        break;
                    }
                }
                std::thread::sleep(std::time::Duration::from_millis(25));
            }
            let Some(window) = window else { return false };
            // Only the first instance may bring its window to the front now: hand it that right.
            let mut pid = 0u32;
            GetWindowThreadProcessId(window, Some(&mut pid));
            if pid != 0 {
                let _ = AllowSetForegroundWindow(pid);
            }
            let cwd = std::env::current_dir().unwrap_or_default().to_string_lossy().into_owned();
            let bytes = encode(&cwd, args);
            let data = COPYDATASTRUCT { dwData: DATA_NUL, cbData: bytes.len() as u32, lpData: bytes.as_ptr() as *mut _ };
            let sent = SendMessageTimeoutW(window, WM_COPYDATA, WPARAM(0), LPARAM(&data as *const _ as isize), SMTO_NORMAL, FORWARD_TIMEOUT_MS, None);
            sent.0 != 0
        }
    }
}

#[cfg(windows)]
pub use imp::{acquire, acquire_with, InstanceLock};
#[cfg(windows)]
pub(crate) use imp::stop_listening_on_thread;

// macOS: the Dock already keeps one copy of an app bundle running and reactivates it ("reopen"),
// so the lock is always ours and nothing is forwarded. `qust --toggle` from a terminal or a
// Stream Deck would need a socket; see the report.
#[cfg(not(windows))]
pub struct InstanceLock;

#[cfg(not(windows))]
impl InstanceLock {
    pub fn listen(&mut self, _native: &crate::NativeThread) -> bool {
        false
    }

    pub fn release(self) {}
}

#[cfg(not(windows))]
pub fn acquire(_id: &str) -> Instance {
    Instance::Primary(InstanceLock)
}

#[cfg(not(windows))]
pub fn acquire_with(_id: &str, _args: &[String]) -> Instance {
    Instance::Primary(InstanceLock)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn payload_round_trips() {
        let args = vec![r"C:\Program Files\Qust\qust.exe".to_string(), "--toggle".to_string(), "a|b".to_string(), String::new()];
        let bytes = encode(r"C:\Users\p", &args);
        assert_eq!(decode(DATA_NUL, &bytes), Some((args, r"C:\Users\p".to_string())));
        // The legacy format: '|' separated, NUL terminated.
        assert_eq!(decode(DATA_LEGACY, b"C:\\x|qust.exe|--background\0"), Some((vec!["qust.exe".to_string(), "--background".to_string()], "C:\\x".to_string())));
        assert_eq!(decode(7, b"whatever"), None);
        assert_eq!(decode(DATA_NUL, b""), Some((vec![], String::new())));
    }

    #[test]
    fn debug_builds_use_their_own_id() {
        if cfg!(debug_assertions) {
            assert_eq!(app_id(), "com.pascalmuangkhot.qust.debug");
        } else {
            assert_eq!(app_id(), IDENTIFIER);
        }
    }
}
