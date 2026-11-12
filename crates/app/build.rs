// The release version is the workspace's (`[workspace.package] version` in the root Cargo.toml,
// the one place a release bumps it): the app reads it as env!("QUST_RELEASE_VERSION") (the
// updater compares it with the latest release, Settings shows it) and the executable's version
// information gets the same numbers. The resources are the shell's qust.rc (icon 1, version
// information); no manifest, GPUI embeds its own.

use std::path::PathBuf;

fn main() {
    let manifest = PathBuf::from(std::env::var_os("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR"));
    let rc = manifest.join("../shell/resources/qust.rc");
    let icons = manifest.join("../../icons");
    println!("cargo:rerun-if-changed={}", rc.display());
    println!("cargo:rerun-if-changed={}", icons.join("icon.ico").display());

    let version = std::env::var("CARGO_PKG_VERSION").expect("CARGO_PKG_VERSION");
    let numbers = version_numbers(&version);
    println!("cargo:rustc-env=QUST_RELEASE_VERSION={version}");

    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        let macros = [format!("QUST_VERSION=\"{version}\""), format!("QUST_VERSION_NUMBERS={numbers}")];
        let includes = [icons.display().to_string()];
        if let Err(error) = embed_resource::compile(&rc, embed_resource::ParamsMacrosAndIncludeDirs(&macros, &includes)).manifest_required() {
            panic!("qust.rc: {error}");
        }
    }
}

// "1.2.0" (or "1.2.0-beta.1") -> "1,2,0,0" for VERSIONINFO, which takes four numbers.
fn version_numbers(version: &str) -> String {
    let core = version.split(['-', '+']).next().unwrap_or(version);
    let mut parts: Vec<u16> = core.split('.').map(|part| part.parse().unwrap_or_else(|_| panic!("bad version {version}"))).collect();
    if parts.len() != 3 {
        panic!("version {version} is not x.y.z");
    }
    parts.push(0);
    parts.iter().map(u16::to_string).collect::<Vec<_>>().join(",")
}
