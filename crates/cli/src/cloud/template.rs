//! Bundled / ejected CloudFormation template helpers.

use anyhow::{Context, Result, bail};
use std::path::Path;

/// Marker embedded in [`bundled_cloud_stack_template`] and compared on custom deploys.
pub const BUNDLED_CLOUD_TEMPLATE_VERSION: &str = "0.3.0";

/// CFN `TemplateBody` hard limit (bytes). Larger templates use S3 `TemplateURL`.
pub const CFN_TEMPLATE_BODY_MAX_BYTES: usize = 51_200;

/// Object key for oversized templates uploaded to the project EIF bucket.
pub const CFN_TEMPLATE_S3_KEY: &str = "cloudformation/stack.yml";

/// How the CLI will pass the template to CloudFormation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StackTemplateKind {
    /// Inline `TemplateBody` (≤ [`CFN_TEMPLATE_BODY_MAX_BYTES`]).
    Body,
    /// S3 `TemplateURL` after upload.
    Url,
}

/// Template payload for `CreateStack` / `UpdateStack`.
#[derive(Debug, Clone)]
pub enum StackTemplate {
    /// Inline YAML body.
    Body(String),
    /// HTTPS S3 URL CloudFormation can fetch.
    Url(String),
}

/// Bundled `cloud-stack.yml` compiled into the CLI.
#[must_use]
pub const fn bundled_cloud_stack_template() -> &'static str {
    include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/template/cloud-stack.yml"
    ))
}

/// Choose Body vs URL from YAML byte length.
#[must_use]
pub const fn stack_template_kind(byte_len: usize) -> StackTemplateKind {
    if byte_len <= CFN_TEMPLATE_BODY_MAX_BYTES {
        StackTemplateKind::Body
    } else {
        StackTemplateKind::Url
    }
}

/// Parse `# nitrum-template-version: …` from template YAML (first matching line).
#[must_use]
pub fn parse_template_version(yaml: &str) -> Option<&str> {
    for line in yaml.lines() {
        let trimmed = line.trim();
        if let Some(rest) = trimmed.strip_prefix("# nitrum-template-version:") {
            let v = rest.trim();
            if !v.is_empty() {
                return Some(v);
            }
        }
    }
    None
}

/// Warn when an ejected template's version marker is missing or does not match the bundle.
pub fn warn_template_skew(yaml: &str, is_custom: bool, kind: &str, eject_cmd: &str) {
    if !is_custom {
        return;
    }
    match parse_template_version(yaml) {
        Some(v) if v == BUNDLED_CLOUD_TEMPLATE_VERSION => {}
        Some(v) => {
            eprintln!(
                "warning: ejected {kind} template version `{v}` does not match the \
                 CLI bundled template version `{BUNDLED_CLOUD_TEMPLATE_VERSION}`. Re-run \
                 `{eject_cmd} --force` to refresh (overwrites local edits), or merge \
                 upstream changes manually."
            );
        }
        None => {
            eprintln!(
                "warning: ejected {kind} template has no `# nitrum-template-version:` \
                 marker (CLI bundled version is `{BUNDLED_CLOUD_TEMPLATE_VERSION}`). Re-run \
                 `{eject_cmd} --force` to refresh, or add the marker after merging."
            );
        }
    }
}

/// Load the CloudFormation YAML: project file when `[cloud].template` is set, else the bundle.
///
/// Returns `(yaml, is_custom)`.
///
/// # Errors
///
/// Returns an error when the configured template path cannot be read.
pub fn load_cloud_template(project_root: &Path, template: Option<&Path>) -> Result<(String, bool)> {
    match template {
        Some(rel) => {
            let path = project_root.join(rel);
            let yaml = std::fs::read_to_string(&path)
                .with_context(|| format!("read CloudFormation template {}", path.display()))?;
            Ok((yaml, true))
        }
        None => Ok((bundled_cloud_stack_template().to_string(), false)),
    }
}

/// Write `contents` to `dest`. Refuses if the file exists unless `force`.
///
/// # Errors
///
/// Returns an error when the destination exists without `force`, or when creating
/// directories / writing the file fails.
pub fn write_bundled_template(dest: &Path, force: bool, contents: &str) -> Result<()> {
    if dest.exists() && !force {
        bail!(
            "{} already exists; pass --force to overwrite",
            dest.display()
        );
    }
    if let Some(parent) = dest.parent()
        && !parent.as_os_str().is_empty()
    {
        std::fs::create_dir_all(parent).with_context(|| format!("create {}", parent.display()))?;
    }
    std::fs::write(dest, contents).with_context(|| format!("write {}", dest.display()))?;
    Ok(())
}

/// Write the bundled CloudFormation template to `dest`.
///
/// # Errors
///
/// See [`write_bundled_template`].
pub fn eject_cloud_template(dest: &Path, force: bool) -> Result<()> {
    write_bundled_template(dest, force, bundled_cloud_stack_template())
}

/// HTTPS URL CloudFormation accepts as `TemplateURL` for an object in `bucket`.
#[must_use]
pub fn s3_template_url(region: &str, bucket: &str, key: &str) -> String {
    if region == "us-east-1" {
        format!("https://s3.amazonaws.com/{bucket}/{key}")
    } else {
        format!("https://s3.{region}.amazonaws.com/{bucket}/{key}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn bundled_has_version_marker() {
        let yaml = bundled_cloud_stack_template();
        assert_eq!(
            parse_template_version(yaml),
            Some(BUNDLED_CLOUD_TEMPLATE_VERSION)
        );
    }

    #[test]
    fn body_under_limit_url_over() {
        assert_eq!(
            stack_template_kind(CFN_TEMPLATE_BODY_MAX_BYTES),
            StackTemplateKind::Body
        );
        assert_eq!(
            stack_template_kind(CFN_TEMPLATE_BODY_MAX_BYTES + 1),
            StackTemplateKind::Url
        );
        assert_eq!(
            stack_template_kind(bundled_cloud_stack_template().len()),
            StackTemplateKind::Body
        );
    }

    #[test]
    fn parse_version_skips_noise() {
        let yaml = "AWSTemplateFormatVersion: '2010-09-09'\n# nitrum-template-version: 0.3.0\n";
        assert_eq!(parse_template_version(yaml), Some("0.3.0"));
        assert!(parse_template_version("no marker").is_none());
    }

    #[test]
    fn eject_refuses_without_force() {
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("time")
            .as_nanos();
        let dir = std::env::temp_dir().join(format!("nitrum-eject-{stamp}"));
        std::fs::create_dir_all(&dir).expect("mkdir");
        let dest = dir.join("cloud-stack.yml");
        eject_cloud_template(&dest, false).expect("first write");
        let err = eject_cloud_template(&dest, false).expect_err("second write");
        assert!(err.to_string().contains("--force"));
        eject_cloud_template(&dest, true).expect("force overwrite");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn load_custom_template() {
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("time")
            .as_nanos();
        let root = std::env::temp_dir().join(format!("nitrum-load-{stamp}"));
        let infra = root.join("infra");
        std::fs::create_dir_all(&infra).expect("mkdir");
        let path = infra.join("cloud-stack.yml");
        std::fs::write(&path, "# nitrum-template-version: 0.2.0\n").expect("write");
        let (yaml, custom) =
            load_cloud_template(&root, Some(std::path::Path::new("infra/cloud-stack.yml")))
                .expect("load");
        assert!(custom);
        assert_eq!(parse_template_version(&yaml), Some("0.2.0"));
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn s3_url_us_east_1_vs_other() {
        assert_eq!(
            s3_template_url("us-east-1", "b", "cloudformation/stack.yml"),
            "https://s3.amazonaws.com/b/cloudformation/stack.yml"
        );
        assert_eq!(
            s3_template_url("eu-west-1", "b", "cloudformation/stack.yml"),
            "https://s3.eu-west-1.amazonaws.com/b/cloudformation/stack.yml"
        );
    }
}
