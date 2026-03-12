//! Symmetric encryption using a Data Encryption Key (DEK).
//!
//! On startup [`setup`] bootstraps the DEK:
//!
//! 1. Reads `NITRUM_KMS_KEY_ID` and `NITRUM_DYNAMODB_TABLE` from the
//!    environment.
//! 2. Tries to fetch an existing encrypted DEK from DynamoDB.
//! 3. If found, decrypts it via KMS using a Nitro attestation document
//!    (`kms:Decrypt` + `Recipient`).
//! 4. If not found, generates a fresh 32-byte DEK, wraps it with the KMS
//!    RSA-2048 key, and stores it with a conditional put so only one instance
//!    wins in a race.  Losers retry step 2.
//! 5. In dev builds (env vars absent) a random ephemeral DEK is used without
//!    touching KMS or DynamoDB.
//!
//! The returned [`Crypto`] instance encrypts/decrypts arbitrary bytes with
//! AES-256-GCM.  The output format is `nonce (12 B) || ciphertext`.

use aes_gcm::{
    Aes256Gcm, Key, Nonce,
    aead::{Aead, AeadCore, KeyInit, OsRng},
};
use anyhow::{Context, Result};

use crate::infra::{dynamodb, kms};

const ENV_KMS_KEY_ID: &str = "NITRUM_KMS_KEY_ID";
const ENV_DYNAMODB_TABLE: &str = "NITRUM_DYNAMODB_TABLE";
const ENV_DYNAMODB_ENDPOINT: &str = "NITRUM_DYNAMODB_ENDPOINT_URL";

// ── public API ────────────────────────────────────────────────────────────────

pub struct Crypto {
    dek: Vec<u8>,
}

impl Crypto {
    /// Encrypt `data` with AES-256-GCM.
    ///
    /// Returns `nonce (12 bytes) || ciphertext+tag`.
    pub fn encrypt(&self, data: &[u8]) -> Result<Vec<u8>> {
        let key = Key::<Aes256Gcm>::from_slice(&self.dek);
        let cipher = Aes256Gcm::new(key);
        let nonce = Aes256Gcm::generate_nonce(&mut OsRng);

        let ciphertext = cipher
            .encrypt(&nonce, data)
            .map_err(|e| anyhow::anyhow!("AES-GCM encrypt error: {e}"))?;

        let mut out = nonce.to_vec();
        out.extend_from_slice(&ciphertext);
        Ok(out)
    }

    /// Decrypt a blob produced by [`Crypto::encrypt`].
    pub fn decrypt(&self, data: &[u8]) -> Result<Vec<u8>> {
        const NONCE_LEN: usize = 12;
        anyhow::ensure!(data.len() > NONCE_LEN, "ciphertext too short");

        let (nonce_bytes, ciphertext) = data.split_at(NONCE_LEN);
        let key = Key::<Aes256Gcm>::from_slice(&self.dek);
        let cipher = Aes256Gcm::new(key);
        let nonce = Nonce::from_slice(nonce_bytes);

        cipher
            .decrypt(nonce, ciphertext)
            .map_err(|e| anyhow::anyhow!("AES-GCM decrypt error: {e}"))
    }
}

// ── setup ─────────────────────────────────────────────────────────────────────

/// Bootstrap the [`Crypto`] instance.  See module-level docs for the full flow.
///
/// When `NITRUM_DEV_RSA_PRIVATE_KEY` is set, DynamoDB can be used without
/// `NITRUM_KMS_KEY_ID` (DEK is encrypted/decrypted locally with that key).
pub async fn setup() -> Result<Crypto> {
    let kms_key_id = std::env::var(ENV_KMS_KEY_ID).ok();
    let dynamodb_table = std::env::var(ENV_DYNAMODB_TABLE).ok();
    let dev_key_set = std::env::var("NITRUM_DEV_RSA_PRIVATE_KEY").is_ok();

    let use_dynamo = match (
        kms_key_id.as_deref(),
        dynamodb_table.as_deref(),
        dev_key_set,
    ) {
        (Some(key_id), Some(table), _) => Some((key_id.to_string(), table.to_string())),
        (_, Some(table), true) => Some(("local".to_string(), table)),
        _ => None,
    };

    match use_dynamo {
        Some((key_id, table)) => setup_with_kms(&key_id, &table).await,
        _ => {
            tracing::warn!(
                "{ENV_KMS_KEY_ID} or {ENV_DYNAMODB_TABLE} not set — \
                 using an ephemeral DEK (dev mode, data is NOT persisted)"
            );
            Ok(Crypto { dek: random_key() })
        }
    }
}

// ── internal helpers ──────────────────────────────────────────────────────────

async fn setup_with_kms(key_id: &str, table: &str) -> Result<Crypto> {
    // The AWS SDK's default provider chain resolves region and credentials
    // from env vars, IMDSv2, and ECS metadata — all of which work inside
    // the enclave because vsock-proxy allowlists 169.254.169.254.
    let aws_cfg = aws_config::from_env().load().await;

    let kms_client = aws_sdk_kms::Client::new(&aws_cfg);

    // Optional custom endpoint for DynamoDB Local or other compatible backends.
    let dynamo_client = match std::env::var(ENV_DYNAMODB_ENDPOINT) {
        Ok(url) => {
            let conf = aws_sdk_dynamodb::config::Builder::from(&aws_cfg)
                .endpoint_url(url)
                .build();
            aws_sdk_dynamodb::Client::from_conf(conf)
        }
        Err(_) => aws_sdk_dynamodb::Client::new(&aws_cfg),
    };

    // Loop handles the race where two instances start simultaneously.
    loop {
        if let Some(encrypted_dek) = dynamodb::retrieve_dek(&dynamo_client, table)
            .await
            .context("failed to retrieve DEK from DynamoDB")?
        {
            tracing::info!("DEK found in DynamoDB, decrypting with KMS");
            let dek = kms::decrypt_with_attestation(&kms_client, key_id, &encrypted_dek)
                .await
                .context("failed to decrypt DEK with KMS")?;
            return Ok(Crypto { dek });
        }

        // No DEK yet — generate one and race to store it.
        tracing::info!("no DEK in DynamoDB, generating a new one");
        let dek = random_key();

        let encrypted_dek = kms::encrypt(&kms_client, key_id, &dek)
            .await
            .context("failed to encrypt DEK with KMS")?;

        let stored = dynamodb::store_dek(&dynamo_client, table, &encrypted_dek)
            .await
            .context("failed to store DEK in DynamoDB")?;

        if stored {
            tracing::info!("DEK stored successfully");
            return Ok(Crypto { dek });
        }

        // Another instance won the race; read its DEK on the next iteration.
        tracing::info!("lost DEK store race, retrying read");
    }
}

fn random_key() -> Vec<u8> {
    use rand::RngCore as _;
    let mut key = vec![0u8; 32];
    rand::thread_rng().fill_bytes(&mut key);
    key
}
