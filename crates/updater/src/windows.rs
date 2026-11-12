// Installing on Windows: the update is the NSIS installer (Qust_<version>_x64-setup.exe). It's
// started the way tauri-plugin-updater 2.14 starts it in `passive` mode, with the same arguments,
// so the installer (packaging/windows/installer.nsi) behaves the same whichever updater ran it:
//   /P       passive: a progress bar only, no pages
//   /UPDATE  an update: no uninstall first, no new shortcuts
//   /R       start Qust again when done
//   /ARGS …  the arguments for that start, the rest of the command line
// The installer closes Qust if it's still running (also a tray-only one), so the caller just
// quits right after this returns, as the Tauri updater's process::exit(0) did.

use crate::Error;
use std::ffi::{OsStr, OsString};
use std::os::windows::ffi::{OsStrExt, OsStringExt};
use std::os::windows::process::CommandExt;
use std::process::{Command, Stdio};

pub fn install(bytes: &[u8], version: &str, relaunch_args: &[OsString]) -> Result<(), Error> {
    // Every Windows executable starts with "MZ"; the Tauri updater checks the same (infer::app::is_exe).
    if !bytes.starts_with(b"MZ") {
        return Err(Error::Install("the update is not a Windows installer".into()));
    }
    let fail = |error: std::io::Error| Error::Install(error.to_string());
    let dir = std::env::temp_dir().join(format!("Qust-{version}-updater-{}", std::process::id()));
    std::fs::create_dir_all(&dir).map_err(fail)?;
    let path = dir.join(format!("Qust-{version}-installer.exe"));
    std::fs::write(&path, bytes).map_err(fail)?;

    // raw_arg: NSIS reads its own command line, so it gets exactly this text. The installer asks
    // for no elevation (per-user install), so a plain CreateProcess starts it.
    Command::new(&path)
        .raw_arg(parameters(relaunch_args))
        .current_dir(&dir)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(fail)?;
    Ok(())
}

pub(crate) fn parameters(relaunch_args: &[OsString]) -> OsString {
    let mut parameters = OsString::from("/P /UPDATE /R /ARGS");
    for arg in relaunch_args {
        parameters.push(" ");
        parameters.push(escape(arg));
    }
    parameters
}

// tauri-plugin-updater's `escape_nsis_current_exe_arg` (Apache-2.0 OR MIT, Tauri Programme within
// The Commons Conservancy), itself adapted from Rust's std: Windows command line quoting, plus
// quotes around any `/` so NSIS doesn't take it for one of its options.
fn escape(arg: &OsStr) -> OsString {
    let quote = arg.as_encoded_bytes().iter().any(|c| *c == b' ' || *c == b'\t' || *c == b'/') || arg.is_empty();
    let mut cmd: Vec<u16> = Vec::new();
    if quote {
        cmd.push('"' as u16);
    }
    let mut backslashes: usize = 0;
    for x in arg.encode_wide() {
        if x == '\\' as u16 {
            backslashes += 1;
        } else {
            if x == '"' as u16 {
                // n+1 backslashes, 2n+1 in all, before an inner quote
                cmd.extend((0..=backslashes).map(|_| '\\' as u16));
            }
            backslashes = 0;
        }
        cmd.push(x);
    }
    if quote {
        // n more backslashes, 2n in all, before the closing quote
        cmd.extend((0..backslashes).map(|_| '\\' as u16));
        cmd.push('"' as u16);
    }
    OsString::from_wide(&cmd)
}
