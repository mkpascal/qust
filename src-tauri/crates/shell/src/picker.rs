// "Add file": the system file picker, as tauri-plugin-dialog (rfd 0.17) showed it: title "Add to
// playlist", the filters "Audio files and playlists" (the audio and list extensions) and "All
// files", several files at once, owned by the player window (which it disables while open).
//
// It blocks until closed, in a thread of its own with its own COM apartment (as GPUI's own
// pickers run), so the UI keeps running: call `pick_files` from a background thread or task.

use qust_player::playlist::{AUDIO_EXTENSIONS, LIST_EXTENSIONS};
use std::path::PathBuf;

pub const TITLE: &str = "Add to playlist";

// (name, pattern) as rfd built them: "*.mp3;*.m4a;…", and "*" as "*.*".
pub fn filters() -> Vec<(String, String)> {
    let all: Vec<&str> = AUDIO_EXTENSIONS.iter().chain(LIST_EXTENSIONS).copied().collect();
    let pattern = |extensions: &[&str]| extensions.iter().map(|extension| format!("*.{extension}")).collect::<Vec<_>>().join(";");
    vec![("Audio files and playlists".to_string(), pattern(&all)), ("All files".to_string(), pattern(&["*"]))]
}

#[cfg(windows)]
pub fn pick_files(owner: Option<isize>) -> Vec<PathBuf> {
    // A thread of its own: COM, and the dialog's modal loop, stay off the caller's thread.
    std::thread::spawn(move || imp::run(owner).unwrap_or_default()).join().unwrap_or_default()
}

#[cfg(windows)]
mod imp {
    use super::*;
    use crate::win;
    use windows::core::PCWSTR;
    use windows::Win32::System::Com::{CoCreateInstance, CoInitializeEx, CoTaskMemFree, CoUninitialize, CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED, COINIT_DISABLE_OLE1DDE};
    use windows::Win32::UI::Shell::Common::COMDLG_FILTERSPEC;
    use windows::Win32::UI::Shell::{FileOpenDialog, IFileOpenDialog, FOS_ALLOWMULTISELECT, SIGDN_FILESYSPATH};

    pub fn run(owner: Option<isize>) -> windows::core::Result<Vec<PathBuf>> {
        unsafe {
            CoInitializeEx(None, COINIT_APARTMENTTHREADED | COINIT_DISABLE_OLE1DDE).ok()?;
            let result = show(owner);
            CoUninitialize();
            result
        }
    }

    unsafe fn show(owner: Option<isize>) -> windows::core::Result<Vec<PathBuf>> {
        let dialog: IFileOpenDialog = CoCreateInstance(&FileOpenDialog, None, CLSCTX_INPROC_SERVER)?;
        let filters = filters();
        let names: Vec<Vec<u16>> = filters.iter().map(|(name, _)| win::wide(name)).collect();
        let patterns: Vec<Vec<u16>> = filters.iter().map(|(_, pattern)| win::wide(pattern)).collect();
        let specs: Vec<COMDLG_FILTERSPEC> = names.iter().zip(&patterns).map(|(name, pattern)| COMDLG_FILTERSPEC { pszName: PCWSTR(name.as_ptr()), pszSpec: PCWSTR(pattern.as_ptr()) }).collect();
        dialog.SetFileTypes(&specs)?;
        let title = win::wide(TITLE);
        dialog.SetTitle(PCWSTR(title.as_ptr()))?;
        // rfd replaced the default options with this alone.
        dialog.SetOptions(FOS_ALLOWMULTISELECT)?;
        // Cancelled: an error (ERROR_CANCELLED), which is no files.
        if dialog.Show(owner.map(win::hwnd)).is_err() {
            return Ok(Vec::new());
        }
        let items = dialog.GetResults()?;
        let mut paths = Vec::new();
        for index in 0..items.GetCount()? {
            let item = items.GetItemAt(index)?;
            let name = item.GetDisplayName(SIGDN_FILESYSPATH)?;
            paths.push(PathBuf::from(name.to_string().unwrap_or_default()));
            CoTaskMemFree(Some(name.0 as *const _));
        }
        Ok(paths)
    }
}

// macOS: GPUI's `prompt_for_paths` (NSOpenPanel) is used instead; it has no filters or title.
#[cfg(not(windows))]
pub fn pick_files(_owner: Option<isize>) -> Vec<PathBuf> {
    Vec::new()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn filters_like_rfd() {
        let filters = filters();
        assert_eq!(filters[0].0, "Audio files and playlists");
        assert_eq!(filters[0].1, "*.mp3;*.m4a;*.aac;*.mp4;*.ogg;*.oga;*.opus;*.flac;*.wav;*.webm;*.weba;*.m3u;*.m3u8;*.pls");
        assert_eq!(filters[1], ("All files".to_string(), "*.*".to_string()));
    }
}
