// latest.json, read the way tauri-plugin-updater 2.14 reads it (updater.rs, `RemoteRelease`'s
// Deserialize impl), so that whatever this accepts the Tauri versions of Qust accept too:
// - `version` (or `name`): semver, leading `v`s ignored
// - `notes`: a string or nothing; `pub_date`: an RFC 3339 date or nothing
// - `platforms`: every entry must have a `url` and a `signature`, or the whole file is refused;
//   without `platforms`, a top-level `url` and `signature` (the "dynamic" format)
// - any other field is ignored

use crate::Error;
use serde_json::{Map, Value};
use std::collections::HashMap;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Platform {
    pub url: String,
    // base64 of the minisign signature file, as in the .sig asset.
    pub signature: String,
}

#[derive(Debug, Clone)]
pub struct Release {
    pub version: semver::Version,
    pub notes: Option<String>,
    pub pub_date: Option<String>,
    platforms: Platforms,
}

#[derive(Debug, Clone)]
enum Platforms {
    Static(HashMap<String, Platform>),
    Dynamic(Platform),
}

// The keys the Tauri updater looks for, in its order: `{os}-{arch}-{installer}`, then
// `{os}-{arch}`. Installed Tauri copies are NSIS installs on Windows and .app bundles on macOS.
pub fn targets() -> Vec<String> {
    let os = if cfg!(windows) {
        "windows"
    } else if cfg!(target_os = "macos") {
        "darwin"
    } else {
        "linux"
    };
    let arch = if cfg!(target_arch = "aarch64") {
        "aarch64"
    } else if cfg!(target_arch = "x86") {
        "i686"
    } else {
        "x86_64"
    };
    let installer = if cfg!(windows) {
        Some("nsis")
    } else if cfg!(target_os = "macos") {
        Some("app")
    } else {
        None
    };
    let mut targets = Vec::new();
    if let Some(installer) = installer {
        targets.push(format!("{os}-{arch}-{installer}"));
    }
    targets.push(format!("{os}-{arch}"));
    targets
}

pub(crate) fn parse_version(text: &str) -> Result<semver::Version, String> {
    semver::Version::parse(text.trim_start_matches('v')).map_err(|error| format!("{text:?}: {error}"))
}

impl Release {
    pub fn parse(text: &str) -> Result<Release, Error> {
        let value: Value = serde_json::from_str(text).map_err(|error| Error::Manifest(error.to_string()))?;
        Release::from_value(&value).map_err(Error::Manifest)
    }

    fn from_value(value: &Value) -> Result<Release, String> {
        let object = value.as_object().ok_or("not a JSON object")?;
        if object.contains_key("version") && object.contains_key("name") {
            return Err("both `version` and `name`".into());
        }
        let version = match object.get("version").or_else(|| object.get("name")) {
            Some(Value::String(version)) => parse_version(version)?,
            Some(_) => return Err("`version` is not a string".into()),
            None => return Err("no `version`".into()),
        };
        let notes = optional_string(object, "notes")?;
        let pub_date = optional_string(object, "pub_date")?;
        if let Some(date) = &pub_date {
            if !is_rfc3339(date) {
                return Err(format!("`pub_date` {date:?} is not an RFC 3339 date"));
            }
        }
        let platforms = match object.get("platforms") {
            None | Some(Value::Null) => Platforms::Dynamic(platform(object).map_err(|error| format!("top level: {error}"))?),
            Some(Value::Object(entries)) => {
                let mut platforms = HashMap::new();
                for (key, entry) in entries {
                    let entry = entry.as_object().ok_or_else(|| format!("`platforms.{key}` is not an object"))?;
                    platforms.insert(key.clone(), platform(entry).map_err(|error| format!("`platforms.{key}`: {error}"))?);
                }
                Platforms::Static(platforms)
            }
            Some(_) => return Err("`platforms` is not an object".into()),
        };
        Ok(Release { version, notes, pub_date, platforms })
    }

    // The entry for the first of `targets` the release has.
    pub fn platform(&self, targets: &[String]) -> Result<&Platform, Error> {
        match &self.platforms {
            Platforms::Dynamic(platform) => Ok(platform),
            Platforms::Static(platforms) => targets
                .iter()
                .find_map(|target| platforms.get(target))
                .ok_or_else(|| Error::NoTarget(targets.to_vec())),
        }
    }
}

fn optional_string(object: &Map<String, Value>, key: &str) -> Result<Option<String>, String> {
    match object.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(text)) => Ok(Some(text.clone())),
        Some(_) => Err(format!("`{key}` is not a string")),
    }
}

fn platform(object: &Map<String, Value>) -> Result<Platform, String> {
    let url = match object.get("url") {
        Some(Value::String(url)) => url.clone(),
        _ => return Err("no `url`".into()),
    };
    let signature = match object.get("signature") {
        Some(Value::String(signature)) => signature.clone(),
        _ => return Err("no `signature`".into()),
    };
    // The Tauri updater only needs a URL here; this one downloads over HTTPS only, since the
    // signature is all that protects a plain HTTP download.
    let rest = url.strip_prefix("https://").ok_or_else(|| format!("{url:?} is not an https URL"))?;
    if rest.is_empty() || rest.starts_with('/') || url.chars().any(|c| c.is_whitespace() || c.is_control()) {
        return Err(format!("{url:?} is not a valid URL"));
    }
    Ok(Platform { url, signature })
}

// `YYYY-MM-DDTHH:MM:SS[.fraction](Z|+HH:MM|-HH:MM)`, what the Tauri updater's
// `time::format_description::well_known::Rfc3339` parses.
fn is_rfc3339(text: &str) -> bool {
    let bytes = text.as_bytes();
    let number = |range: std::ops::Range<usize>, max: u32| -> bool {
        bytes.get(range.clone()).is_some_and(|digits| digits.iter().all(u8::is_ascii_digit))
            && text[range].parse::<u32>().is_ok_and(|value| value <= max)
    };
    if bytes.len() < 20
        || !number(0..4, 9999)
        || bytes[4] != b'-'
        || !number(5..7, 12)
        || bytes[7] != b'-'
        || !number(8..10, 31)
        || !matches!(bytes[10], b'T' | b't')
        || !number(11..13, 23)
        || bytes[13] != b':'
        || !number(14..16, 59)
        || bytes[16] != b':'
        || !number(17..19, 60)
        || text[5..7] == *"00"
        || text[8..10] == *"00"
    {
        return false;
    }
    let mut rest = &text[19..];
    if let Some(fraction) = rest.strip_prefix('.') {
        let digits = fraction.bytes().take_while(u8::is_ascii_digit).count();
        if digits == 0 {
            return false;
        }
        rest = &fraction[digits..];
    }
    match rest.as_bytes() {
        [b'Z' | b'z'] => true,
        [b'+' | b'-', h1, h2, b':', m1, m2] => {
            [h1, h2, m1, m2].iter().all(|digit| digit.is_ascii_digit())
                && rest[1..3].parse::<u32>().is_ok_and(|hours| hours <= 23)
                && rest[4..6].parse::<u32>().is_ok_and(|minutes| minutes <= 59)
        }
        _ => false,
    }
}

#[cfg(test)]
pub(crate) fn rfc3339(text: &str) -> bool {
    is_rfc3339(text)
}
