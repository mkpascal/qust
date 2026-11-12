// qust-sign with a key made by `tauri signer generate` (tests/fixtures/test-only.key, a
// throwaway key with the password below): its signatures must verify the way both updaters
// verify, and say the version they're for.
//   cargo test -p qust-updater --features sign
#![cfg(feature = "sign")]

use base64::Engine;
use std::path::PathBuf;
use std::process::Command;

const PASSWORD: &str = "test-only-password";

fn fixtures() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests").join("fixtures")
}

fn test_key() -> String {
    std::fs::read_to_string(fixtures().join("test-only.key.pub")).unwrap().trim().to_string()
}

fn sign(file: &std::path::Path, version: &str, key_env: (&str, String)) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_qust-sign"))
        .arg(file)
        .args(["--version", version, "--pubkey", &test_key()])
        .env_remove("TAURI_SIGNING_PRIVATE_KEY")
        .env_remove("TAURI_SIGNING_PRIVATE_KEY_PATH")
        .env(key_env.0, key_env.1)
        .env("TAURI_SIGNING_PRIVATE_KEY_PASSWORD", PASSWORD)
        .output()
        .unwrap()
}

fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("qust-sign-test-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("Qust_1.2.0_x64-setup.exe");
    std::fs::copy(fixtures().join("payload.bin"), &file).unwrap();
    file
}

#[test]
fn signs_like_the_tauri_cli() {
    let key_path = fixtures().join("test-only.key");
    // The key as a path (TAURI_SIGNING_PRIVATE_KEY_PATH) and as the string itself, as the
    // repository secret holds it.
    let key_string = std::fs::read_to_string(&key_path).unwrap();
    for (name, env) in [
        ("path", ("TAURI_SIGNING_PRIVATE_KEY_PATH", key_path.display().to_string())),
        ("string", ("TAURI_SIGNING_PRIVATE_KEY", key_string)),
    ] {
        let file = scratch(name);
        let output = sign(&file, "1.2.0", env);
        assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));

        let signature = std::fs::read_to_string(file.with_extension("exe.sig")).unwrap();
        // One line of base64, no newline: the Tauri updater's decoder takes nothing else.
        assert!(!signature.contains(['\n', '\r', ' ']));
        let text = String::from_utf8(base64::engine::general_purpose::STANDARD.decode(&signature).unwrap()).unwrap();
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(lines[0], "untrusted comment: signature from tauri secret key");
        // Prehashed (Ed25519 over BLAKE2b-512), like the Tauri CLI's: "ED" -> "RUR"/"RUQ".
        assert!(lines[1].starts_with("RU"), "{}", lines[1]);
        assert_eq!(&base64::engine::general_purpose::STANDARD.decode(lines[1]).unwrap()[..2], b"ED");
        assert!(lines[2].starts_with("trusted comment: timestamp:"));
        assert!(lines[2].ends_with("\tfile:Qust_1.2.0_x64-setup.exe\tversion:1.2.0"), "{}", lines[2]);

        let data = std::fs::read(&file).unwrap();
        qust_updater::verify(&data, &signature, &test_key(), "1.2.0").unwrap();
        assert!(qust_updater::verify(&data, &signature, &test_key(), "1.2.1").is_err());
        assert!(qust_updater::verify(b"tampered", &signature, &test_key(), "1.2.0").is_err());
        assert!(qust_updater::verify(&data, &signature, qust_updater::PUBLIC_KEY, "1.2.0").is_err());
        std::fs::remove_dir_all(file.parent().unwrap()).unwrap();
    }
}

#[test]
fn refuses_a_wrong_password_and_a_missing_version() {
    let file = scratch("refuse");
    let wrong = Command::new(env!("CARGO_BIN_EXE_qust-sign"))
        .arg(&file)
        .args(["--version", "1.2.0", "--pubkey", &test_key()])
        .env_remove("TAURI_SIGNING_PRIVATE_KEY")
        .env("TAURI_SIGNING_PRIVATE_KEY_PATH", fixtures().join("test-only.key"))
        .env("TAURI_SIGNING_PRIVATE_KEY_PASSWORD", "wrong")
        .output()
        .unwrap();
    assert!(!wrong.status.success());
    let no_version = Command::new(env!("CARGO_BIN_EXE_qust-sign")).arg(&file).output().unwrap();
    assert!(!no_version.status.success());
    // Signed with the test key, checked against Qust's real key (the default): refused.
    let output = sign(&file, "1.2.0", ("TAURI_SIGNING_PRIVATE_KEY_PATH", fixtures().join("test-only.key").display().to_string()));
    assert!(output.status.success());
    let real = Command::new(env!("CARGO_BIN_EXE_qust-sign")).arg("--verify").arg(&file).args(["--version", "1.2.0"]).output().unwrap();
    assert!(!real.status.success());
    std::fs::remove_dir_all(file.parent().unwrap()).unwrap();
}

#[test]
fn verifies_what_the_tauri_cli_signed() {
    // payload.bin.sig was made by `tauri signer sign --app-version 1.2.0`.
    let output = Command::new(env!("CARGO_BIN_EXE_qust-sign"))
        .arg("--verify")
        .arg(fixtures().join("payload.bin"))
        .args(["--version", "1.2.0", "--pubkey", &test_key()])
        .output()
        .unwrap();
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
}
