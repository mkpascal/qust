// Finding the code in a `.vst3`. A VST3 plugin is usually a bundle, a folder named `Name.vst3`
// with the Windows binary at `Contents/x86_64-win/Name.vst3`; older ones are a single file. The
// user may pick the bundle (from the list of installed plugins), the file, or the binary inside
// the bundle (the file picker walks into bundles like any folder): all three mean the bundle.

use std::path::{Path, PathBuf};

pub const NOT_64_BIT: &str = "This is a 32-bit plugin. Qust works with 64-bit VST3 plugins.";
pub const ARM: &str = "This plugin is built for ARM processors. Qust works with 64-bit VST3 plugins for Intel and AMD.";
pub const NOT_VST3: &str = "This isn't a VST3 plugin.";
pub const MISSING: &str = "Couldn't find the plugin file. It may have been moved or uninstalled.";

/// A plugin's code: the `.vst3` remembered (a bundle or a file) and the binary to load.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Located {
    pub bundle: PathBuf,
    pub binary: PathBuf,
}

/// The name shown before the plugin has told its own: the file name without `.vst3`.
pub fn display_name(path: &Path) -> String {
    let name = path.file_name().map(|name| name.to_string_lossy().to_string()).unwrap_or_default();
    match name.len().checked_sub(5) {
        Some(cut) if name.is_char_boundary(cut) && name[cut..].eq_ignore_ascii_case(".vst3") => name[..cut].to_string(),
        _ => name,
    }
}

fn is_vst3(path: &Path) -> bool {
    path.extension().is_some_and(|extension| extension.eq_ignore_ascii_case("vst3"))
}

fn named(path: Option<&Path>, name: &str) -> bool {
    path.and_then(Path::file_name).is_some_and(|file| file.eq_ignore_ascii_case(name))
}

/// Where the code is, for a picked or remembered path.
pub fn locate(path: &Path) -> Result<Located, String> {
    if path.is_dir() {
        let contents = path.join("Contents");
        for architecture in ["x86_64-win", "x86-win", "arm64-win", "arm64ec-win", "arm64x-win"] {
            let folder = contents.join(architecture);
            let Some(binary) = binary_in(&folder, path) else { continue };
            return match architecture {
                "x86_64-win" => Ok(Located { bundle: path.to_path_buf(), binary }),
                "x86-win" => Err(NOT_64_BIT.into()),
                _ => Err(ARM.into()),
            };
        }
        return Err(if path.exists() { NOT_VST3.into() } else { MISSING.into() });
    }
    if !path.is_file() {
        return Err(MISSING.into());
    }
    if !is_vst3(path) {
        return Err(NOT_VST3.into());
    }
    // The binary inside a bundle: remember the bundle.
    let folder = path.parent();
    let contents = folder.and_then(Path::parent);
    let bundle = contents.and_then(Path::parent);
    let in_bundle = named(contents, "Contents") && bundle.is_some_and(is_vst3);
    if in_bundle && named(folder, "x86-win") {
        return Err(NOT_64_BIT.into());
    }
    let bundle = if in_bundle { bundle.unwrap_or(path) } else { path };
    Ok(Located { bundle: bundle.to_path_buf(), binary: path.to_path_buf() })
}

// `Contents/<architecture>/<bundle name>.vst3`, or the only `.vst3` there.
fn binary_in(folder: &Path, bundle: &Path) -> Option<PathBuf> {
    let expected = folder.join(bundle.file_name()?);
    if expected.is_file() {
        return Some(expected);
    }
    let mut found = std::fs::read_dir(folder).ok()?.flatten().map(|entry| entry.path()).filter(|path| path.is_file() && is_vst3(path));
    let first = found.next()?;
    found.next().is_none().then_some(first)
}

/// What a binary was built for, from its PE header: Ok for x64, else why it won't load.
pub fn check_binary(binary: &Path) -> Result<(), String> {
    use std::io::{Read, Seek, SeekFrom};
    let mut file = std::fs::File::open(binary).map_err(|_| MISSING.to_string())?;
    let mut header = [0u8; 64];
    file.read_exact(&mut header).map_err(|_| NOT_VST3.to_string())?;
    if &header[..2] != b"MZ" {
        return Err(NOT_VST3.into());
    }
    let offset = u32::from_le_bytes([header[60], header[61], header[62], header[63]]);
    let mut pe = [0u8; 6];
    file.seek(SeekFrom::Start(offset as u64)).and_then(|_| file.read_exact(&mut pe)).map_err(|_| NOT_VST3.to_string())?;
    if &pe[..4] != b"PE\0\0" {
        return Err(NOT_VST3.into());
    }
    match u16::from_le_bytes([pe[4], pe[5]]) {
        0x8664 => Ok(()),
        0x014c => Err(NOT_64_BIT.into()),
        0xaa64 | 0x01c4 => Err(ARM.into()),
        _ => Err(NOT_VST3.into()),
    }
}

/// The folders VST3 plugins are installed in: `%CommonProgramFiles%\VST3` for everyone, and
/// `%LOCALAPPDATA%\Programs\Common\VST3` for one user.
pub fn standard_folders() -> Vec<PathBuf> {
    let mut folders = Vec::new();
    if let Some(common) = std::env::var_os("CommonProgramFiles") {
        folders.push(PathBuf::from(common).join("VST3"));
    }
    if let Some(local) = std::env::var_os("LOCALAPPDATA") {
        folders.push(PathBuf::from(local).join("Programs").join("Common").join("VST3"));
    }
    folders
}

/// The plugins installed in [`standard_folders`] (and in vendor folders inside them), by name.
/// Only names are read: nothing is loaded.
pub fn installed() -> Vec<PathBuf> {
    let mut found = Vec::new();
    for folder in standard_folders() {
        scan(&folder, 0, &mut found);
    }
    found.sort_by_key(|path| display_name(path).to_lowercase());
    found.dedup();
    found
}

fn scan(folder: &Path, depth: usize, found: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(folder) else { return };
    for path in entries.flatten().map(|entry| entry.path()) {
        if is_vst3(&path) {
            found.push(path);
        } else if path.is_dir() && depth < 2 {
            scan(&path, depth + 1, found);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("qust-vst-bundle-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    // A PE header for `machine`.
    fn binary(path: &Path, machine: u16) {
        let mut data = vec![0u8; 128];
        data[..2].copy_from_slice(b"MZ");
        data[60..64].copy_from_slice(&64u32.to_le_bytes());
        data[64..68].copy_from_slice(b"PE\0\0");
        data[68..70].copy_from_slice(&machine.to_le_bytes());
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, data).unwrap();
    }

    #[test]
    fn names_drop_the_extension() {
        assert_eq!(display_name(Path::new(r"C:\VST3\Khot Broadcast.vst3")), "Khot Broadcast");
        assert_eq!(display_name(Path::new("EQ.VST3")), "EQ");
        assert_eq!(display_name(Path::new("plain")), "plain");
    }

    #[test]
    fn a_bundle_its_binary_and_a_single_file_all_locate() {
        let root = dir("locate");
        let bundle = root.join("Gain.vst3");
        let inside = bundle.join("Contents").join("x86_64-win").join("Gain.vst3");
        binary(&inside, 0x8664);
        let expected = Located { bundle: bundle.clone(), binary: inside.clone() };
        assert_eq!(locate(&bundle), Ok(expected.clone()));
        assert_eq!(locate(&inside), Ok(expected));
        assert_eq!(check_binary(&inside), Ok(()));

        let single = root.join("Old.vst3");
        binary(&single, 0x8664);
        assert_eq!(locate(&single), Ok(Located { bundle: single.clone(), binary: single }));
        assert_eq!(locate(&root.join("Gone.vst3")), Err(MISSING.into()));
    }

    #[test]
    fn thirty_two_bit_and_arm_plugins_are_named() {
        let root = dir("bits");
        let old = root.join("Old.vst3");
        binary(&old.join("Contents").join("x86-win").join("Old.vst3"), 0x014c);
        assert_eq!(locate(&old), Err(NOT_64_BIT.into()));
        let file = root.join("File32.vst3");
        binary(&file, 0x014c);
        assert_eq!(check_binary(&file), Err(NOT_64_BIT.into()));
        let arm = root.join("Arm.vst3");
        binary(&arm, 0xaa64);
        assert_eq!(check_binary(&arm), Err(ARM.into()));
        let text = root.join("Text.vst3");
        std::fs::write(&text, "hello").unwrap();
        assert_eq!(check_binary(&text), Err(NOT_VST3.into()));
    }
}
