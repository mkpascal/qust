//! The Stream information window's formatting, as info.js did it with JavaScript's number
//! formatting (`String(n)`, `toFixed`, `toLocaleString('en')`, `Math.round`). `qust_ui::format`
//! has the player's; these are the ones info.js needed on top (candidates to move there).

/// `String(n)` for a number: integers without a fraction ("128", not "128.0"), other values in
/// their shortest form ("2.43").
pub fn js_number(value: f64) -> String {
    if value.is_nan() {
        return "NaN".into();
    }
    if value.is_infinite() {
        return if value > 0.0 { "Infinity".into() } else { "-Infinity".into() };
    }
    if value.fract() == 0.0 && value.abs() < 1e21 {
        return format!("{}", value as i128);
    }
    format!("{value}")
}

/// Rounds a non-negative decimal string ("12.3456") to `digits` fraction digits, ties away from
/// zero, keeping exactly `digits` digits.
fn round_digits(text: &str, digits: usize) -> String {
    let (int, frac) = text.split_once('.').unwrap_or((text, ""));
    let mut all: Vec<u8> = int.bytes().chain(frac.bytes().chain(std::iter::repeat(b'0')).take(digits)).map(|b| b - b'0').collect();
    let round_up = frac.as_bytes().get(digits).is_some_and(|b| *b >= b'5');
    if round_up {
        let mut i = all.len();
        loop {
            if i == 0 {
                all.insert(0, 1);
                break;
            }
            i -= 1;
            if all[i] == 9 {
                all[i] = 0;
            } else {
                all[i] += 1;
                break;
            }
        }
    }
    let split = all.len() - digits;
    let int: String = all[..split].iter().map(|d| char::from(b'0' + d)).collect();
    let frac: String = all[split..].iter().map(|d| char::from(b'0' + d)).collect();
    let int = if int.is_empty() { "0".to_string() } else { int };
    if digits == 0 {
        int
    } else {
        format!("{int}.{frac}")
    }
}

/// `n.toFixed(digits)`: the exact value of the double, rounded half up.
pub fn to_fixed(value: f64, digits: usize) -> String {
    if !value.is_finite() {
        return js_number(value);
    }
    // 60 digits cover every double the windows format exactly enough to decide the rounding.
    let exact = format!("{:.60}", value.abs());
    let text = round_digits(&exact, digits);
    let zero = text.bytes().all(|b| b == b'0' || b == b'.');
    if value < 0.0 && !zero {
        format!("-{text}")
    } else {
        text
    }
}

/// `n.toLocaleString('en', { maximumFractionDigits })` (the default is 3): thousands separated
/// by commas, at most that many fraction digits (rounded half away from zero on the shortest
/// decimal form, as ICU does), trailing zeros dropped.
pub fn locale_en(value: f64, max_fraction: usize) -> String {
    if !value.is_finite() {
        return if value.is_nan() { "NaN".into() } else if value > 0.0 { "∞".into() } else { "-∞".into() };
    }
    let shortest = format!("{}", value.abs());
    let rounded = round_digits(&shortest, max_fraction);
    let (int, frac) = rounded.split_once('.').unwrap_or((&rounded, ""));
    let frac = frac.trim_end_matches('0');
    let mut grouped = String::new();
    for (i, c) in int.chars().enumerate() {
        if i > 0 && (int.len() - i) % 3 == 0 {
            grouped.push(',');
        }
        grouped.push(c);
    }
    let zero = int.bytes().all(|b| b == b'0') && frac.is_empty();
    let sign = if value < 0.0 && !zero { "-" } else { "" };
    if frac.is_empty() {
        format!("{sign}{grouped}")
    } else {
        format!("{sign}{grouped}.{frac}")
    }
}

/// `Math.round(x)`: halves toward +∞.
pub fn js_round(value: f64) -> f64 {
    (value + 0.5).floor()
}

/// info.js `bytes`: under 1,000,000 "N KB" (÷1000, rounded), else "x.x MB".
pub fn bytes(count: f64) -> String {
    if count < 1e6 {
        format!("{} KB", js_number(js_round(count / 1000.0)))
    } else {
        format!("{} MB", to_fixed(count / 1e6, 1))
    }
}

/// info.js `duration`: `M:SS`, or `H:MM:SS` from an hour.
pub fn duration(seconds: f64) -> String {
    let h = (seconds / 3600.0).floor();
    let m = (seconds / 60.0).floor() % 60.0;
    let s = js_number(seconds % 60.0);
    let s = if s.chars().count() < 2 { format!("{s:0>2}") } else { s };
    if h != 0.0 {
        let m = js_number(m);
        let m = if m.chars().count() < 2 { format!("{m:0>2}") } else { m };
        format!("{}:{m}:{s}", js_number(h))
    } else {
        format!("{}:{s}", js_number(m))
    }
}

/// The format from a content type (info.js `FORMATS`): `audio/mpeg; charset=…` → MP3.
pub fn format_from_content_type(content_type: &str) -> Option<&'static str> {
    let base = content_type.split(';').next().unwrap_or("").trim().to_lowercase();
    Some(match base.as_str() {
        "audio/mpeg" => "MP3",
        "audio/aac" => "AAC",
        "audio/aacp" => "AAC+",
        "audio/ogg" | "application/ogg" => "Ogg",
        "audio/flac" => "FLAC",
        "audio/opus" => "Opus",
        _ => return None,
    })
}

/// JavaScript truthiness of an optional number (null, 0 and NaN are false).
pub fn truthy(value: Option<f64>) -> Option<f64> {
    value.filter(|v| *v != 0.0 && !v.is_nan())
}

/// JavaScript truthiness of an optional string ("" is false).
pub fn present(value: &Option<String>) -> Option<&str> {
    value.as_deref().filter(|s| !s.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn like_javascript() {
        assert_eq!(js_number(128.0), "128");
        assert_eq!(js_number(2.43), "2.43");
        assert_eq!(js_number(-0.0), "0");
        assert_eq!(to_fixed(2.43, 1), "2.4");
        assert_eq!(to_fixed(0.25, 1), "0.3");
        assert_eq!(to_fixed(12.364110, 1), "12.4");
        assert_eq!(to_fixed(9.96, 1), "10.0");
        assert_eq!(to_fixed(0.0, 1), "0.0");
        assert_eq!(locale_en(16000.0, 3), "16,000");
        assert_eq!(locale_en(2345678.0, 3), "2,345,678");
        assert_eq!(locale_en(44.1, 1), "44.1");
        assert_eq!(locale_en(48.0, 1), "48");
        assert_eq!(locale_en(22.05, 1), "22.1");
        assert_eq!(locale_en(0.125, 2), "0.13");
        assert_eq!(locale_en(999.96, 1), "1,000");
        assert_eq!(locale_en(1234.5, 3), "1,234.5");
        assert_eq!(bytes(812000.0), "812 KB");
        assert_eq!(bytes(500.0), "1 KB");
        assert_eq!(bytes(499.0), "0 KB");
        assert_eq!(bytes(12345678.0 + 18432.0), "12.4 MB");
        assert_eq!(bytes(987654321.0 + 2345678.0), "990.0 MB");
        assert_eq!(duration(754.0), "12:34");
        assert_eq!(duration(61.0), "1:01");
        assert_eq!(duration(3725.0), "1:02:05");
        assert_eq!(duration(5.0), "0:05");
        assert_eq!(format_from_content_type("audio/aac; charset=utf-8; codecs=\"mp4a.40.2\""), Some("AAC"));
        assert_eq!(format_from_content_type(" Audio/AACP "), Some("AAC+"));
        assert_eq!(format_from_content_type("video/mp4"), None);
        assert_eq!(truthy(Some(0.0)), None);
        assert_eq!(present(&Some(String::new())), None);
    }
}
