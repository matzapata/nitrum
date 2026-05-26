//! Redact sensitive values before they reach log sinks.

use regex::Regex;
use std::sync::LazyLock;

/// Placeholder written when a value is scrubbed.
pub const REDACTED_PLACEHOLDER: &str = "[REDACTED]";

const REDACTED: &str = REDACTED_PLACEHOLDER;

static AUTH_HEADER: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)(authorization\s*:\s*)([^\r\n]+)").expect("authorization header regex")
});

static COOKIE_HEADER: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)(set-cookie\s*:\s*)([^\r\n]+)").expect("set-cookie header regex")
});

static COOKIE_KV: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)(cookie\s*:\s*)([^\r\n]+)").expect("cookie header regex"));

static PRIVATE_KEY_BLOCK: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"-----BEGIN [A-Z ]*PRIVATE KEY-----[\s\S]*?-----END [A-Z ]*PRIVATE KEY-----")
        .expect("pem private key regex")
});

/// Field names whose string values are always redacted.
const SENSITIVE_FIELD_NAMES: &[&str] = &[
    "authorization",
    "cookie",
    "set-cookie",
    "plaintext",
    "plaintext_dek",
    "dek",
    "private_key",
    "tls_private_key",
    "ciphertext",
    "ciphertext_blob",
    "securestring",
    "ssm_value",
    "user_env",
    "password",
    "secret",
];

/// Returns `true` when the field name should not be logged verbatim.
#[must_use]
pub fn is_sensitive_field_name(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    SENSITIVE_FIELD_NAMES.iter().any(|s| lower == *s)
}

/// Scrubs sensitive patterns and PEM material from a single string (messages and field values).
#[must_use]
pub fn redact_str(input: &str) -> String {
    if input.is_empty() {
        return String::new();
    }

    if is_sensitive_field_name(input) {
        return REDACTED.to_string();
    }

    let mut out = input.to_string();
    out = AUTH_HEADER
        .replace_all(&out, |caps: &regex::Captures| {
            format!("{}{REDACTED}", &caps[1])
        })
        .into_owned();
    out = COOKIE_HEADER
        .replace_all(&out, |caps: &regex::Captures| {
            format!("{}{REDACTED}", &caps[1])
        })
        .into_owned();
    out = COOKIE_KV
        .replace_all(&out, |caps: &regex::Captures| {
            format!("{}{REDACTED}", &caps[1])
        })
        .into_owned();
    out = PRIVATE_KEY_BLOCK.replace_all(&out, REDACTED).into_owned();

    if looks_like_kms_ciphertext_blob(&out) {
        return REDACTED.to_string();
    }

    out
}

/// Redacts a structured tracing field value using the field name and string content.
#[must_use]
pub fn redact_field(name: &str, value: &str) -> String {
    if is_sensitive_field_name(name) {
        REDACTED.to_string()
    } else {
        redact_str(value)
    }
}

/// Heuristic: long base64-ish blobs often wrap KMS ciphertext in debug paths.
fn looks_like_kms_ciphertext_blob(s: &str) -> bool {
    let trimmed = s.trim();
    if trimmed.len() < 128 {
        return false;
    }
    let alnum_slash_plus = trimmed
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == '+' || *c == '/' || *c == '=')
        .count();
    alnum_slash_plus * 10 >= trimmed.len() * 9
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn redacts_authorization_header() {
        let input = "request headers: Authorization: Bearer super-secret-token";
        let out = redact_str(input);
        assert!(!out.contains("super-secret-token"));
        assert!(out.contains(REDACTED));
    }

    #[test]
    fn redacts_cookie_headers() {
        let set_cookie = "Set-Cookie: session=abc123; HttpOnly; Secure";
        let out = redact_str(set_cookie);
        assert!(!out.contains("abc123"));
        assert!(out.contains(REDACTED));

        let cookie = "Cookie: sid=deadbeef";
        let out2 = redact_str(cookie);
        assert!(!out2.contains("deadbeef"));
        assert!(out2.contains(REDACTED));
    }

    #[test]
    fn redacts_private_key_and_kms_like_blob() {
        let pem = "-----BEGIN PRIVATE KEY-----\nMIIEvQIBADANBgkqhkiG9w0BAQEFAASCBKcwggSjAgEAAoIBAQC\n-----END PRIVATE KEY-----";
        let out = redact_str(pem);
        assert!(!out.contains("MIIEvQIBADAN"));
        assert!(out.contains(REDACTED));

        let blob = "a".repeat(200);
        assert_eq!(redact_str(&blob), REDACTED);
    }

    #[test]
    fn redacts_sensitive_field_names() {
        assert_eq!(redact_str("plaintext_dek"), REDACTED);
        assert_eq!(redact_str("ciphertext"), REDACTED);
    }

    #[test]
    fn redacts_ssm_value_field_payload() {
        let secret = "postgresql://user:password@db.example.com/app";
        assert_eq!(redact_field("ssm_value", secret), REDACTED);
        assert_eq!(redact_field("securestring", secret), REDACTED);
        assert!(!redact_field("ssm_value", secret).contains("password"));
    }

    #[test]
    fn redacts_user_env_debug_dump() {
        let dump = r#"{"DATABASE_URL": "postgres://secret", "API_KEY": "sk-live-abc123"}"#;
        assert_eq!(redact_field("user_env", dump), REDACTED);
        assert!(!redact_field("user_env", dump).contains("sk-live"));
    }
}
