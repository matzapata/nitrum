/// Validated Docker image reference (for example `registry/repo:tag`).
#[derive(Debug, Clone, PartialEq, Eq, Hash, serde::Serialize)]
#[serde(transparent)]
pub struct DockerImageRef(String);

#[derive(Debug, Clone, thiserror::Error)]
#[error("{0}")]
pub struct DockerImageRefError(String);

/// Env var that overrides `[runtime].data_plane` when loading `nitrum.toml`.
pub const ENV_RUNTIME_DATA_PLANE_IMAGE: &str = "NITRUM_RUNTIME_DATA_PLANE_IMAGE";
/// Env var that overrides `[runtime].control_plane` when loading `nitrum.toml`.
pub const ENV_RUNTIME_CONTROL_PLANE_IMAGE: &str = "NITRUM_RUNTIME_CONTROL_PLANE_IMAGE";
/// Env var that overrides `[runtime].nitro_cli` when loading `nitrum.toml`.
pub const ENV_RUNTIME_NITRO_CLI_IMAGE: &str = "NITRUM_RUNTIME_NITRO_CLI_IMAGE";

impl DockerImageRef {
    /// Validates and constructs a Docker image reference.
    ///
    /// # Errors
    ///
    /// Returns an error when `value` is empty, contains whitespace, or is malformed.
    pub fn try_new(value: &str) -> Result<Self, DockerImageRefError> {
        let s = value.trim();
        if s.is_empty() {
            return Err(DockerImageRefError(
                "`docker image` must not be empty (expected a Docker image, e.g. registry/repo:tag)"
                    .to_string(),
            ));
        }
        if s != value {
            return Err(DockerImageRefError(
                "`docker image` must not have leading or trailing whitespace".to_string(),
            ));
        }
        if s.contains(char::is_whitespace) {
            return Err(DockerImageRefError(
                "`docker image` must not contain whitespace".to_string(),
            ));
        }
        if !s.contains('/') {
            return Err(DockerImageRefError(
                "`docker image` should look like `registry/repo:tag` or `user/repo:tag` (must contain `/`)"
                    .to_string(),
            ));
        }
        Ok(Self(value.to_string()))
    }

    /// Borrow the validated reference as UTF-8.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Append `-{suffix}` to the image tag.
    ///
    /// Splits on the last `:` that appears after the last `/`, so
    /// `registry:port/repo:tag` keeps the registry port intact. When the
    /// reference has no tag, `latest` is assumed before suffixing
    /// (e.g. `registry/repo` → `registry/repo:latest-local`).
    ///
    /// Digest references (`repo@sha256:…`) drop the digest and use
    /// `latest-{suffix}` on the repository name, so `nitrum local` can still
    /// resolve a floating local tag after `nitrum init` pinned digests.
    ///
    /// # Panics
    ///
    /// Panics only if the resulting string somehow fails [`Self::try_new`]; that
    /// cannot happen for a previously-validated reference plus a simple suffix.
    #[must_use]
    pub fn with_tag_suffix(&self, suffix: &str) -> Self {
        let s = self.0.as_str();
        if let Some(at) = s.find('@') {
            let repo = &s[..at];
            return Self::try_new(&format!("{repo}:latest-{suffix}"))
                .expect("suffixing a validated image ref yields a valid image ref");
        }
        let slash = s.rfind('/').expect("validated image refs contain `/`");
        let (repo, tag) = s[slash + 1..].rfind(':').map_or((s, "latest"), |rel| {
            let colon = slash + 1 + rel;
            (&s[..colon], &s[colon + 1..])
        });
        Self::try_new(&format!("{repo}:{tag}-{suffix}"))
            .expect("suffixing a validated image ref yields a valid image ref")
    }
}

impl std::ops::Deref for DockerImageRef {
    type Target = str;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl std::fmt::Display for DockerImageRef {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::str::FromStr for DockerImageRef {
    type Err = DockerImageRefError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        Self::try_new(value)
    }
}

impl TryFrom<String> for DockerImageRef {
    type Error = DockerImageRefError;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::try_new(&value)
    }
}

impl<'de> serde::Deserialize<'de> for DockerImageRef {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        Self::try_new(&value).map_err(serde::de::Error::custom)
    }
}

fn default_data_plane_image() -> DockerImageRef {
    DockerImageRef::try_new("ghcr.io/matzapata/nitrum/data-plane:latest")
        .expect("default data-plane image is valid")
}

fn default_control_plane_image() -> DockerImageRef {
    DockerImageRef::try_new("ghcr.io/matzapata/nitrum/control-plane:latest")
        .expect("default control-plane image is valid")
}

fn default_nitro_cli_image() -> DockerImageRef {
    DockerImageRef::try_new("ghcr.io/matzapata/nitrum/nitro-cli:latest")
        .expect("default nitro-cli image is valid")
}

/// `[runtime]` in `nitrum.toml`: Docker images for Nitrum platform components.
#[derive(Clone, serde::Deserialize, serde::Serialize)]
pub struct Runtime {
    pub data_plane: DockerImageRef,

    pub control_plane: DockerImageRef,
    /// Docker image for `nitro-cli` (used by `nitrum build` / `nitrum describe` for EIF tooling).
    pub nitro_cli: DockerImageRef,
}

impl Default for Runtime {
    fn default() -> Self {
        Self {
            data_plane: default_data_plane_image(),
            control_plane: default_control_plane_image(),
            nitro_cli: default_nitro_cli_image(),
        }
    }
}

impl Runtime {
    /// Apply `NITRUM_RUNTIME_*_IMAGE` overrides from the process environment.
    ///
    /// Unset or empty values are ignored. Invalid values return
    /// [`crate::NitrumConfigError::RuntimeOverrideInvalid`].
    ///
    /// # Errors
    ///
    /// Returns an error when an override env var is set to a value that fails
    /// [`DockerImageRef::try_new`].
    pub fn apply_env_overrides(&mut self) -> Result<(), crate::NitrumConfigError> {
        apply_one(ENV_RUNTIME_DATA_PLANE_IMAGE, &mut self.data_plane)?;
        apply_one(ENV_RUNTIME_CONTROL_PLANE_IMAGE, &mut self.control_plane)?;
        apply_one(ENV_RUNTIME_NITRO_CLI_IMAGE, &mut self.nitro_cli)?;
        Ok(())
    }
}

fn apply_one(key: &str, target: &mut DockerImageRef) -> Result<(), crate::NitrumConfigError> {
    let Ok(value) = std::env::var(key) else {
        return Ok(());
    };
    if value.is_empty() {
        return Ok(());
    }
    *target = DockerImageRef::try_new(&value).map_err(|error| {
        crate::NitrumConfigError::RuntimeOverrideInvalid {
            key: key.to_string(),
            message: error.to_string(),
        }
    })?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::DockerImageRef;

    #[test]
    fn accepts_valid_image() {
        assert!(DockerImageRef::try_new("ghcr.io/user/repo:tag").is_ok());
    }

    #[test]
    fn rejects_invalid_image() {
        assert!(DockerImageRef::try_new("").is_err());
        assert!(DockerImageRef::try_new("no-slash").is_err());
    }

    #[test]
    fn with_tag_suffix_appends_to_existing_tag() {
        let image = DockerImageRef::try_new("ghcr.io/matzapata/nitrum/data-plane:latest").unwrap();
        assert_eq!(
            image.with_tag_suffix("local").as_str(),
            "ghcr.io/matzapata/nitrum/data-plane:latest-local"
        );
    }

    #[test]
    fn with_tag_suffix_defaults_missing_tag_to_latest() {
        let image = DockerImageRef::try_new("ghcr.io/matzapata/nitrum/data-plane").unwrap();
        assert_eq!(
            image.with_tag_suffix("local").as_str(),
            "ghcr.io/matzapata/nitrum/data-plane:latest-local"
        );
    }

    #[test]
    fn with_tag_suffix_keeps_registry_port() {
        let image = DockerImageRef::try_new("localhost:5000/nitrum/data-plane:dev").unwrap();
        assert_eq!(
            image.with_tag_suffix("local").as_str(),
            "localhost:5000/nitrum/data-plane:dev-local"
        );
    }

    #[test]
    fn with_tag_suffix_maps_digest_to_latest_local() {
        let image =
            DockerImageRef::try_new("ghcr.io/matzapata/nitrum/data-plane@sha256:abcdef0123456789")
                .unwrap();
        assert_eq!(
            image.with_tag_suffix("local").as_str(),
            "ghcr.io/matzapata/nitrum/data-plane:latest-local"
        );
    }
}
