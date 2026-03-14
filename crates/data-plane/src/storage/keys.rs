/// KMS-encrypted Data Encryption Key.
pub const DEK_OBJECT_KEY: &str = "dek";

/// TLS certificate (DER).
pub const CERTIFICATE_OBJECT_KEY: &str = "cert";

/// TLS private key (DER).
pub const CERTIFICATE_PRIVATE_KEY_OBJECT_KEY: &str = "cert_key";

/// Leader lock key for crypto (DEK creation).
pub const CRYPTO_LEADER_KEY: &str = "lock:crypto";

/// Leader lock key for ACME certificate provisioning.
pub const ACME_LEADER_KEY: &str = "lock:acme";

/// ACME account credentials (JSON, persisted for reuse).
pub const ACME_ACCOUNT_OBJECT_KEY: &str = "acme_account";

/// Object key prefix for ACME HTTP-01 challenge tokens. Full key: `{ACME_CHALLENGE_KEY_PREFIX}{token}`.
pub const ACME_CHALLENGE_KEY_PREFIX: &str = "acme_challenge:";

/// Returns the storage key for an ACME HTTP-01 challenge token.
pub fn acme_challenge_key(token: &str) -> String {
    format!("{ACME_CHALLENGE_KEY_PREFIX}{token}")
}
