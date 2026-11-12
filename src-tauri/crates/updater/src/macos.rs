// Installing on macOS: the update is the .app bundle in a .tar.gz, which replaces the running
// bundle where it is, as tauri-plugin-updater 2.14 does it: unpacked next to it on the same volume,
// then moved into place, with the old bundle kept until the new one is there and put back if the
// move fails. When the user can't write where the app is (an /Applications owned by another
// admin), the moves run with administrator rights after a password prompt. Untested here: it's
// written from the Tauri updater's code and compiles, but has not run on a Mac.
//
// The caller starts the app again afterwards (the bundle path and the executable name are the
// same, so the LaunchAgent that starts Qust at login keeps working).

use crate::archive::unpack_bundle;
use crate::Error;
use std::os::unix::fs::{MetadataExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

// /Applications/Qust.app for /Applications/Qust.app/Contents/MacOS/qust.
pub fn bundle_of(exe: &Path) -> Option<PathBuf> {
    let macos = exe.parent()?;
    let contents = macos.parent()?;
    let bundle = contents.parent()?;
    let is_bundle = macos.file_name()? == "MacOS"
        && contents.file_name()? == "Contents"
        && bundle.extension().is_some_and(|extension| extension == "app");
    is_bundle.then(|| bundle.to_path_buf())
}

pub fn install(bytes: &[u8]) -> Result<(), Error> {
    let fail = |error: std::io::Error| Error::Install(error.to_string());
    let exe = std::env::current_exe().map_err(fail)?;
    let bundle = bundle_of(&exe).ok_or_else(|| Error::Install("Qust is not running from its app bundle".into()))?;

    // Renames only work within one volume: the temp dir if it's on the app's, or else next to it.
    let device = std::fs::symlink_metadata(&bundle).map_err(fail)?.dev();
    let root = [Some(std::env::temp_dir()), bundle.parent().map(Path::to_path_buf)]
        .into_iter()
        .flatten()
        .find(|dir| dir.metadata().is_ok_and(|metadata| metadata.dev() == device))
        .ok_or_else(|| Error::Install("no temporary folder on the app's volume".into()))?;
    let stamp = format!(
        "{}-{}",
        std::process::id(),
        SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |time| time.as_nanos())
    );
    let staged = root.join(format!("qust-update-{stamp}"));
    let backup = root.join(format!("qust-previous-{stamp}"));

    std::fs::create_dir(&staged).map_err(fail)?;
    let unpacked = unpack_bundle(bytes, &staged)
        // The new bundle's root: a temp dir would be 0700, which other users couldn't open.
        .and_then(|()| std::fs::set_permissions(&staged, std::fs::Permissions::from_mode(0o755)).map_err(fail));
    if let Err(error) = unpacked {
        let _ = std::fs::remove_dir_all(&staged);
        return Err(error);
    }

    match std::fs::rename(&bundle, &backup) {
        Ok(()) => {
            if let Err(error) = std::fs::rename(&staged, &bundle) {
                // Best effort: the error worth reporting is the failed move.
                let _ = std::fs::rename(&backup, &bundle);
                let _ = std::fs::remove_dir_all(&staged);
                return Err(fail(error));
            }
            let _ = std::fs::remove_dir_all(&backup);
        }
        Err(error) if error.kind() == std::io::ErrorKind::PermissionDenied => {
            let result = install_as_admin(&bundle, &staged, &backup);
            if result.is_err() {
                let _ = std::fs::remove_dir_all(&staged);
            }
            result?;
        }
        Err(error) => {
            let _ = std::fs::remove_dir_all(&staged);
            return Err(fail(error));
        }
    }

    // So Finder and Launch Services notice the new bundle.
    let _ = Command::new("touch").arg(&bundle).status();
    Ok(())
}

// The same moves with administrator rights, through AppleScript's password prompt.
fn install_as_admin(bundle: &Path, staged: &Path, backup: &Path) -> Result<(), Error> {
    let (bundle, staged, backup) = (sh_quote(bundle), sh_quote(staged), sh_quote(backup));
    let command = format!(
        "mv -f {bundle} {backup} && {{ mv -f {staged} {bundle} || {{ mv -f {backup} {bundle}; exit 1; }}; }} && rm -rf {backup}"
    );
    let script = format!("do shell script {} with administrator privileges", applescript_string(&command));
    let status = Command::new("/usr/bin/osascript")
        .arg("-e")
        .arg(script)
        .status()
        .map_err(|error| Error::Install(error.to_string()))?;
    if status.success() {
        Ok(())
    } else {
        Err(Error::Install("the new version couldn't be moved into place".into()))
    }
}

fn sh_quote(path: &Path) -> String {
    format!("'{}'", path.to_string_lossy().replace('\'', r"'\''"))
}

fn applescript_string(text: &str) -> String {
    format!("\"{}\"", text.replace('\\', r"\\").replace('"', "\\\""))
}
