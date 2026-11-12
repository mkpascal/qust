// Signs release files for the updaters, as `tauri signer sign --app-version` did, with the same
// key from the same repository secrets:
//   TAURI_SIGNING_PRIVATE_KEY           the key: base64 of minisign's secret key file, as
//                                       `tauri signer generate` wrote it (or a path to that file)
//   TAURI_SIGNING_PRIVATE_KEY_PATH      or a path to it
//   TAURI_SIGNING_PRIVATE_KEY_PASSWORD  its password
//
//   qust-sign <file> --version <x.y.z> [--pubkey <base64>]
//     writes <file>.sig: base64 of the minisign signature file, which goes as is into
//     latest.json. The trusted comment is `timestamp:<secs>\tfile:<name>\tversion:<x.y.z>`, as
//     the Tauri CLI writes it. The new signature is then checked against the public key
//     (Qust's own unless --pubkey), the way the updaters will check it.
//   qust-sign --verify <file> --version <x.y.z> [--pubkey <base64>]
//     checks <file> against <file>.sig.
//
// Built with `cargo run -p qust-updater --features sign --release --bin qust-sign -- …`.

use base64::Engine;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::{SystemTime, UNIX_EPOCH};

fn main() -> ExitCode {
    match run(std::env::args().skip(1).collect()) {
        Ok(message) => {
            println!("{message}");
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("qust-sign: {error}");
            ExitCode::FAILURE
        }
    }
}

fn run(args: Vec<String>) -> Result<String, String> {
    let mut file = None;
    let mut version = None;
    let mut pubkey = qust_updater::PUBLIC_KEY.to_string();
    let mut check_only = false;
    let mut args = args.into_iter();
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--version" => version = Some(args.next().ok_or("--version needs a value")?),
            "--pubkey" => pubkey = args.next().ok_or("--pubkey needs a value")?,
            "--verify" => check_only = true,
            _ if arg.starts_with("--") => return Err(format!("unknown option {arg}")),
            _ if file.is_none() => file = Some(PathBuf::from(arg)),
            _ => return Err("one file at a time".into()),
        }
    }
    let file = file.ok_or("usage: qust-sign [--verify] <file> --version <x.y.z> [--pubkey <base64>]")?;
    let version = version.ok_or("--version is required: the signature names the version it is for")?;
    let signature_path = sig_path(&file);

    if !check_only {
        let signature = sign(&file, &version)?;
        std::fs::write(&signature_path, &signature).map_err(|error| format!("{}: {error}", signature_path.display()))?;
    }
    let data = std::fs::read(&file).map_err(|error| format!("{}: {error}", file.display()))?;
    let signature = std::fs::read_to_string(&signature_path).map_err(|error| format!("{}: {error}", signature_path.display()))?;
    qust_updater::verify(&data, signature.trim_end(), &pubkey, &version).map_err(|error| error.to_string())?;
    Ok(if check_only {
        format!("{}: valid signature for version {version}", file.display())
    } else {
        format!("{} signed for version {version}", signature_path.display())
    })
}

fn sig_path(file: &Path) -> PathBuf {
    let mut name = file.as_os_str().to_os_string();
    name.push(".sig");
    PathBuf::from(name)
}

fn sign(file: &Path, version: &str) -> Result<String, String> {
    let key = match std::env::var("TAURI_SIGNING_PRIVATE_KEY") {
        Ok(key) if Path::new(key.trim()).is_file() => read_key(Path::new(key.trim()))?,
        Ok(key) if !key.trim().is_empty() => key,
        _ => match std::env::var("TAURI_SIGNING_PRIVATE_KEY_PATH") {
            Ok(path) => read_key(Path::new(&path))?,
            Err(_) => return Err("set TAURI_SIGNING_PRIVATE_KEY or TAURI_SIGNING_PRIVATE_KEY_PATH".into()),
        },
    };
    // An unset password is an empty one; minisign would otherwise prompt for it.
    let password = std::env::var("TAURI_SIGNING_PRIVATE_KEY_PASSWORD").unwrap_or_default();

    let decoded = base64::engine::general_purpose::STANDARD
        .decode(key.trim())
        .map_err(|error| format!("the private key is not base64: {error}"))?;
    let decoded = String::from_utf8(decoded).map_err(|_| "the private key is not text")?;
    let secret = minisign::SecretKeyBox::from_string(&decoded)
        .and_then(|secret| secret.into_secret_key(Some(password)))
        .map_err(|error| format!("couldn't open the private key: {error}"))?;

    let name = file.file_name().ok_or("no file name")?.to_string_lossy().into_owned();
    let timestamp = SystemTime::now().duration_since(UNIX_EPOCH).map_err(|error| error.to_string())?.as_secs();
    let trusted = format!("timestamp:{timestamp}\tfile:{name}\tversion:{version}");
    let reader = std::fs::File::open(file).map_err(|error| format!("{}: {error}", file.display()))?;
    let signature = minisign::sign(None, &secret, std::io::BufReader::new(reader), Some(&trusted), Some("signature from tauri secret key"))
        .map_err(|error| format!("signing failed: {error}"))?;
    Ok(base64::engine::general_purpose::STANDARD.encode(signature.to_string()))
}

fn read_key(path: &Path) -> Result<String, String> {
    std::fs::read_to_string(path).map_err(|error| format!("{}: {error}", path.display()))
}
