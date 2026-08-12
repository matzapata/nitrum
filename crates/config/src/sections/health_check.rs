/// Validated `[health_check].path` from `nitrum.toml`.
#[derive(Debug, Clone, PartialEq, Eq, Hash, serde::Serialize)]
#[serde(transparent)]
pub struct HealthCheckPath(String);

#[derive(Debug, Clone, thiserror::Error)]
#[error("{0}")]
pub struct HealthCheckPathError(String);

impl HealthCheckPath {
    /// Validates and constructs a health-check path.
    ///
    /// # Errors
    ///
    /// Returns an error when `value` is empty, has surrounding whitespace, or does not start with `/`.
    pub fn try_new(value: &str) -> Result<Self, HealthCheckPathError> {
        if value != value.trim() {
            return Err(HealthCheckPathError(
                "`health_check.path` must not have leading or trailing whitespace".to_string(),
            ));
        }
        if value.is_empty() {
            return Err(HealthCheckPathError(
                "`health_check.path` must not be empty".to_string(),
            ));
        }
        if !value.starts_with('/') {
            return Err(HealthCheckPathError(
                "`health_check.path` must start with `/`".to_string(),
            ));
        }
        Ok(Self(value.to_string()))
    }

    /// Borrow the validated path as UTF-8.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::ops::Deref for HealthCheckPath {
    type Target = str;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl std::fmt::Display for HealthCheckPath {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::str::FromStr for HealthCheckPath {
    type Err = HealthCheckPathError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        Self::try_new(value)
    }
}

impl TryFrom<String> for HealthCheckPath {
    type Error = HealthCheckPathError;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::try_new(&value)
    }
}

impl<'de> serde::Deserialize<'de> for HealthCheckPath {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        Self::try_new(&value).map_err(serde::de::Error::custom)
    }
}

/// `[health_check]` in `nitrum.toml`.
#[derive(Clone, serde::Deserialize, serde::Serialize)]
pub struct HealthCheck {
    /// Path to the health check endpoint.
    pub path: HealthCheckPath,

    /// Port to use for the health check.
    pub port: std::num::NonZeroU16,

    /// Interval in seconds to wait between health checks.
    pub interval: std::num::NonZeroU32,
}

impl Default for HealthCheck {
    fn default() -> Self {
        Self {
            path: HealthCheckPath::try_new("/health").expect("default health check path is valid"),
            port: std::num::NonZeroU16::new(8080).expect("default health check port is valid"),
            interval: std::num::NonZeroU32::new(10)
                .expect("default health check interval is valid"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::HealthCheckPath;

    #[test]
    fn accepts_valid_path() {
        assert!(HealthCheckPath::try_new("/health").is_ok());
    }

    #[test]
    fn rejects_invalid_path() {
        assert!(HealthCheckPath::try_new("health").is_err());
        assert!(HealthCheckPath::try_new("").is_err());
    }
}
