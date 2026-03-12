//! KMS helpers for DEK envelope encryption.
//!
//! `encrypt`   – wraps a plaintext blob with the RSA-2048 KMS key.
//! `decrypt_with_attestation` – unwraps using a Nitro attestation document so
//!   that KMS releases the plaintext only to a verified enclave instance.
//!
//! In non-enclave (dev) builds the attestation step is skipped and decryption
//! is performed locally using an RSA private key from `NITRUM_DEV_RSA_PRIVATE_KEY`.

use anyhow::{Context, Result};
use aws_sdk_kms::primitives::Blob;
use aws_sdk_kms::types::EncryptionAlgorithmSpec;

// ── encrypt ──────────────────────────────────────────────────────────────────

/// Encrypt `plaintext` under the RSA-2048 KMS key identified by `key_id`.
/// Uses RSAES_OAEP_SHA_256 padding.
///
/// In non-enclave (dev) builds, if `NITRUM_DEV_RSA_PRIVATE_KEY` is set the
/// public key is derived from it and encryption is performed locally without
/// calling KMS at all.
#[cfg(feature = "enclave")]
pub async fn encrypt(
    client: &aws_sdk_kms::Client,
    key_id: &str,
    plaintext: &[u8],
) -> Result<Vec<u8>> {
    let resp = client
        .encrypt()
        .key_id(key_id)
        .plaintext(Blob::new(plaintext))
        .encryption_algorithm(EncryptionAlgorithmSpec::RsaesOaepSha256)
        .send()
        .await
        .context("KMS Encrypt failed")?;

    resp.ciphertext_blob()
        .map(|b| b.as_ref().to_vec())
        .context("KMS Encrypt returned no ciphertext")
}

/// Dev / non-enclave fallback: encrypt locally with the public key derived
/// from `NITRUM_DEV_RSA_PRIVATE_KEY`, falling back to KMS if the variable is
/// absent.
#[cfg(not(feature = "enclave"))]
pub async fn encrypt(
    client: &aws_sdk_kms::Client,
    key_id: &str,
    plaintext: &[u8],
) -> Result<Vec<u8>> {
    use rsa::{Oaep, RsaPublicKey, pkcs1::DecodeRsaPrivateKey as _, pkcs8::DecodePrivateKey as _};
    use sha2::Sha256;

    if let Ok(pem) = std::env::var("NITRUM_DEV_RSA_PRIVATE_KEY") {
        let private_key = rsa::RsaPrivateKey::from_pkcs8_pem(&pem)
            .or_else(|_| rsa::RsaPrivateKey::from_pkcs1_pem(&pem))
            .context("failed to parse RSA private key from NITRUM_DEV_RSA_PRIVATE_KEY")?;

        let public_key = RsaPublicKey::from(&private_key);
        let mut rng = rand::thread_rng();
        return public_key
            .encrypt(&mut rng, Oaep::new::<Sha256>(), plaintext)
            .context("local RSA-OAEP-SHA256 encrypt failed");
    }

    // No dev key — fall back to KMS.
    let resp = client
        .encrypt()
        .key_id(key_id)
        .plaintext(Blob::new(plaintext))
        .encryption_algorithm(EncryptionAlgorithmSpec::RsaesOaepSha256)
        .send()
        .await
        .context("KMS Encrypt failed")?;

    resp.ciphertext_blob()
        .map(|b| b.as_ref().to_vec())
        .context("KMS Encrypt returned no ciphertext")
}

// ── decrypt_with_attestation (enclave) ───────────────────────────────────────

/// Decrypt `ciphertext` using a Nitro attestation-based `Recipient`.
///
/// The flow:
/// 1. Generate an ephemeral RSA-2048 keypair.
/// 2. Embed the public key in a fresh NSM attestation document.
/// 3. Send `kms:Decrypt` with the attestation doc as `Recipient`.
/// 4. KMS verifies the PCR measurements, decrypts the ciphertext, and
///    re-encrypts the plaintext with the ephemeral public key.
/// 5. Decrypt that envelope locally with the ephemeral private key.
#[cfg(feature = "enclave")]
pub async fn decrypt_with_attestation(
    client: &aws_sdk_kms::Client,
    key_id: &str,
    ciphertext: &[u8],
) -> Result<Vec<u8>> {
    use aws_sdk_kms::types::{KeyEncryptionMechanism, RecipientInfo};
    use rsa::{Oaep, RsaPrivateKey, pkcs8::EncodePublicKey as _};
    use sha2::Sha256;

    // 1. Ephemeral RSA-2048 keypair (lives only for this call).
    let mut rng = rand::thread_rng();
    let private_key =
        RsaPrivateKey::new(&mut rng, 2048).context("failed to generate ephemeral RSA key")?;
    let public_key_der = rsa::RsaPublicKey::from(&private_key)
        .to_public_key_der()
        .context("failed to DER-encode ephemeral public key")?
        .to_vec();

    // 2. Attestation document with the ephemeral public key embedded.
    let attestation_doc = crate::attestation::get_attestation_doc(None, Some(public_key_der), None)
        .map_err(|e| anyhow::anyhow!("attestation failed: {e}"))?;

    // 3. KMS Decrypt with Recipient.
    let recipient = RecipientInfo::builder()
        .key_encryption_algorithm(KeyEncryptionMechanism::RsaesOaepSha256)
        .attestation_document(Blob::new(attestation_doc))
        .build();

    let resp = client
        .decrypt()
        .key_id(key_id)
        .ciphertext_blob(Blob::new(ciphertext))
        .encryption_algorithm(EncryptionAlgorithmSpec::RsaesOaepSha256)
        .recipient(recipient)
        .send()
        .await
        .context("KMS Decrypt (with attestation) failed")?;

    // 4. KMS returns the plaintext encrypted for our ephemeral key.
    let ciphertext_for_recipient = resp
        .ciphertext_for_recipient()
        .context("KMS did not return ciphertext_for_recipient")?
        .as_ref();

    // 5. Decrypt with ephemeral private key using OAEP-SHA256.
    let plaintext = private_key
        .decrypt(Oaep::new::<Sha256>(), ciphertext_for_recipient)
        .context("failed to decrypt ciphertext_for_recipient with ephemeral key")?;

    Ok(plaintext)
}

/// Dev / non-enclave fallback: decrypt locally with an RSA private key
/// supplied via the `NITRUM_DEV_RSA_PRIVATE_KEY` environment variable.
///
/// The variable must contain a PEM-encoded RSA private key (PKCS#8
/// `-----BEGIN PRIVATE KEY-----` or PKCS#1 `-----BEGIN RSA PRIVATE KEY-----`).
/// Generate a matching key pair with:
///
/// ```sh
/// # private key
/// openssl genpkey -algorithm RSA -pkeyopt rsa_keygen_bits:2048 \
///   -out dev_private.pem
/// # public key (for encrypting the DEK with `encrypt()`)
/// openssl pkey -in dev_private.pem -pubout -out dev_public.pem
/// ```
///
/// Then set the env var before running the binary:
/// ```sh
/// export NITRUM_DEV_RSA_PRIVATE_KEY="$(cat dev_private.pem)"
/// ```
#[cfg(not(feature = "enclave"))]
pub async fn decrypt_with_attestation(
    _client: &aws_sdk_kms::Client,
    _key_id: &str,
    ciphertext: &[u8],
) -> Result<Vec<u8>> {
    use rsa::{Oaep, pkcs1::DecodeRsaPrivateKey as _, pkcs8::DecodePrivateKey as _};
    use sha2::Sha256;

    let pem = std::env::var("NITRUM_DEV_RSA_PRIVATE_KEY")
        .context("NITRUM_DEV_RSA_PRIVATE_KEY not set (required for dev-mode decrypt)")?;

    // Accept both PKCS#8 ("BEGIN PRIVATE KEY") and PKCS#1 ("BEGIN RSA PRIVATE KEY").
    let private_key = rsa::RsaPrivateKey::from_pkcs8_pem(&pem)
        .or_else(|_| rsa::RsaPrivateKey::from_pkcs1_pem(&pem))
        .context("failed to parse RSA private key from NITRUM_DEV_RSA_PRIVATE_KEY")?;

    private_key
        .decrypt(Oaep::new::<Sha256>(), ciphertext)
        .context("RSA-OAEP-SHA256 decrypt failed")
}
