use super::*;
use base64::Engine;

// Published with Qust 1.1.4: its latest.json and the signatures of its installers, made by the
// Tauri CLI 2.12 in CI with the real key.
const LATEST_1_1_4: &str = include_str!("../tests/fixtures/latest-1.1.4.json");
const SIG_1_1_4_EXE: &str = include_str!("../tests/fixtures/Qust_1.1.4_x64-setup.exe.sig");
const SIG_1_1_4_APP: &str = include_str!("../tests/fixtures/Qust_1.1.4_universal.app.tar.gz.sig");

// The format proposed from now on, as packaging/latest-json.mjs writes it, here with files signed
// by qust-sign with a throwaway key (tests/fixtures/test-only.key, made by `tauri signer
// generate`): both entries sign payload.bin.
const LATEST_PROPOSED: &str = include_str!("../tests/fixtures/latest-proposed.json");
const PAYLOAD: &[u8] = include_bytes!("../tests/fixtures/payload.bin");
// payload.bin signed by `tauri signer sign --app-version 1.2.0` with the throwaway key.
const PAYLOAD_SIG_TAURI: &str = include_str!("../tests/fixtures/payload.bin.sig");
const TEST_KEY: &str = include_str!("../tests/fixtures/test-only.key.pub");

const WINDOWS: [&str; 2] = ["windows-x86_64-nsis", "windows-x86_64"];
const MAC_ARM: [&str; 2] = ["darwin-aarch64-app", "darwin-aarch64"];
const MAC_INTEL: [&str; 2] = ["darwin-x86_64-app", "darwin-x86_64"];

fn keys(targets: [&str; 2]) -> Vec<String> {
    targets.iter().map(|target| target.to_string()).collect()
}

fn b64(text: &str) -> String {
    base64::engine::general_purpose::STANDARD.encode(text)
}

fn updater(current: &str, key: &str) -> Updater {
    Updater::new("https://example.invalid/latest.json", key, current).unwrap()
}

#[test]
fn reads_the_published_1_1_4_manifest() {
    let release = Release::parse(LATEST_1_1_4).unwrap();
    assert_eq!(release.version, semver::Version::new(1, 1, 4));
    assert_eq!(release.notes.as_deref(), Some(""));
    let windows = release.platform(&keys(WINDOWS)).unwrap();
    assert_eq!(windows.url, "https://github.com/mkpascal/qust/releases/latest/download/Qust_1.1.4_x64-setup.exe");
    assert_eq!(windows.signature, SIG_1_1_4_EXE.trim());
    for targets in [MAC_ARM, MAC_INTEL] {
        let mac = release.platform(&keys(targets)).unwrap();
        assert_eq!(mac.url, "https://github.com/mkpascal/qust/releases/latest/download/Qust_universal.app.tar.gz");
        assert_eq!(mac.signature, SIG_1_1_4_APP.trim());
    }
    // This platform's own keys find an entry too.
    assert!(release.platform(&targets()).is_ok());
}

#[test]
fn reads_the_proposed_manifest() {
    let release = Release::parse(LATEST_PROPOSED).unwrap();
    assert_eq!(release.version, semver::Version::new(1, 2, 0));
    assert!(manifest::rfc3339(release.pub_date.as_deref().unwrap()));
    let windows = release.platform(&keys(WINDOWS)).unwrap();
    assert_eq!(windows.url, "https://github.com/mkpascal/qust/releases/download/v1.2.0/Qust_1.2.0_x64-setup.exe");
    for targets in [MAC_ARM, MAC_INTEL] {
        let mac = release.platform(&keys(targets)).unwrap();
        assert_eq!(mac.url, "https://github.com/mkpascal/qust/releases/download/v1.2.0/Qust_1.2.0_universal.app.tar.gz");
    }
    // Every key is there by itself too, not only as the fallback of the other.
    let value: serde_json::Value = serde_json::from_str(LATEST_PROPOSED).unwrap();
    for key in WINDOWS.iter().chain(&MAC_ARM).chain(&MAC_INTEL) {
        assert!(value["platforms"][key]["url"].is_string(), "{key}");
        assert!(value["platforms"][key]["signature"].is_string(), "{key}");
    }
}

#[test]
fn proposed_manifest_installs_end_to_end() {
    // check -> pick this platform's entry -> verify the downloaded bytes, offline.
    let update = updater("1.1.5", TEST_KEY.trim()).update_from(LATEST_PROPOSED).unwrap().unwrap();
    assert_eq!(update.version, "1.2.0");
    let package = update.verify(PAYLOAD.to_vec()).unwrap();
    assert_eq!(package.bytes(), PAYLOAD);
    assert!(update.verify(b"tampered".to_vec()).is_err());
}

#[test]
fn offers_only_newer_versions() {
    let key = PUBLIC_KEY;
    assert_eq!(updater("1.1.3", key).update_from(LATEST_1_1_4).unwrap().unwrap().version, "1.1.4");
    assert!(updater("1.1.4", key).update_from(LATEST_1_1_4).unwrap().is_none());
    assert!(updater("v1.1.4", key).update_from(LATEST_1_1_4).unwrap().is_none());
    assert!(updater("1.2.0", key).update_from(LATEST_1_1_4).unwrap().is_none());
    // semver order, as the Tauri updater compares: numbers, then a pre-release before its release
    let release = |version: &str| format!(r#"{{"version":"{version}","url":"https://example.invalid/a","signature":"x"}}"#);
    assert!(updater("1.9.0", key).update_from(&release("1.10.0")).unwrap().is_some());
    assert!(updater("1.2.0-beta.1", key).update_from(&release("1.2.0")).unwrap().is_some());
    assert!(updater("1.2.0", key).update_from(&release("1.2.0-beta.2")).unwrap().is_none());
    assert!(updater("1.1.5", key).update_from(&release("v2.0.0")).unwrap().is_some());
}

#[test]
fn missing_platform_is_an_error_only_for_an_update() {
    let manifest = r#"{"version":"9.0.0","platforms":{"linux-x86_64":{"url":"https://example.invalid/a","signature":"x"}}}"#;
    let error = updater("1.0.0", PUBLIC_KEY).update_from(manifest).err().unwrap();
    assert_eq!(error, Error::NoTarget(targets()));
    let old = manifest.replace("9.0.0", "0.9.0");
    assert!(updater("1.0.0", PUBLIC_KEY).update_from(&old).unwrap().is_none());
}

#[test]
fn reads_the_dynamic_format_and_ignores_unknown_fields() {
    let release = Release::parse(
        r#"{"name":"v2.0.0","url":"https://example.invalid/a.exe","signature":"x","extra":{"a":1},"notes":null,"pub_date":"2026-11-09T08:13:59+01:00"}"#,
    )
    .unwrap();
    assert_eq!(release.version.to_string(), "2.0.0");
    assert_eq!(release.platform(&keys(WINDOWS)).unwrap().url, "https://example.invalid/a.exe");
    let with_extra = LATEST_1_1_4.replacen("\"notes\"", "\"minimum_system\": \"10.13\", \"notes\"", 1).replacen(
        "\"url\":",
        "\"size\": 123, \"url\":",
        1,
    );
    assert!(Release::parse(&with_extra).is_ok());
}

#[test]
fn refuses_what_the_tauri_updater_refuses() {
    let good = r#"{"version":"1.2.0","pub_date":"2026-11-09T08:13:59.187Z","platforms":{"a":{"url":"https://example.invalid/a","signature":"x"},"b":{"url":"https://example.invalid/b","signature":"y"}}}"#;
    assert!(Release::parse(good).is_ok());
    let bad = [
        // one entry without a signature spoils the whole file, even for the other platforms
        good.replace(r#","signature":"y""#, ""),
        good.replace(r#""url":"https://example.invalid/b","#, ""),
        good.replace("1.2.0", "1.2"),
        good.replace(r#""version":"1.2.0","#, ""),
        good.replace("2026-11-09T08:13:59.187Z", "2026-11-09 08:13:59"),
        good.replace("2026-11-09T08:13:59.187Z", "2026-11-09"),
        good.replace("2026-11-09T08:13:59.187Z", "2026-13-09T08:13:59Z"),
        good.replace(r#""pub_date""#, r#""notes":1,"pub_date""#),
        good.replace(r#""platforms":{"#, r#""platforms":[],"x":{"#),
        r#"{"version":"1.2.0"}"#.to_string(),
        "[]".to_string(),
    ];
    for manifest in bad {
        assert!(Release::parse(&manifest).is_err(), "{manifest}");
    }
    // Downloads are https only here (the Tauri updater would allow http).
    assert!(Release::parse(&good.replace("https://example.invalid/b", "http://example.invalid/b")).is_err());
}

#[test]
fn rfc3339_dates() {
    for date in ["2026-11-09T08:13:59.187Z", "2026-11-09T08:13:59Z", "2026-11-09t08:13:59z", "2026-11-09T08:13:59-05:30"] {
        assert!(manifest::rfc3339(date), "{date}");
    }
    for date in ["2026-11-09T08:13:59", "2026-11-09T08:13Z", "2026-11-09T08:13:59.Z", "2026-11-09T24:00:00Z", "2026-00-09T08:13:59Z", "26-11-09T08:13:59Z"] {
        assert!(!manifest::rfc3339(date), "{date}");
    }
}

#[test]
fn verifies_a_tauri_signature() {
    verify(PAYLOAD, PAYLOAD_SIG_TAURI.trim(), TEST_KEY.trim(), "1.2.0").unwrap();
    verify(PAYLOAD, PAYLOAD_SIG_TAURI.trim(), TEST_KEY.trim(), "v1.2.0").unwrap();
}

#[test]
fn refuses_a_tampered_file_or_a_wrong_version_or_key() {
    let sig = PAYLOAD_SIG_TAURI.trim();
    let key = TEST_KEY.trim();
    let mut tampered = PAYLOAD.to_vec();
    tampered[10] ^= 1;
    assert!(matches!(verify(&tampered, sig, key, "1.2.0"), Err(Error::Signature(_))));
    assert!(verify(&PAYLOAD[..PAYLOAD.len() - 1], sig, key, "1.2.0").is_err());
    // Signed for 1.2.0, announced as something else: an old file passed off as new.
    assert!(verify(PAYLOAD, sig, key, "1.2.1").is_err());
    // Not Qust's key.
    assert!(verify(PAYLOAD, sig, PUBLIC_KEY, "1.2.0").is_err());
    // The trusted comment is covered by the signature: changing the version in it breaks it.
    let text = String::from_utf8(base64::engine::general_purpose::STANDARD.decode(sig).unwrap()).unwrap();
    let forged = b64(&text.replace("version:1.2.0", "version:1.3.0"));
    assert!(verify(PAYLOAD, &forged, key, "1.3.0").is_err());
    // The same strict base64 as the Tauri updater: a line break in it is refused.
    assert!(verify(PAYLOAD, &format!("{sig}\n"), key, "1.2.0").is_err());
    assert!(verify(PAYLOAD, "not base64!", key, "1.2.0").is_err());
}

#[test]
fn requires_a_signed_version() {
    // minisign-verify's public test vector (also in tauri-plugin-updater's tests): payload
    // "test", signed with no version in the trusted comment.
    let key = "untrusted comment: minisign public key E7620F1842B4E81F\nRWQf6LRCGA9i53mlYecO4IzT51TGPpvWucNSCh1CBM0QTaLn73Y7GFO3";
    let sig = "untrusted comment: signature from minisign secret key\nRUQf6LRCGA9i559r3g7V1qNyJDApGip8MfqcadIgT9CuhV3EMhHoN1mGTkUidF/z7SrlQgXdy8ofjb7bNJJylDOocrCo8KLzZwo=\ntrusted comment: timestamp:1633700835\tfile:test\tprehashed\nwLMDjy9FLAuxZ3q4NlEvkgtyhrr0gtTu6KC4KBJdITbbOeAi1zBIYo0v4iTgt8jJpIidRJnp94ABQkJAgAooBQ==";
    let error = verify(b"test", &b64(sig), &b64(key), "1.0.0").err().unwrap();
    assert!(error.to_string().contains("which version"), "{error}");
    assert!(verify(b"tset", &b64(sig), &b64(key), "1.0.0").is_err());
}

#[test]
fn reads_the_real_key_and_release_signatures() {
    // The real public key and the 1.1.4 signatures decode and belong together; wrong bytes then
    // fail on the signature itself, not on the format.
    let key = String::from_utf8(base64::engine::general_purpose::STANDARD.decode(PUBLIC_KEY).unwrap()).unwrap();
    assert!(key.starts_with("untrusted comment: minisign public key: 1409FDC858141A4F\n"));
    for sig in [SIG_1_1_4_EXE, SIG_1_1_4_APP] {
        let text = String::from_utf8(base64::engine::general_purpose::STANDARD.decode(sig.trim()).unwrap()).unwrap();
        let signature = minisign_verify::Signature::decode(&text).unwrap();
        assert_eq!(signature::signed_version(signature.trusted_comment()), Some("1.1.4"));
        let error = verify(b"not the installer", sig.trim(), PUBLIC_KEY, "1.1.4").err().unwrap();
        assert_eq!(error, Error::Signature(minisign_verify::Error::InvalidSignature.to_string()));
        // The test key's id is not the release key's.
        let error = verify(b"x", sig.trim(), TEST_KEY.trim(), "1.1.4").err().unwrap();
        assert!(matches!(error, Error::Signature(_)));
    }
}

#[test]
fn schedule() {
    let hours = |hours: u64| hours * 60 * 60;
    assert!(check_due(None, now()));
    assert!(!check_due(Some(1_000_000), 1_000_000 + hours(7)));
    assert!(check_due(Some(1_000_000), 1_000_000 + hours(8)));
    assert!(!check_due(Some(2_000_000), 1_000_000));
}

#[test]
fn status_names() {
    let names: Vec<_> =
        [Status::Idle, Status::Checking, Status::Current, Status::Downloading, Status::Ready, Status::Installing, Status::Error]
            .iter()
            .map(|status| status.as_str())
            .collect();
    assert_eq!(names, ["idle", "checking", "current", "downloading", "ready", "installing", "error"]);
}

#[test]
fn not_installed_in_development() {
    // Test builds are debug builds, which never update themselves.
    if cfg!(debug_assertions) {
        assert!(!installed());
        let updates = Updates::new(updater("1.0.0", PUBLIC_KEY), |_| {});
        assert_eq!(updates.check(), Err("Updates work in the installed version of Qust".into()));
        assert_eq!(updates.snapshot(), Snapshot::default());
    }
}

#[cfg(windows)]
#[test]
fn installer_arguments_match_the_tauri_updater() {
    use std::ffi::OsString;
    let parameters = |args: &[&str]| windows::parameters(&args.iter().map(OsString::from).collect::<Vec<_>>());
    assert_eq!(parameters(&[]), "/P /UPDATE /R /ARGS");
    assert_eq!(parameters(&["--background"]), "/P /UPDATE /R /ARGS --background");
    assert_eq!(
        parameters(&["a b", "/x", "", r#"say "hi""#, r"C:\dir\", r"C:\a dir\"]),
        r#"/P /UPDATE /R /ARGS "a b" "/x" "" "say \"hi\"" C:\dir\ "C:\a dir\\""#
    );
}

mod archives {
    use super::super::archive::unpack_bundle;
    use crate::Error;
    use std::path::Path;

    fn tar_gz(entries: &[(&str, &[u8])]) -> Vec<u8> {
        let mut builder = tar::Builder::new(flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast()));
        for (path, data) in entries {
            let mut header = tar::Header::new_gnu();
            header.set_size(data.len() as u64);
            header.set_mode(0o755);
            header.set_entry_type(tar::EntryType::Regular);
            // set_path refuses "..", which is the point of one test: write the name directly.
            let name = &mut header.as_gnu_mut().unwrap().name;
            name[..path.len()].copy_from_slice(path.as_bytes());
            header.set_cksum();
            builder.append(&header, *data).unwrap();
        }
        builder.into_inner().unwrap().finish().unwrap()
    }

    fn temp(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("qust-updater-test-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn unpacks_the_bundle_into_place() {
        let dir = temp("unpack");
        let archive = tar_gz(&[("Qust.app/Contents/Info.plist", b"<plist/>"), ("Qust.app/Contents/MacOS/qust", b"binary")]);
        unpack_bundle(&archive, &dir).unwrap();
        assert_eq!(std::fs::read(dir.join("Contents/Info.plist")).unwrap(), b"<plist/>");
        assert_eq!(std::fs::read(dir.join(Path::new("Contents/MacOS/qust"))).unwrap(), b"binary");
        assert!(!dir.join("Qust.app").exists());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn refuses_paths_that_climb_out_and_archives_without_a_bundle() {
        let dir = temp("climb");
        let archive = tar_gz(&[("Qust.app/../../evil", b"x"), ("Qust.app/Contents/Info.plist", b"<plist/>")]);
        assert!(matches!(unpack_bundle(&archive, &dir), Err(Error::Install(_))));
        let empty = tar_gz(&[("Qust.app/readme", b"x")]);
        assert!(matches!(unpack_bundle(&empty, &dir), Err(Error::Install(_))));
        std::fs::remove_dir_all(&dir).unwrap();
    }
}

// Against GitHub: `cargo test -p qust-updater -- --ignored`.
#[test]
#[ignore]
fn network_unpacks_the_published_mac_archive() {
    let url = "https://github.com/mkpascal/qust/releases/download/v1.1.4/Qust_universal.app.tar.gz";
    let mut bytes = Vec::new();
    ureq::get(url).call().unwrap().into_reader().read_to_end(&mut bytes).unwrap();
    verify(&bytes, SIG_1_1_4_APP.trim(), PUBLIC_KEY, "1.1.4").unwrap();
    let dir = std::env::temp_dir().join(format!("qust-updater-test-mac-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    archive::unpack_bundle(&bytes, &dir).unwrap();
    assert!(dir.join("Contents/MacOS/qust").is_file());
    assert!(dir.join("Contents/Resources/icon.icns").is_file());
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
#[ignore]
fn network_finds_and_verifies_the_published_release() {
    let updater = Updater::new(ENDPOINT, PUBLIC_KEY, "1.0.0").unwrap();
    let update = updater.check().unwrap().expect("a release newer than 1.0.0");
    let mut last = 0;
    let package = update.download(|received, _| last = received).unwrap();
    assert_eq!(last as usize, package.bytes().len());
    if cfg!(windows) {
        assert!(package.bytes().starts_with(b"MZ"));
    }
}
