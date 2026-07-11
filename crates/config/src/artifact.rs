//! EIF version labels and S3 object key conventions shared by CLI deploy and control-plane runtime.

use thiserror::Error;

/// Length of the EIF version label (first hex chars of the EIF SHA-256 from deploy).
pub const EIF_VERSION_LABEL_LEN: usize = 12;

#[derive(Debug, Error)]
#[error("{message}")]
pub struct ArtifactValidationError {
    /// Human-readable validation failure reason.
    message: String,
}

impl ArtifactValidationError {
    fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

/// S3 object key for a deployed EIF (`{version_label}.eif`).
#[must_use]
pub fn eif_s3_key(version_label: &str) -> String {
    format!("{version_label}.eif")
}

/// Validates an EIF version label for use in S3 keys and CLI flags (no path separators or `..`).
///
/// # Errors
///
/// Returns an error when the label is empty or contains path-like segments.
pub fn validate_eif_version_label(label: &str) -> Result<&str, ArtifactValidationError> {
    let label = label.trim();
    if label.is_empty() {
        return Err(ArtifactValidationError::new(
            "EIF version label must not be empty",
        ));
    }
    if label.contains('/') || label.contains('\\') || label.contains("..") {
        return Err(ArtifactValidationError::new(
            "EIF version label must not contain path separators or '..'",
        ));
    }
    Ok(label)
}

/// Derives the deploy version label from a full EIF SHA-256 hex digest.
#[must_use]
pub fn eif_version_label_from_hash(full_hash: &str) -> String {
    full_hash.chars().take(EIF_VERSION_LABEL_LEN).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn eif_s3_key_format() {
        assert_eq!(eif_s3_key("abc123"), "abc123.eif");
    }

    #[test]
    fn validate_rejects_path_traversal() {
        assert!(validate_eif_version_label("../etc").is_err());
        assert!(validate_eif_version_label("").is_err());
        assert_eq!(
            validate_eif_version_label("  deadbeef  ").unwrap(),
            "deadbeef"
        );
    }

    #[test]
    fn version_label_from_hash() {
        let hash = "a".repeat(64);
        assert_eq!(eif_version_label_from_hash(&hash), "a".repeat(12));
    }
}
