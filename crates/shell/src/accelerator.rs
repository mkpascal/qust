// Global shortcut accelerators as Qust stores them: "Ctrl+Alt+Insert", "Ctrl+Alt+KeyH" (the
// modifiers, then the key's `KeyboardEvent.code`), read exactly the way tauri-plugin-global-shortcut
// read them (global-hotkey 0.8's parser), so every saved shortcut keeps working. Also the labels the
// UI shows (the old page's `shortcutKeys`) and the accelerator text of native menus (what muda 0.20
// displayed for those labels).

use std::fmt;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct Mods {
    pub ctrl: bool,
    pub alt: bool,
    pub shift: bool,
    // The Windows key / Command.
    pub meta: bool,
}

impl Mods {
    pub fn is_empty(&self) -> bool {
        !(self.ctrl || self.alt || self.shift || self.meta)
    }
}

// A parsed accelerator: modifiers and one key, the key named by its W3C `code` ("KeyH").
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Accelerator {
    pub mods: Mods,
    pub code: &'static str,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ParseError {
    UnsupportedKey(String),
    EmptyToken(String),
    InvalidFormat(String),
}

impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ParseError::UnsupportedKey(key) => write!(f, "unsupported key \"{key}\""),
            ParseError::EmptyToken(text) => write!(f, "empty token in \"{text}\""),
            ParseError::InvalidFormat(text) => write!(f, "invalid shortcut \"{text}\""),
        }
    }
}

impl std::error::Error for ParseError {}

// The keys a global shortcut can use: global-hotkey's `parse_key`, with each key's aliases, its
// Windows virtual-key code (global-hotkey's `key_to_vk`, quirks included) and its macOS virtual
// key code (kVK_*, for RegisterEventHotKey).
struct KeyInfo {
    code: &'static str,
    aliases: &'static [&'static str],
    vk: u16,
    mac: Option<u16>,
}

macro_rules! keys {
    ($( $code:literal [$($alias:literal),*] $vk:literal $mac:expr ;)*) => {
        &[ $( KeyInfo { code: $code, aliases: &[$($alias),*], vk: $vk, mac: $mac }, )* ]
    };
}

static KEYS: &[KeyInfo] = keys! {
    "Backquote" ["`"] 0xC0 Some(0x32);
    "Backslash" ["\\"] 0xDC Some(0x2A);
    "BracketLeft" ["["] 0xDB Some(0x21);
    "BracketRight" ["]"] 0xDD Some(0x1E);
    "Pause" ["PAUSEBREAK"] 0x13 None;
    "Comma" [","] 0xBC Some(0x2B);
    "Digit0" ["0"] 0x30 Some(0x1D);
    "Digit1" ["1"] 0x31 Some(0x12);
    "Digit2" ["2"] 0x32 Some(0x13);
    "Digit3" ["3"] 0x33 Some(0x14);
    "Digit4" ["4"] 0x34 Some(0x15);
    "Digit5" ["5"] 0x35 Some(0x17);
    "Digit6" ["6"] 0x36 Some(0x16);
    "Digit7" ["7"] 0x37 Some(0x1A);
    "Digit8" ["8"] 0x38 Some(0x1C);
    "Digit9" ["9"] 0x39 Some(0x19);
    "Equal" ["="] 0xBB Some(0x18);
    "KeyA" ["A"] 0x41 Some(0x00);
    "KeyB" ["B"] 0x42 Some(0x0B);
    "KeyC" ["C"] 0x43 Some(0x08);
    "KeyD" ["D"] 0x44 Some(0x02);
    "KeyE" ["E"] 0x45 Some(0x0E);
    "KeyF" ["F"] 0x46 Some(0x03);
    "KeyG" ["G"] 0x47 Some(0x05);
    "KeyH" ["H"] 0x48 Some(0x04);
    "KeyI" ["I"] 0x49 Some(0x22);
    "KeyJ" ["J"] 0x4A Some(0x26);
    "KeyK" ["K"] 0x4B Some(0x28);
    "KeyL" ["L"] 0x4C Some(0x25);
    "KeyM" ["M"] 0x4D Some(0x2E);
    "KeyN" ["N"] 0x4E Some(0x2D);
    "KeyO" ["O"] 0x4F Some(0x1F);
    "KeyP" ["P"] 0x50 Some(0x23);
    "KeyQ" ["Q"] 0x51 Some(0x0C);
    "KeyR" ["R"] 0x52 Some(0x0F);
    "KeyS" ["S"] 0x53 Some(0x01);
    "KeyT" ["T"] 0x54 Some(0x11);
    "KeyU" ["U"] 0x55 Some(0x20);
    "KeyV" ["V"] 0x56 Some(0x09);
    "KeyW" ["W"] 0x57 Some(0x0D);
    "KeyX" ["X"] 0x58 Some(0x07);
    "KeyY" ["Y"] 0x59 Some(0x10);
    "KeyZ" ["Z"] 0x5A Some(0x06);
    "Minus" ["-"] 0xBD Some(0x1B);
    "Period" ["."] 0xBE Some(0x2F);
    "Quote" ["'"] 0xDE Some(0x27);
    "Semicolon" [";"] 0xBA Some(0x29);
    "Slash" ["/"] 0xBF Some(0x2C);
    "Backspace" [] 0x08 Some(0x33);
    "CapsLock" [] 0x14 Some(0x39);
    "Enter" [] 0x0D Some(0x24);
    "Space" [] 0x20 Some(0x31);
    "Tab" [] 0x09 Some(0x30);
    "Delete" [] 0x2E Some(0x75);
    "End" [] 0x23 Some(0x77);
    "Home" [] 0x24 Some(0x73);
    "Insert" [] 0x2D Some(0x72);
    "PageDown" [] 0x22 Some(0x79);
    "PageUp" [] 0x21 Some(0x74);
    "PrintScreen" [] 0x2C None;
    "ScrollLock" [] 0x91 None;
    "ArrowDown" ["DOWN"] 0x28 Some(0x7D);
    "ArrowLeft" ["LEFT"] 0x25 Some(0x7B);
    "ArrowRight" ["RIGHT"] 0x27 Some(0x7C);
    "ArrowUp" ["UP"] 0x26 Some(0x7E);
    "NumLock" [] 0x90 Some(0x47);
    "Numpad0" ["NUM0"] 0x60 Some(0x52);
    "Numpad1" ["NUM1"] 0x61 Some(0x53);
    "Numpad2" ["NUM2"] 0x62 Some(0x54);
    "Numpad3" ["NUM3"] 0x63 Some(0x55);
    "Numpad4" ["NUM4"] 0x64 Some(0x56);
    "Numpad5" ["NUM5"] 0x65 Some(0x57);
    "Numpad6" ["NUM6"] 0x66 Some(0x58);
    "Numpad7" ["NUM7"] 0x67 Some(0x59);
    "Numpad8" ["NUM8"] 0x68 Some(0x5B);
    "Numpad9" ["NUM9"] 0x69 Some(0x5C);
    "NumpadAdd" ["NUMADD", "NUMPADPLUS", "NUMPLUS"] 0x6B Some(0x45);
    "NumpadDecimal" ["NUMDECIMAL"] 0x6E Some(0x41);
    "NumpadDivide" ["NUMDIVIDE"] 0x6F Some(0x4B);
    // global-hotkey registers NumpadEnter as Enter.
    "NumpadEnter" ["NUMENTER"] 0x0D Some(0x4C);
    // global-hotkey maps NumpadEqual to VK_E, a bug kept so the same key registers as before.
    "NumpadEqual" ["NUMEQUAL"] 0x45 Some(0x51);
    "NumpadMultiply" ["NUMMULTIPLY"] 0x6A Some(0x43);
    "NumpadSubtract" ["NUMSUBTRACT"] 0x6D Some(0x4E);
    "Escape" ["ESC"] 0x1B Some(0x35);
    "F1" [] 0x70 Some(0x7A);
    "F2" [] 0x71 Some(0x78);
    "F3" [] 0x72 Some(0x63);
    "F4" [] 0x73 Some(0x76);
    "F5" [] 0x74 Some(0x60);
    "F6" [] 0x75 Some(0x61);
    "F7" [] 0x76 Some(0x62);
    "F8" [] 0x77 Some(0x64);
    "F9" [] 0x78 Some(0x65);
    "F10" [] 0x79 Some(0x6D);
    "F11" [] 0x7A Some(0x67);
    "F12" [] 0x7B Some(0x6F);
    "AudioVolumeDown" ["VOLUMEDOWN"] 0xAE Some(0x49);
    "AudioVolumeUp" ["VOLUMEUP"] 0xAF Some(0x48);
    "AudioVolumeMute" ["VOLUMEMUTE"] 0xAD Some(0x4A);
    "MediaPlay" [] 0xFA None;
    // global-hotkey registers MediaPause as Pause.
    "MediaPause" [] 0x13 None;
    "MediaPlayPause" [] 0xB3 None;
    "MediaStop" [] 0xB2 None;
    "MediaTrackNext" [] 0xB0 None;
    "MediaTrackPrevious" ["MEDIATRACKPREV"] 0xB1 None;
    "F13" [] 0x7C Some(0x69);
    "F14" [] 0x7D Some(0x6B);
    "F15" [] 0x7E Some(0x71);
    "F16" [] 0x7F Some(0x6A);
    "F17" [] 0x80 Some(0x40);
    "F18" [] 0x81 Some(0x4F);
    "F19" [] 0x82 Some(0x50);
    "F20" [] 0x83 Some(0x5A);
    "F21" [] 0x84 None;
    "F22" [] 0x85 None;
    "F23" [] 0x86 None;
    "F24" [] 0x87 None;
};

fn key_info(code: &str) -> Option<&'static KeyInfo> {
    KEYS.iter().find(|key| key.code == code)
}

// global-hotkey's parse_key: the code or an alias, in any case.
fn parse_key(token: &str) -> Result<&'static str, ParseError> {
    let upper = token.to_uppercase();
    KEYS.iter()
        .find(|key| key.code.to_uppercase() == upper || key.aliases.contains(&upper.as_str()))
        .map(|key| key.code)
        .ok_or_else(|| ParseError::UnsupportedKey(token.to_string()))
}

impl Accelerator {
    // global-hotkey 0.8's parse_hotkey: tokens split on '+', modifiers first and one key last.
    // A single token is a key alone (not trimmed); with several, each is trimmed and none may be
    // empty.
    pub fn parse(text: &str) -> Result<Accelerator, ParseError> {
        let tokens: Vec<&str> = text.split('+').collect();
        let mut mods = Mods::default();
        let mut code = None;
        if tokens.len() == 1 {
            code = Some(parse_key(tokens[0])?);
        } else {
            for raw in tokens {
                let token = raw.trim();
                if token.is_empty() {
                    return Err(ParseError::EmptyToken(text.to_string()));
                }
                if code.is_some() {
                    return Err(ParseError::InvalidFormat(text.to_string()));
                }
                match token.to_uppercase().as_str() {
                    "OPTION" | "ALT" => mods.alt = true,
                    "CONTROL" | "CTRL" => mods.ctrl = true,
                    "COMMAND" | "CMD" | "SUPER" => mods.meta = true,
                    "SHIFT" => mods.shift = true,
                    "COMMANDORCONTROL" | "COMMANDORCTRL" | "CMDORCTRL" | "CMDORCONTROL" => {
                        if cfg!(target_os = "macos") {
                            mods.meta = true
                        } else {
                            mods.ctrl = true
                        }
                    }
                    _ => code = Some(parse_key(token)?),
                }
            }
        }
        let code = code.ok_or_else(|| ParseError::InvalidFormat(text.to_string()))?;
        Ok(Accelerator { mods, code })
    }

    // The stored form, as the Settings recorder writes it: Ctrl, Alt, Shift, Super (those present,
    // in that order), then the code.
    pub fn to_stored(&self) -> String {
        format(self.mods, self.code)
    }

    // The Windows virtual-key code RegisterHotKey takes.
    pub fn vk(&self) -> u16 {
        key_info(self.code).map_or(0, |key| key.vk)
    }

    // The macOS virtual key code (kVK_*), when the key has one.
    pub fn mac_key_code(&self) -> Option<u16> {
        key_info(self.code).and_then(|key| key.mac)
    }
}

// "Ctrl+Alt+Shift+Super+<code>" with the modifiers present.
pub fn format(mods: Mods, code: &str) -> String {
    let mut parts: Vec<&str> = Vec::new();
    if mods.ctrl {
        parts.push("Ctrl");
    }
    if mods.alt {
        parts.push("Alt");
    }
    if mods.shift {
        parts.push("Shift");
    }
    if mods.meta {
        parts.push("Super");
    }
    parts.push(code);
    parts.join("+")
}

// The labels the UI shows for a stored accelerator, one per key chip: the page's shortcutKeys().
// "Ctrl+Alt+KeyH" → ["Ctrl", "Alt", "H"]; Super → "Win", KeyX → "X", DigitN → "N", Numpad… →
// "Num …", Arrow… → Up / Down / Left / Right, Backquote → "`", anything else as written.
pub fn keys(accelerator: &str) -> Vec<String> {
    accelerator
        .split('+')
        .map(|part| {
            let named = match part {
                "Super" => Some("Win"),
                "ArrowUp" => Some("Up"),
                "ArrowDown" => Some("Down"),
                "ArrowLeft" => Some("Left"),
                "ArrowRight" => Some("Right"),
                "Backquote" => Some("`"),
                _ => None,
            };
            if let Some(named) = named {
                return named.to_string();
            }
            let bytes = part.as_bytes();
            // /^Key([A-Z])$/
            if bytes.len() == 4 && part.starts_with("Key") && bytes[3].is_ascii_uppercase() {
                return part[3..].to_string();
            }
            // /^Digit(\d)$/
            if bytes.len() == 6 && part.starts_with("Digit") && bytes[5].is_ascii_digit() {
                return part[5..].to_string();
            }
            // /^Numpad/ → "Num "
            if let Some(rest) = part.strip_prefix("Numpad") {
                return format!("Num {rest}");
            }
            part.to_string()
        })
        .collect()
}

// The keys joined for tooltips: "Play (Ctrl+Alt+Insert)".
pub fn label(accelerator: &str) -> String {
    keys(accelerator).join("+")
}

// The accelerator text a native menu shows next to an item for this stored accelerator, as the
// page's menus had it: the labels above went through muda 0.20's parser and its Windows display
// ("Ctrl+Alt+Insert" → "Ctrl+Alt+Ins", modifiers in muda's order Ctrl, Shift, Alt). None when muda
// couldn't parse them ("Win", "Num 5"): the page then rebuilt the whole menu without any
// accelerator text, and the caller should do the same.
pub fn menu_text(accelerator: &str) -> Option<String> {
    let labels = label(accelerator);
    let text = labels.trim();
    if text.is_empty() {
        return None;
    }
    // muda splits at the last '+': "Ctrl++" has the key '+'.
    let (modifiers, key) = match text.rfind('+') {
        Some(at) if text[at + 1..].trim().is_empty() => (text[..at].trim_end_matches('+'), "+"),
        Some(at) => (&text[..at], text[at + 1..].trim()),
        None => ("", text),
    };
    let mut mods = (false, false, false, false); // ctrl, shift, alt, meta
    if !modifiers.is_empty() {
        for token in modifiers.split('+') {
            match token.trim().to_uppercase().as_str() {
                "" => return None,
                "OPTION" | "ALT" => mods.2 = true,
                "CONTROL" | "CTRL" | "COMMANDORCONTROL" | "COMMANDORCTRL" | "CMDORCTRL" | "CMDORCONTROL" => mods.0 = true,
                "COMMAND" | "CMD" | "SUPER" | "META" => mods.3 = true,
                "SHIFT" => mods.1 = true,
                _ => return None,
            }
        }
    }
    let shown = muda_key_display(key)?;
    let mut out = String::new();
    if mods.0 {
        out.push_str("Ctrl+");
    }
    if mods.1 {
        out.push_str("Shift+");
    }
    if mods.2 {
        out.push_str("Alt+");
    }
    if mods.3 {
        out.push_str("Meta+");
    }
    out.push_str(&shown);
    Some(out)
}

// muda's parse_code (named codes, then the W3C code names) followed by its code_display, for the
// keys a label can name. None when muda wouldn't parse it.
fn muda_key_display(key: &str) -> Option<String> {
    let upper = key.to_uppercase();
    // Labels that map to a code, with what muda displays for it.
    let single = |text: &str| Some(text.to_string());
    match upper.as_str() {
        "`" | "BACKQUOTE" => return single("`"),
        "\\" | "BACKSLASH" => return single("\\"),
        "[" | "BRACKETLEFT" => return single("["),
        "]" | "BRACKETRIGHT" => return single("]"),
        "," | "COMMA" => return single(","),
        "=" | "EQUAL" => return single("="),
        "-" | "MINUS" => return single("-"),
        "." | "PERIOD" => return single("."),
        "'" | "QUOTE" => return single("'"),
        ";" | "SEMICOLON" => return single(";"),
        "/" | "SLASH" => return single("/"),
        "BACKSPACE" => return single("Backspace"),
        "CAPSLOCK" => return single("CapsLock"),
        "ENTER" => return single("Enter"),
        "SPACE" => return single("Space"),
        "TAB" => return single("Tab"),
        "DELETE" => return single("Del"),
        "END" => return single("End"),
        "HOME" => return single("Home"),
        "INSERT" => return single("Ins"),
        "PAGEDOWN" => return single("PgDn"),
        "PAGEUP" => return single("PgUp"),
        "PRINTSCREEN" => return single("PrintScreen"),
        "SCROLLLOCK" => return single("ScrollLock"),
        "ARROWDOWN" | "DOWN" => return single("Down"),
        "ARROWLEFT" | "LEFT" => return single("Left"),
        "ARROWRIGHT" | "RIGHT" => return single("Right"),
        "ARROWUP" | "UP" => return single("Up"),
        "NUMLOCK" => return single("NumLock"),
        "ESCAPE" | "ESC" => return single("Esc"),
        "PAUSE" | "PAUSEBREAK" => return single("Pause"),
        "MEDIAPLAYPAUSE" => return single("MediaPlayPause"),
        "MEDIASTOP" => return single("MediaStop"),
        "MEDIATRACKNEXT" => return single("MediaTrackNext"),
        "MEDIATRACKPREVIOUS" | "MEDIATRACKPREV" => return single("MediaTrackPrevious"),
        "AUDIOVOLUMEDOWN" | "VOLUMEDOWN" => return single("VolumeDown"),
        "AUDIOVOLUMEUP" | "VOLUMEUP" => return single("VolumeUp"),
        "AUDIOVOLUMEMUTE" | "VOLUMEMUTE" => return single("VolumeMute"),
        _ => {}
    }
    // One letter or digit.
    if key.len() == 1 {
        let c = key.chars().next()?;
        if c.is_ascii_alphanumeric() {
            return Some(c.to_ascii_uppercase().to_string());
        }
        return None;
    }
    // F1–F24.
    if let Some(number) = upper.strip_prefix('F').and_then(|n| n.parse::<u32>().ok()) {
        if (1..=24).contains(&number) && upper == format!("F{number}") {
            return Some(format!("F{number}"));
        }
    }
    // W3C code names muda parses but doesn't rename (exact case, as Code::from_str wants).
    match key {
        "MediaPlay" | "MediaPause" => Some(key.to_string()),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parsed(text: &str) -> Accelerator {
        Accelerator::parse(text).unwrap_or_else(|error| panic!("{text}: {error}"))
    }

    #[test]
    fn defaults_parse() {
        let play = parsed("Ctrl+Alt+Insert");
        assert_eq!(play.mods, Mods { ctrl: true, alt: true, ..Default::default() });
        assert_eq!(play.code, "Insert");
        assert_eq!(play.vk(), 0x2D);
        let stop = parsed("Ctrl+Alt+KeyH");
        assert_eq!(stop.code, "KeyH");
        assert_eq!(stop.vk(), 0x48);
    }

    #[test]
    fn parser_matches_global_hotkey() {
        // Cases and aliases.
        assert_eq!(parsed("ctrl+alt+h"), parsed("Ctrl+Alt+KeyH"));
        assert_eq!(parsed("CONTROL+OPTION+keyh"), parsed("Ctrl+Alt+KeyH"));
        assert_eq!(parsed("Shift + Alt + K").code, "KeyK");
        assert_eq!(parsed("Cmd+Up").mods.meta, true);
        assert_eq!(parsed("Super+ArrowUp").code, "ArrowUp");
        assert_eq!(parsed("KeyX").mods, Mods::default());
        assert_eq!(parsed("F9").code, "F9");
        assert_eq!(parsed("MediaPlayPause").code, "MediaPlayPause");
        assert_eq!(parsed("Ctrl+Num5").code, "Numpad5");
        assert_eq!(parsed("CommandOrControl+KeyQ").mods, if cfg!(target_os = "macos") { Mods { meta: true, ..Default::default() } } else { Mods { ctrl: true, ..Default::default() } });
        // Errors, with the same kinds as global-hotkey.
        assert_eq!(Accelerator::parse("Ctrl+"), Err(ParseError::EmptyToken("Ctrl+".into())));
        assert_eq!(Accelerator::parse("Ctrl+KeyC+Shift"), Err(ParseError::InvalidFormat("Ctrl+KeyC+Shift".into())));
        assert_eq!(Accelerator::parse("Ctrl+Shift"), Err(ParseError::InvalidFormat("Ctrl+Shift".into())));
        assert_eq!(Accelerator::parse("Ctrl+Win"), Err(ParseError::UnsupportedKey("Win".into())));
        assert!(Accelerator::parse("").is_err());
        // A single token is not trimmed.
        assert!(Accelerator::parse(" KeyX").is_err());
        assert!(Accelerator::parse("Ctrl+ContextMenu").is_err());
        assert!(Accelerator::parse("Ctrl+IntlBackslash").is_err());
    }

    #[test]
    fn stored_form_round_trips() {
        for text in [
            "Ctrl+Alt+Insert",
            "Ctrl+Alt+KeyH",
            "Ctrl+Alt+Shift+Super+F12",
            "Alt+Digit5",
            "Shift+Numpad0",
            "Super+ArrowLeft",
            "F13",
            "MediaPlayPause",
            "Ctrl+Backquote",
            "Ctrl+Alt+Slash",
        ] {
            let accelerator = parsed(text);
            assert_eq!(accelerator.to_stored(), text);
            assert_eq!(parsed(&accelerator.to_stored()), accelerator);
        }
        // Every key round-trips through its stored form.
        for key in KEYS {
            let text = format(Mods { ctrl: true, alt: true, ..Default::default() }, key.code);
            assert_eq!(parsed(&text).code, key.code, "{text}");
        }
    }

    #[test]
    fn labels_like_shortcut_keys() {
        assert_eq!(keys("Ctrl+Alt+KeyH"), ["Ctrl", "Alt", "H"]);
        assert_eq!(keys("Ctrl+Alt+Insert"), ["Ctrl", "Alt", "Insert"]);
        assert_eq!(keys("Super+Shift+Digit5"), ["Win", "Shift", "5"]);
        assert_eq!(keys("Ctrl+Numpad5"), ["Ctrl", "Num 5"]);
        assert_eq!(keys("Ctrl+NumpadAdd"), ["Ctrl", "Num Add"]);
        assert_eq!(keys("Alt+ArrowUp"), ["Alt", "Up"]);
        assert_eq!(keys("Ctrl+Backquote"), ["Ctrl", "`"]);
        assert_eq!(keys("F9"), ["F9"]);
        assert_eq!(keys("MediaPlayPause"), ["MediaPlayPause"]);
        // Only exact patterns are renamed.
        assert_eq!(keys("Ctrl+Keyboard"), ["Ctrl", "Keyboard"]);
        assert_eq!(label("Ctrl+Alt+KeyH"), "Ctrl+Alt+H");
    }

    #[test]
    fn menu_text_like_muda() {
        assert_eq!(menu_text("Ctrl+Alt+Insert").as_deref(), Some("Ctrl+Alt+Ins"));
        assert_eq!(menu_text("Ctrl+Alt+KeyH").as_deref(), Some("Ctrl+Alt+H"));
        assert_eq!(menu_text("Ctrl+Alt+Shift+KeyP").as_deref(), Some("Ctrl+Shift+Alt+P"));
        assert_eq!(menu_text("Ctrl+Delete").as_deref(), Some("Ctrl+Del"));
        assert_eq!(menu_text("Alt+ArrowUp").as_deref(), Some("Alt+Up"));
        assert_eq!(menu_text("Ctrl+Digit1").as_deref(), Some("Ctrl+1"));
        assert_eq!(menu_text("Ctrl+Backquote").as_deref(), Some("Ctrl+`"));
        assert_eq!(menu_text("Ctrl+Minus").as_deref(), Some("Ctrl+-"));
        assert_eq!(menu_text("F9").as_deref(), Some("F9"));
        assert_eq!(menu_text("Ctrl+PageUp").as_deref(), Some("Ctrl+PgUp"));
        assert_eq!(menu_text("Ctrl+AudioVolumeUp").as_deref(), Some("Ctrl+VolumeUp"));
        assert_eq!(menu_text("MediaPlayPause").as_deref(), Some("MediaPlayPause"));
        // Labels muda couldn't parse: no accelerators in that menu.
        assert_eq!(menu_text("Super+KeyP"), None);
        assert_eq!(menu_text("Ctrl+Numpad5"), None);
        assert_eq!(menu_text(""), None);
    }
}
