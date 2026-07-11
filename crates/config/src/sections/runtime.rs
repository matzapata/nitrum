/// Validated Docker image reference (for example `registry/repo:tag`).
#[derive(Debug, Clone, PartialEq, Eq, Hash, serde::Serialize)]
#[serde(transparent)]
pub struct DockerImageRef(String);

#[derive(Debug, Clone, thiserror::Error)]
#[error("{0}")]
pub struct DockerImageRefError(String);

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
    /// Validates semantic constraints for `[runtime]`.
    ///
    /// Image references are validated when deserialized or constructed; this is a no-op.
    pub const fn validate(&self) -> Result<(), String> {
        Ok(())
    }
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
}
