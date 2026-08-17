//! Shared project-relative path validation for `nitrum.toml` keys.

use std::path::{Component, PathBuf};

/// Normalize an optional path string into a project-relative [`PathBuf`].
///
/// Empty / omitted → `None`. Absolute paths and `..` segments are rejected.
///
/// # Errors
///
/// Returns `Err` with a message that includes `field` when the path is invalid.
pub fn normalize_optional_relative_path(
    value: Option<String>,
    field: &str,
) -> Result<Option<PathBuf>, String> {
    let Some(raw) = value else {
        return Ok(None);
    };
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Ok(None);
    }
    let path = PathBuf::from(trimmed);
    if path.is_absolute() {
        return Err(format!("`{field}` must be a project-relative path"));
    }
    if path.components().any(|c| matches!(c, Component::ParentDir)) {
        return Err(format!("`{field}` must not contain `..` path segments"));
    }
    Ok(Some(path))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_becomes_none() {
        assert!(
            normalize_optional_relative_path(None, "x")
                .unwrap()
                .is_none()
        );
        assert!(
            normalize_optional_relative_path(Some(String::new()), "x")
                .unwrap()
                .is_none()
        );
        assert!(
            normalize_optional_relative_path(Some("  ".into()), "x")
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn accepts_relative() {
        let p = normalize_optional_relative_path(Some("infra/local-stack.yml".into()), "x")
            .unwrap()
            .unwrap();
        assert_eq!(p, PathBuf::from("infra/local-stack.yml"));
    }

    #[test]
    fn rejects_absolute_and_parent() {
        assert!(
            normalize_optional_relative_path(Some("/tmp/x".into()), "local.template")
                .unwrap_err()
                .contains("project-relative")
        );
        assert!(
            normalize_optional_relative_path(Some("../x".into()), "local.template")
                .unwrap_err()
                .contains("`..`")
        );
    }
}
