// The user's accent from the Windows palette: the base color, the lighter shade Windows uses in
// dark mode (Light 2) and the darker one used in light mode (Dark 1).
#[cfg(windows)]
pub fn accent() -> Option<serde_json::Value> {
    let key = windows_registry::CURRENT_USER
        .open(r"Software\Microsoft\Windows\CurrentVersion\Explorer\Accent")
        .ok()?;
    let palette = key.get_value("AccentPalette").ok()?;
    let color = |index: usize| {
        let rgb = palette.get(index * 4..index * 4 + 3)?;
        Some(format!("#{:02x}{:02x}{:02x}", rgb[0], rgb[1], rgb[2]))
    };
    Some(serde_json::json!({ "base": color(3)?, "light": color(1)?, "dark": color(4)? }))
}

#[cfg(not(windows))]
pub fn accent() -> Option<serde_json::Value> {
    None
}
