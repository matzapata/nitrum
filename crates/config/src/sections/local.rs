//! `[local]` in `nitrum.toml` — Compose-only knobs (ignored by `nitrum cloud`).

use super::path::normalize_optional_relative_path;
use std::path::PathBuf;

#[derive(Debug, Clone, thiserror::Error)]
#[error("{0}")]
pub struct LocalError(pub String);

/// `[local]` section: optional ejected Compose template.
#[derive(Debug, Clone, Default, serde::Serialize)]
pub struct Local {
    /// Project-relative Compose YAML. Empty / omitted → CLI rewrites `.nitrum/local-stack.yml`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub template: Option<PathBuf>,
}

impl Local {
    fn try_from_raw(raw: LocalRaw) -> Result<Self, LocalError> {
        let template =
            normalize_optional_relative_path(raw.template, "local.template").map_err(LocalError)?;
        Ok(Self { template })
    }
}

#[derive(serde::Deserialize)]
struct LocalRaw {
    #[serde(default)]
    template: Option<String>,
}

impl<'de> serde::Deserialize<'de> for Local {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let raw = LocalRaw::deserialize(deserializer)?;
        Self::try_from_raw(raw).map_err(serde::de::Error::custom)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_empty() {
        let local = Local::default();
        assert!(local.template.is_none());
    }

    #[test]
    fn template_path_passthrough() {
        let local: Local = toml::from_str(r#"template = "infra/local-stack.yml""#)
            .expect("relative template path");
        assert_eq!(
            local.template.as_deref(),
            Some(std::path::Path::new("infra/local-stack.yml"))
        );
    }

    #[test]
    fn empty_template_becomes_none() {
        let local: Local = toml::from_str(r#"template = """#).expect("empty template");
        assert!(local.template.is_none());
    }

    #[test]
    fn rejects_absolute_template() {
        let err =
            toml::from_str::<Local>(r#"template = "/tmp/stack.yml""#).expect_err("absolute path");
        assert!(err.to_string().contains("project-relative"));
    }

    #[test]
    fn rejects_parent_dir_template() {
        let err = toml::from_str::<Local>(r#"template = "../stack.yml""#).expect_err("parent dir");
        assert!(err.to_string().contains("`..`"));
    }

    #[test]
    fn omits_empty_optional_fields_on_serialize() {
        let toml = toml::to_string(&Local::default()).expect("serialize");
        assert!(!toml.contains("template"));
    }
}
