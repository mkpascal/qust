// Signature checks, as tauri-plugin-updater 2.14 does them (updater.rs, `verify_signature`): the
// public key and the signature are base64 of minisign's text files, the signature may be prehashed
// (what minisign and the Tauri CLI make) or legacy, and the trusted comment, which the signature
// covers, says which version was signed (`timestamp:…\tfile:…\tversion:1.2.3`).
//
// One difference: the Tauri updater only compares that version when one is there
// (`requireSignedVersion` is off by default); this one requires it. Every release since the
// updater exists carries it, and without it an old, validly signed installer could be passed off
// as a newer version (a forced downgrade).

use crate::Error;
use base64::Engine;
use minisign_verify::{PublicKey, Signature};

pub fn verify(data: &[u8], signature: &str, pubkey: &str, announced_version: &str) -> Result<(), Error> {
    let fail = |message: String| Error::Signature(message);
    let key = PublicKey::decode(&decode(pubkey).map_err(|error| fail(format!("public key: {error}")))?)
        .map_err(|error| fail(format!("public key: {error}")))?;
    let signature = Signature::decode(&decode(signature).map_err(fail)?).map_err(|error| fail(error.to_string()))?;
    key.verify(data, &signature, true).map_err(|error| fail(error.to_string()))?;

    // Only now is the trusted comment worth reading: verify() checked the signature over it.
    let signed = signed_version(signature.trusted_comment()).ok_or_else(|| fail("the signature doesn't say which version it is for".into()))?;
    let matches = match (crate::manifest::parse_version(signed), crate::manifest::parse_version(announced_version)) {
        (Ok(signed), Ok(announced)) => signed == announced,
        _ => signed == announced_version,
    };
    if !matches {
        return Err(fail(format!("it is for version {signed}, not {announced_version}")));
    }
    Ok(())
}

fn decode(base64: &str) -> Result<String, String> {
    let bytes = base64::engine::general_purpose::STANDARD.decode(base64).map_err(|error| error.to_string())?;
    String::from_utf8(bytes).map_err(|_| "not UTF-8 text".to_string())
}

pub(crate) fn signed_version(trusted_comment: &str) -> Option<&str> {
    trusted_comment.split('\t').find_map(|field| field.strip_prefix("version:"))
}
