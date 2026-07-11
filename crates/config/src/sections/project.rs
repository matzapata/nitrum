/// Validated `[project].name` from `nitrum.toml`.
///
/// Must be compatible with CloudFormation, SSM, Docker, and S3 naming rules.
#[derive(Debug, Clone, PartialEq, Eq, Hash, serde::Serialize)]
#[serde(transparent)]
pub struct ProjectName(String);

#[derive(Debug, Clone, thiserror::Error)]
#[error("{0}")]
pub struct ProjectNameError(String);

impl ProjectName {
    /// Validates and constructs a project name.
    ///
    /// # Errors
    ///
    /// Returns an error when `value` does not satisfy Nitrum project naming rules.
    pub fn try_new(value: &str) -> Result<Self, ProjectNameError> {
        if value != value.trim() {
            return Err(ProjectNameError(
                "`project.name` must not have leading or trailing whitespace".to_string(),
            ));
        }
        if value.is_empty() {
            return Err(ProjectNameError(
                "`project.name` must not be empty".to_string(),
            ));
        }

        let mut chars = value.chars();
        let Some(first) = chars.next() else {
            return Err(ProjectNameError(
                "`project.name` must not be empty".to_string(),
            ));
        };
        if !first.is_ascii_lowercase() {
            return Err(ProjectNameError(
                "`project.name` must start with a lowercase letter (a-z)".to_string(),
            ));
        }

        let mut rest_len = 0usize;
        for c in chars {
            rest_len += 1;
            if !matches!(c, 'a'..='z' | '0'..='9' | '-') {
                return Err(ProjectNameError(format!(
                    "`project.name` after the first character must use only lowercase letters, digits, or hyphens (invalid character {c:?})"
                )));
            }
        }
        if !(2..=127).contains(&rest_len) {
            return Err(ProjectNameError(format!(
                "`project.name` must be 3-128 characters (one leading letter plus 2-127 more); got {rest_len} character(s) after the first",
            )));
        }

        Ok(Self(value.to_string()))
    }

    /// Borrow the validated name as UTF-8.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::ops::Deref for ProjectName {
    type Target = str;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl std::fmt::Display for ProjectName {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::str::FromStr for ProjectName {
    type Err = ProjectNameError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        Self::try_new(value)
    }
}

impl TryFrom<String> for ProjectName {
    type Error = ProjectNameError;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::try_new(&value)
    }
}

impl<'de> serde::Deserialize<'de> for ProjectName {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        Self::try_new(&value).map_err(serde::de::Error::custom)
    }
}

/// Validates a project name string without constructing [`ProjectName`].
///
/// # Errors
///
/// Returns `Err` with a human-readable message when `name` does not satisfy
/// Nitrum project naming rules.
pub fn validate_project_name(name: &str) -> Result<(), String> {
    ProjectName::try_new(name)
        .map(|_| ())
        .map_err(|error| error.to_string())
}

/// `[project]` in `nitrum.toml`.
#[derive(Clone, serde::Deserialize, serde::Serialize)]
pub struct Project {
    pub name: ProjectName,
    /// TCP port your application listens on (`127.0.0.1`); the ingress proxies here after TLS.
    pub port: u16,
    /// Process argv for the user workload (read from `nitrum.toml` by the data-plane).
    #[serde(default, alias = "command")]
    pub start_command: Vec<String>,
}

impl Project {
    /// Validates semantic constraints for `[project]`.
    ///
    /// # Errors
    ///
    /// Returns `Err` with a human-readable message when `port` is invalid.
    pub fn validate(&self) -> Result<(), String> {
        if self.port == 0 {
            return Err("`project.port` must not be 0".to_string());
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::ProjectName;

    #[test]
    fn accepts_valid_name() {
        assert!(ProjectName::try_new("nitrum-hello").is_ok());
    }

    #[test]
    fn rejects_invalid_name() {
        assert!(ProjectName::try_new("Bad").is_err());
        assert!(ProjectName::try_new("ab").is_err());
    }
}
