use std::{env, fs, path::Path};

use anyhow::{Context, Result, bail};
use base64::{Engine as _, engine::general_purpose::STANDARD};
use minisign_verify::{PublicKey, Signature};

fn main() -> Result<()> {
    let mut args = env::args_os().skip(1);
    let artifact_path = args
        .next()
        .context("usage: verify_updater_signature <artifact> <signature> <public-key-file>")?;
    let signature_path = args
        .next()
        .context("usage: verify_updater_signature <artifact> <signature> <public-key-file>")?;
    let public_key_path = args
        .next()
        .context("usage: verify_updater_signature <artifact> <signature> <public-key-file>")?;
    if args.next().is_some() {
        bail!("usage: verify_updater_signature <artifact> <signature> <public-key-file>");
    }

    let public_key = PublicKey::from_file(Path::new(&public_key_path))
        .context("failed to parse updater public key")?;
    let encoded_signature =
        fs::read_to_string(&signature_path).context("failed to read updater signature envelope")?;
    let signature_text = STANDARD
        .decode(encoded_signature.trim())
        .context("failed to decode updater signature envelope")?;
    let signature_text = String::from_utf8(signature_text)
        .context("decoded updater signature is not valid UTF-8")?;
    let signature =
        Signature::decode(&signature_text).context("failed to parse updater signature")?;
    let artifact = fs::read(&artifact_path).context("failed to read updater artifact")?;

    public_key
        .verify(&artifact, &signature, false)
        .context("updater signature does not match the configured public key")?;
    println!("updater-signature=valid");
    Ok(())
}
