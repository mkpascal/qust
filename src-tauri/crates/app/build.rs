// The release version comes from package.json, the one place a release bumps it (with
// src-tauri/Cargo.toml for the Tauri app until it goes): the app reads it as
// env!("QUST_RELEASE_VERSION") (the updater compares it with the latest release, Settings shows
// it) and the executable's version information gets the same numbers. The resources are the
// shell's qust.rc (icon 1, version information); no manifest, GPUI embeds its own.

use std::path::{Path, PathBuf};

fn main() {
    let manifest = PathBuf::from(std::env::var_os("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR"));
    let repo = manifest.join("../../..");
    let package = repo.join("package.json");
    let rc = manifest.join("../shell/resources/qust.rc");
    let icons = manifest.join("../../icons");
    println!("cargo:rerun-if-changed={}", package.display());
    println!("cargo:rerun-if-changed={}", rc.display());
    println!("cargo:rerun-if-changed={}", icons.join("icon.ico").display());

    let version = release_version(&package);
    let numbers = version_numbers(&version);
    println!("cargo:rustc-env=QUST_RELEASE_VERSION={version}");

    // qust.rc's fallback numbers (used when built without these defines) should not drift
    // either; they are only a fallback, so a mismatch is a warning.
    if let Ok(text) = std::fs::read_to_string(&rc) {
        let fallback = format!("#define QUST_VERSION \"{version}\"");
        if !text.contains(&fallback) {
            println!("cargo:warning=crates/shell/resources/qust.rc's fallback QUST_VERSION differs from package.json's {version} (the build passes package.json's)");
        }
    }

    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        let macros = [format!("QUST_VERSION=\"{version}\""), format!("QUST_VERSION_NUMBERS={numbers}")];
        let includes = [icons.display().to_string()];
        if let Err(error) = embed_resource::compile(&rc, embed_resource::ParamsMacrosAndIncludeDirs(&macros, &includes)).manifest_required() {
            panic!("qust.rc: {error}");
        }
    }
}

fn release_version(package: &Path) -> String {
    let text = std::fs::read_to_string(package).unwrap_or_else(|error| panic!("{}: {error}", package.display()));
    let json: serde_json::Value = serde_json::from_str(&text).unwrap_or_else(|error| panic!("{}: {error}", package.display()));
    json["version"].as_str().unwrap_or_else(|| panic!("{}: no version", package.display())).to_string()
}

// "1.1.5" (or "1.2.0-beta.1") → "1,1,5,0" for VERSIONINFO, which takes four numbers.
fn version_numbers(version: &str) -> String {
    let core = version.split(['-', '+']).next().unwrap_or(version);
    let mut parts: Vec<u16> = core.split('.').map(|part| part.parse().unwrap_or_else(|_| panic!("package.json: bad version {version}"))).collect();
    if parts.len() != 3 {
        panic!("package.json: version {version} is not x.y.z");
    }
    parts.push(0);
    parts.iter().map(u16::to_string).collect::<Vec<_>>().join(",")
}
