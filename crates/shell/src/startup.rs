// Start with Windows (and macOS), for the background helper: what tauri-plugin-autostart 2.7
// (auto-launch 0.6) wrote, so existing entries keep working and the installer's uninstaller still
// finds them.
//
// Windows: HKCU\Software\Microsoft\Windows\CurrentVersion\Run, value "Qust" = `<exe> --background`
// (the path unquoted, as auto-launch wrote it), plus the 12-byte "enabled" value under
// Explorer\StartupApproved\Run when that key exists. It counts as enabled when the Run value exists
// (HKLM or HKCU) and Task Manager hasn't disabled it in either StartupApproved key. Disabling
// removes the Run value from both.
//
// macOS: ~/Library/LaunchAgents/Qust.plist running the executable with --background at login.

// productName in tauri.conf.json, which named the entry.
pub const NAME: &str = "Qust";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StartupEntry {
    pub name: String,
    pub program: String,
    pub args: Vec<String>,
}

impl StartupEntry {
    // Qust's own entry: this executable with --background.
    pub fn for_app() -> std::io::Result<StartupEntry> {
        let exe = std::env::current_exe()?;
        #[cfg(target_os = "macos")]
        let exe = exe.canonicalize()?;
        Ok(StartupEntry { name: NAME.to_string(), program: exe.display().to_string(), args: vec![crate::background::LAUNCH_ARG.to_string()] })
    }

    pub fn new(name: &str, program: &str, args: &[&str]) -> StartupEntry {
        StartupEntry { name: name.to_string(), program: program.to_string(), args: args.iter().map(|arg| arg.to_string()).collect() }
    }

    // The Run value: auto-launch's `format!("{} {}", path, args.join(" "))`.
    pub fn command(&self) -> String {
        format!("{} {}", self.program, self.args.join(" "))
    }

    // The LaunchAgent, as auto-launch builds it (no bundle identifiers, no extra config).
    pub fn launch_agent_plist(&self) -> String {
        let mut arguments = format!("<string>{}</string>", self.program);
        for arg in &self.args {
            arguments.push_str(&format!("<string>{arg}</string>"));
        }
        format!(
            "{}\n{}\n<plist version=\"1.0\">\n  <dict>\n  <key>Label</key>\n  <string>{}</string>\n  <key>AssociatedBundleIdentifiers</key>\n  <array></array>\n  <key>ProgramArguments</key>\n  <array>{}</array>\n  <key>RunAtLoad</key>\n  <true/>\n  </dict>\n</plist>",
            r#"<?xml version="1.0" encoding="UTF-8"?>"#,
            r#"<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">"#,
            self.name,
            arguments
        )
    }
}

// Task Manager writes 12 bytes: 2 then zeros when enabled, 3 and a timestamp when disabled. Like
// auto-launch: enabled when the last eight bytes are zero (or the value is too short to tell).
pub fn approved(value: &[u8]) -> bool {
    if value.len() < 8 {
        return true;
    }
    value.iter().rev().take(8).all(|byte| *byte == 0)
}

const APPROVED_ENABLED: [u8; 12] = [0x02, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0];

#[cfg(windows)]
mod imp {
    use super::*;
    use windows_registry::{Key, Type, CURRENT_USER, LOCAL_MACHINE};

    pub const RUN_KEY: &str = r"SOFTWARE\Microsoft\Windows\CurrentVersion\Run";
    pub const APPROVED_KEY: &str = r"SOFTWARE\Microsoft\Windows\CurrentVersion\Explorer\StartupApproved\Run";

    // HRESULTs of ERROR_FILE_NOT_FOUND and ERROR_ACCESS_DENIED.
    const NOT_FOUND: i32 = 0x80070002_u32 as i32;
    const ACCESS_DENIED: i32 = 0x80070005_u32 as i32;

    impl StartupEntry {
        pub fn enable(&self) -> Result<(), String> {
            // auto-launch tried HKLM first and fell back to HKCU on "access denied", which is
            // where it always landed for a per-user install: HKCU directly.
            CURRENT_USER.create(RUN_KEY).and_then(|key| key.set_string(&self.name, self.command())).map_err(|error| error.message())?;
            match CURRENT_USER.options().write().open(APPROVED_KEY) {
                Ok(key) => key.set_bytes(&self.name, Type::Bytes, &APPROVED_ENABLED).map_err(|error| error.message()),
                Err(error) if error.code().0 == NOT_FOUND => Ok(()),
                Err(error) => Err(error.message()),
            }
        }

        pub fn disable(&self) -> Result<(), String> {
            for (root, is_machine) in [(LOCAL_MACHINE, true), (CURRENT_USER, false)] {
                match root.options().write().open(RUN_KEY).and_then(|key| key.remove_value(&self.name)) {
                    Ok(()) => {}
                    Err(error) if error.code().0 == NOT_FOUND => {}
                    Err(error) if is_machine && error.code().0 == ACCESS_DENIED => {}
                    Err(error) => return Err(error.message()),
                }
            }
            Ok(())
        }

        pub fn is_enabled(&self) -> Result<bool, String> {
            let registered = |root: &Key| match root.open(RUN_KEY).and_then(|key| key.get_string(&self.name)) {
                Ok(_) => Ok(true),
                Err(error) if error.code().0 == NOT_FOUND => Ok(false),
                Err(error) => Err(error.message()),
            };
            let not_disabled = |root: &Key| match root.open(APPROVED_KEY).and_then(|key| key.get_value(&self.name)) {
                Ok(value) => Ok(approved(&value)),
                Err(error) if error.code().0 == NOT_FOUND => Ok(true),
                Err(error) => Err(error.message()),
            };
            if !(registered(LOCAL_MACHINE)? || registered(CURRENT_USER)?) {
                return Ok(false);
            }
            Ok(not_disabled(LOCAL_MACHINE)? && not_disabled(CURRENT_USER)?)
        }

        // The Run value as it is now (tests check it is left exactly as found).
        pub fn current_command(&self) -> Option<String> {
            CURRENT_USER.open(RUN_KEY).and_then(|key| key.get_string(&self.name)).ok()
        }

        pub fn current_approval(&self) -> Option<Vec<u8>> {
            CURRENT_USER.open(APPROVED_KEY).and_then(|key| key.get_value(&self.name)).ok().map(|value| value.to_vec())
        }

        // Removes the StartupApproved value too (tests clean up after themselves with it).
        pub fn forget_approval(&self) {
            if let Ok(key) = CURRENT_USER.options().write().open(APPROVED_KEY) {
                let _ = key.remove_value(&self.name);
            }
        }
    }
}

#[cfg(target_os = "macos")]
mod imp {
    use super::*;
    use std::path::PathBuf;

    fn file(name: &str) -> Result<PathBuf, String> {
        let home = dirs::home_dir().ok_or("no home directory")?;
        Ok(home.join("Library").join("LaunchAgents").join(format!("{name}.plist")))
    }

    impl StartupEntry {
        pub fn enable(&self) -> Result<(), String> {
            let path = file(&self.name)?;
            if let Some(dir) = path.parent() {
                std::fs::create_dir_all(dir).map_err(|error| error.to_string())?;
            }
            std::fs::write(path, self.launch_agent_plist()).map_err(|error| error.to_string())
        }

        pub fn disable(&self) -> Result<(), String> {
            let path = file(&self.name)?;
            if path.exists() {
                std::fs::remove_file(path).map_err(|error| error.to_string())?;
            }
            Ok(())
        }

        pub fn is_enabled(&self) -> Result<bool, String> {
            Ok(file(&self.name)?.exists())
        }
    }
}

#[cfg(not(any(windows, target_os = "macos")))]
impl StartupEntry {
    pub fn enable(&self) -> Result<(), String> {
        Err("not supported on this platform".into())
    }

    pub fn disable(&self) -> Result<(), String> {
        Ok(())
    }

    pub fn is_enabled(&self) -> Result<bool, String> {
        Ok(false)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn command_like_auto_launch() {
        let entry = StartupEntry::new("Qust", r"C:\Users\p\AppData\Local\Qust\qust.exe", &["--background"]);
        assert_eq!(entry.command(), r"C:\Users\p\AppData\Local\Qust\qust.exe --background");
    }

    #[test]
    fn approval_bytes() {
        assert!(approved(&APPROVED_ENABLED));
        assert!(approved(&[0x06, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]));
        assert!(!approved(&[0x03, 0, 0, 0, 0x10, 0x20, 0x30, 0x40, 0x50, 0x60, 0x70, 0x01]));
        assert!(approved(&[0x03]));
    }

    #[test]
    fn launch_agent() {
        let entry = StartupEntry::new("Qust", "/Applications/Qust.app/Contents/MacOS/qust", &["--background"]);
        let plist = entry.launch_agent_plist();
        assert!(plist.contains("<key>Label</key>\n  <string>Qust</string>"));
        assert!(plist.contains("<array><string>/Applications/Qust.app/Contents/MacOS/qust</string><string>--background</string></array>"));
        assert!(plist.contains("<key>RunAtLoad</key>\n  <true/>"));
    }
}
