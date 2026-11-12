// Recording a global shortcut in Settings: the old page's keydown handler, which turned a
// `KeyboardEvent` into the stored accelerator. The rules (`record`) take the key's
// `KeyboardEvent.code` and the modifier flags; where those come from:
//
// - Windows, exactly: `code_from_scan_code`, from the WM_KEYDOWN / WM_SYSKEYDOWN scan code, the
//   way Chromium computes `code` (physical position, not the layout). `KeyCapture` (in `window`)
//   reads it straight from the Settings window's messages while recording, which also sees the
//   keys GPUI drops (media keys, Pause, Print Screen, Num Lock, Scroll Lock) and tells the numpad
//   from the digit row.
// - From a GPUI key event (`code_from_gpui`), where only GPUI's event is at hand (macOS, or a test
//   without a window). GPUI reports the layout's character, so this is the US-layout code of that
//   character: the same as `code` on a US layout, and not for keys GPUI doesn't report.

// What a key press does while a shortcut is being recorded.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Recorded {
    // Swallowed and nothing changes (a modifier alone, or a plain key without modifiers).
    Ignore,
    // Esc: stop recording, keep the shortcut.
    Cancel,
    // Backspace or Delete: reset to the default (`set_shortcut` with an empty accelerator).
    Reset,
    // The new accelerator, to give `set_shortcut`.
    Set(String),
}

const MODIFIER_CODES: [&str; 8] = ["ControlLeft", "ControlRight", "AltLeft", "AltRight", "ShiftLeft", "ShiftRight", "MetaLeft", "MetaRight"];

// The page's keydown handler while recording.
pub fn record(code: &str, ctrl: bool, alt: bool, shift: bool, meta: bool) -> Recorded {
    if MODIFIER_CODES.contains(&code) {
        return Recorded::Ignore;
    }
    if code == "Escape" {
        return Recorded::Cancel;
    }
    if code == "Backspace" || code == "Delete" {
        return Recorded::Reset;
    }
    let mods = crate::accelerator::Mods { ctrl, alt, shift, meta };
    // /^F\d+$/ or a Media key may stand alone.
    let standalone = code.strip_prefix('F').is_some_and(|rest| !rest.is_empty() && rest.bytes().all(|b| b.is_ascii_digit())) || code.starts_with("Media");
    if mods.is_empty() && !standalone {
        return Recorded::Ignore;
    }
    Recorded::Set(crate::accelerator::format(mods, code))
}

// Chromium's `code` for a Windows scan code (bits 16–23 of the key message's lParam) and its
// extended flag (bit 24). From Chromium's dom_code_data.inc, Windows column. Codes Chromium
// doesn't know give None (`code` was then "" and the page ignored the key without modifiers).
pub fn code_from_scan_code(scan_code: u32, extended: bool) -> Option<&'static str> {
    let code = if extended { 0xE000 | scan_code } else { scan_code };
    Some(match code {
        0x0001 => "Escape",
        0x0002 => "Digit1",
        0x0003 => "Digit2",
        0x0004 => "Digit3",
        0x0005 => "Digit4",
        0x0006 => "Digit5",
        0x0007 => "Digit6",
        0x0008 => "Digit7",
        0x0009 => "Digit8",
        0x000A => "Digit9",
        0x000B => "Digit0",
        0x000C => "Minus",
        0x000D => "Equal",
        0x000E => "Backspace",
        0x000F => "Tab",
        0x0010 => "KeyQ",
        0x0011 => "KeyW",
        0x0012 => "KeyE",
        0x0013 => "KeyR",
        0x0014 => "KeyT",
        0x0015 => "KeyY",
        0x0016 => "KeyU",
        0x0017 => "KeyI",
        0x0018 => "KeyO",
        0x0019 => "KeyP",
        0x001A => "BracketLeft",
        0x001B => "BracketRight",
        0x001C => "Enter",
        0x001D => "ControlLeft",
        0x001E => "KeyA",
        0x001F => "KeyS",
        0x0020 => "KeyD",
        0x0021 => "KeyF",
        0x0022 => "KeyG",
        0x0023 => "KeyH",
        0x0024 => "KeyJ",
        0x0025 => "KeyK",
        0x0026 => "KeyL",
        0x0027 => "Semicolon",
        0x0028 => "Quote",
        0x0029 => "Backquote",
        0x002A => "ShiftLeft",
        0x002B => "Backslash",
        0x002C => "KeyZ",
        0x002D => "KeyX",
        0x002E => "KeyC",
        0x002F => "KeyV",
        0x0030 => "KeyB",
        0x0031 => "KeyN",
        0x0032 => "KeyM",
        0x0033 => "Comma",
        0x0034 => "Period",
        0x0035 => "Slash",
        0x0036 => "ShiftRight",
        0x0037 => "NumpadMultiply",
        0x0038 => "AltLeft",
        0x0039 => "Space",
        0x003A => "CapsLock",
        0x003B => "F1",
        0x003C => "F2",
        0x003D => "F3",
        0x003E => "F4",
        0x003F => "F5",
        0x0040 => "F6",
        0x0041 => "F7",
        0x0042 => "F8",
        0x0043 => "F9",
        0x0044 => "F10",
        0x0045 => "Pause",
        0x0046 => "ScrollLock",
        0x0047 => "Numpad7",
        0x0048 => "Numpad8",
        0x0049 => "Numpad9",
        0x004A => "NumpadSubtract",
        0x004B => "Numpad4",
        0x004C => "Numpad5",
        0x004D => "Numpad6",
        0x004E => "NumpadAdd",
        0x004F => "Numpad1",
        0x0050 => "Numpad2",
        0x0051 => "Numpad3",
        0x0052 => "Numpad0",
        0x0053 => "NumpadDecimal",
        0x0056 => "IntlBackslash",
        0x0057 => "F11",
        0x0058 => "F12",
        0x0059 => "NumpadEqual",
        0x0064 => "F13",
        0x0065 => "F14",
        0x0066 => "F15",
        0x0067 => "F16",
        0x0068 => "F17",
        0x0069 => "F18",
        0x006A => "F19",
        0x006B => "F20",
        0x006C => "F21",
        0x006D => "F22",
        0x006E => "F23",
        0x0070 => "KanaMode",
        0x0073 => "IntlRo",
        0x0076 => "F24",
        0x0079 => "Convert",
        0x007B => "NonConvert",
        0x007D => "IntlYen",
        0x007E => "NumpadComma",
        0xE010 => "MediaTrackPrevious",
        0xE019 => "MediaTrackNext",
        0xE01C => "NumpadEnter",
        0xE01D => "ControlRight",
        0xE020 => "AudioVolumeMute",
        0xE021 => "LaunchApp2",
        0xE022 => "MediaPlayPause",
        0xE024 => "MediaStop",
        0xE02E => "AudioVolumeDown",
        0xE030 => "AudioVolumeUp",
        0xE032 => "BrowserHome",
        0xE035 => "NumpadDivide",
        0xE037 => "PrintScreen",
        0xE038 => "AltRight",
        0xE045 => "NumLock",
        0xE046 => "Pause",
        0xE047 => "Home",
        0xE048 => "ArrowUp",
        0xE049 => "PageUp",
        0xE04B => "ArrowLeft",
        0xE04D => "ArrowRight",
        0xE04F => "End",
        0xE050 => "ArrowDown",
        0xE051 => "PageDown",
        0xE052 => "Insert",
        0xE053 => "Delete",
        0xE05B => "MetaLeft",
        0xE05C => "MetaRight",
        0xE05D => "ContextMenu",
        0xE05E => "Power",
        0xE05F => "Sleep",
        0xE063 => "WakeUp",
        0xE065 => "BrowserSearch",
        0xE066 => "BrowserFavorites",
        0xE067 => "BrowserRefresh",
        0xE068 => "BrowserStop",
        0xE069 => "BrowserForward",
        0xE06A => "BrowserBack",
        0xE06B => "LaunchApp1",
        0xE06C => "LaunchMail",
        0xE06D => "MediaSelect",
        _ => return None,
    })
}

// Some keyboards send media keys with no scan code (only the virtual key): their code then comes
// from the virtual key, so they can still be recorded.
pub fn code_from_media_vk(vk: u16) -> Option<&'static str> {
    Some(match vk {
        0xB3 => "MediaPlayPause",
        0xB2 => "MediaStop",
        0xB0 => "MediaTrackNext",
        0xB1 => "MediaTrackPrevious",
        0xAD => "AudioVolumeMute",
        0xAE => "AudioVolumeDown",
        0xAF => "AudioVolumeUp",
        0xFA => "MediaPlay",
        _ => return None,
    })
}

// The `code` for a GPUI key event's `key` (and whether Shift was pressed, as GPUI reports it): see
// the module comment. On Windows GPUI turns Shift+1 into the key "!" with Shift off; that comes
// back as Digit1 with Shift on. None for keys with no code here (they are then ignored).
pub fn code_from_gpui(key: &str, shift: bool) -> Option<(&'static str, bool)> {
    let named = match key {
        "escape" => Some("Escape"),
        "backspace" => Some("Backspace"),
        "delete" => Some("Delete"),
        "enter" => Some("Enter"),
        "tab" => Some("Tab"),
        "space" => Some("Space"),
        "insert" => Some("Insert"),
        "home" => Some("Home"),
        "end" => Some("End"),
        "pageup" => Some("PageUp"),
        "pagedown" => Some("PageDown"),
        "up" => Some("ArrowUp"),
        "down" => Some("ArrowDown"),
        "left" => Some("ArrowLeft"),
        "right" => Some("ArrowRight"),
        "menu" => Some("ContextMenu"),
        "-" => Some("Minus"),
        "=" => Some("Equal"),
        "[" => Some("BracketLeft"),
        "]" => Some("BracketRight"),
        "\\" => Some("Backslash"),
        ";" => Some("Semicolon"),
        "'" => Some("Quote"),
        "," => Some("Comma"),
        "." => Some("Period"),
        "/" => Some("Slash"),
        "`" => Some("Backquote"),
        _ => None,
    };
    if let Some(code) = named {
        return Some((code, shift));
    }
    // Shifted characters of the US layout.
    let shifted = match key {
        "~" => Some("Backquote"),
        "!" => Some("Digit1"),
        "@" => Some("Digit2"),
        "#" => Some("Digit3"),
        "$" => Some("Digit4"),
        "%" => Some("Digit5"),
        "^" => Some("Digit6"),
        "&" => Some("Digit7"),
        "*" => Some("Digit8"),
        "(" => Some("Digit9"),
        ")" => Some("Digit0"),
        "_" => Some("Minus"),
        "+" => Some("Equal"),
        "{" => Some("BracketLeft"),
        "}" => Some("BracketRight"),
        "|" => Some("Backslash"),
        ":" => Some("Semicolon"),
        "\"" => Some("Quote"),
        "<" => Some("Comma"),
        ">" => Some("Period"),
        "?" => Some("Slash"),
        _ => None,
    };
    if let Some(code) = shifted {
        return Some((code, true));
    }
    const LETTERS: [&str; 26] = [
        "KeyA", "KeyB", "KeyC", "KeyD", "KeyE", "KeyF", "KeyG", "KeyH", "KeyI", "KeyJ", "KeyK", "KeyL", "KeyM", "KeyN", "KeyO", "KeyP", "KeyQ", "KeyR", "KeyS", "KeyT", "KeyU", "KeyV", "KeyW", "KeyX", "KeyY", "KeyZ",
    ];
    const DIGITS: [&str; 10] = ["Digit0", "Digit1", "Digit2", "Digit3", "Digit4", "Digit5", "Digit6", "Digit7", "Digit8", "Digit9"];
    const FUNCTION: [&str; 24] = ["F1", "F2", "F3", "F4", "F5", "F6", "F7", "F8", "F9", "F10", "F11", "F12", "F13", "F14", "F15", "F16", "F17", "F18", "F19", "F20", "F21", "F22", "F23", "F24"];
    let mut chars = key.chars();
    if let (Some(c), None) = (chars.next(), chars.next()) {
        if c.is_ascii_lowercase() || c.is_ascii_uppercase() {
            return Some((LETTERS[(c.to_ascii_lowercase() as u8 - b'a') as usize], shift));
        }
        if c.is_ascii_digit() {
            return Some((DIGITS[(c as u8 - b'0') as usize], shift));
        }
        return None;
    }
    if let Some(number) = key.strip_prefix('f').and_then(|rest| rest.parse::<usize>().ok()) {
        if (1..=24).contains(&number) && key == format!("f{number}") {
            return Some((FUNCTION[number - 1], shift));
        }
    }
    None
}

// A GPUI key down while recording: `code_from_gpui`, then the rules. A key GPUI reports with no
// code is ignored. (GPUI reports modifier presses as separate events, not key downs.)
pub fn record_gpui(key: &str, ctrl: bool, alt: bool, shift: bool, meta: bool) -> Recorded {
    match code_from_gpui(key, shift) {
        Some((code, shift)) => record(code, ctrl, alt, shift, meta),
        None => Recorded::Ignore,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::accelerator::Accelerator;

    #[test]
    fn rules_like_the_page() {
        assert_eq!(record("ControlLeft", true, false, false, false), Recorded::Ignore);
        assert_eq!(record("ShiftRight", false, false, true, false), Recorded::Ignore);
        assert_eq!(record("MetaLeft", false, false, false, true), Recorded::Ignore);
        assert_eq!(record("Escape", true, true, false, false), Recorded::Cancel);
        assert_eq!(record("Backspace", false, false, false, false), Recorded::Reset);
        assert_eq!(record("Delete", true, false, false, false), Recorded::Reset);
        assert_eq!(record("KeyH", false, false, false, false), Recorded::Ignore);
        assert_eq!(record("KeyH", true, true, false, false), Recorded::Set("Ctrl+Alt+KeyH".into()));
        assert_eq!(record("Insert", true, true, false, false), Recorded::Set("Ctrl+Alt+Insert".into()));
        assert_eq!(record("KeyP", true, true, true, true), Recorded::Set("Ctrl+Alt+Shift+Super+KeyP".into()));
        assert_eq!(record("F9", false, false, false, false), Recorded::Set("F9".into()));
        assert_eq!(record("F24", false, false, false, false), Recorded::Set("F24".into()));
        assert_eq!(record("MediaPlayPause", false, false, false, false), Recorded::Set("MediaPlayPause".into()));
        // "F" alone isn't a function key; an empty code (unknown key) needs modifiers.
        assert_eq!(record("F", false, false, false, false), Recorded::Ignore);
        assert_eq!(record("", false, false, false, false), Recorded::Ignore);
        assert_eq!(record("", true, false, false, false), Recorded::Set("Ctrl+".into()));
    }

    #[test]
    fn recorded_accelerators_are_stored_form() {
        for (code, ctrl, alt, shift, meta) in [("KeyH", true, true, false, false), ("Numpad5", true, false, true, false), ("F12", false, false, false, false), ("ArrowUp", false, true, false, true)] {
            let Recorded::Set(text) = record(code, ctrl, alt, shift, meta) else { panic!() };
            assert_eq!(Accelerator::parse(&text).map(|a| a.to_stored()), Ok(text));
        }
    }

    #[test]
    fn scan_codes_like_chromium() {
        assert_eq!(code_from_scan_code(0x23, false), Some("KeyH"));
        assert_eq!(code_from_scan_code(0x52, true), Some("Insert"));
        assert_eq!(code_from_scan_code(0x52, false), Some("Numpad0"));
        assert_eq!(code_from_scan_code(0x1C, true), Some("NumpadEnter"));
        assert_eq!(code_from_scan_code(0x1C, false), Some("Enter"));
        assert_eq!(code_from_scan_code(0x45, false), Some("Pause"));
        assert_eq!(code_from_scan_code(0x45, true), Some("NumLock"));
        assert_eq!(code_from_scan_code(0x22, true), Some("MediaPlayPause"));
        assert_eq!(code_from_scan_code(0x1D, false), Some("ControlLeft"));
        assert_eq!(code_from_scan_code(0x38, true), Some("AltRight"));
        assert_eq!(code_from_scan_code(0x5B, true), Some("MetaLeft"));
        assert_eq!(code_from_scan_code(0x01, false), Some("Escape"));
        assert_eq!(code_from_scan_code(0x0E, false), Some("Backspace"));
        assert_eq!(code_from_scan_code(0x53, true), Some("Delete"));
        assert_eq!(code_from_scan_code(0x7F, false), None);
    }

    #[test]
    fn gpui_keys() {
        assert_eq!(record_gpui("h", true, true, false, false), Recorded::Set("Ctrl+Alt+KeyH".into()));
        assert_eq!(record_gpui("insert", true, true, false, false), Recorded::Set("Ctrl+Alt+Insert".into()));
        assert_eq!(record_gpui("f9", false, false, false, false), Recorded::Set("F9".into()));
        assert_eq!(record_gpui("escape", false, false, false, false), Recorded::Cancel);
        assert_eq!(record_gpui("backspace", true, false, false, false), Recorded::Reset);
        assert_eq!(record_gpui("delete", false, false, false, false), Recorded::Reset);
        assert_eq!(record_gpui("!", true, false, false, false), Recorded::Set("Ctrl+Shift+Digit1".into()));
        assert_eq!(record_gpui("up", false, true, false, false), Recorded::Set("Alt+ArrowUp".into()));
        assert_eq!(record_gpui("a", false, false, false, false), Recorded::Ignore);
        assert_eq!(record_gpui("é", true, false, false, false), Recorded::Ignore);
        assert_eq!(record_gpui("f25", true, false, false, false), Recorded::Ignore);
    }
}
