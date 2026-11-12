// The macOS update archive (Qust_universal.app.tar.gz) holds one bundle, `Qust.app/…`. Like the
// Tauri updater, this unpacks what's inside it into `dir`, which becomes the new bundle.

use crate::Error;
use std::path::{Component, Path, PathBuf};

pub fn unpack_bundle(bytes: &[u8], dir: &Path) -> Result<(), Error> {
    let fail = |error: std::io::Error| Error::Install(format!("unpacking the update: {error}"));
    let mut archive = tar::Archive::new(flate2::read::GzDecoder::new(bytes));
    for entry in archive.entries().map_err(fail)? {
        let mut entry = entry.map_err(fail)?;
        let path = entry.path().map_err(fail)?.into_owned();
        // The signature vouches for the archive, but a path that climbs out is never right.
        if path.components().any(|component| !matches!(component, Component::Normal(_) | Component::CurDir)) {
            return Err(Error::Install(format!("unexpected path in the update: {}", path.display())));
        }
        let inside: PathBuf = path.components().filter(|component| matches!(component, Component::Normal(_))).skip(1).collect();
        let target = dir.join(&inside);
        if let Some(parent) = target.parent() {
            std::fs::create_dir_all(parent).map_err(fail)?;
        }
        entry.unpack(&target).map_err(fail)?;
    }
    if !dir.join("Contents").join("Info.plist").is_file() {
        return Err(Error::Install("the update holds no app bundle".into()));
    }
    Ok(())
}
