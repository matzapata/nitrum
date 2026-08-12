/// Validated `[egress].destinations` regex pattern from `nitrum.toml`.
#[derive(Debug, Clone, PartialEq, Eq, Hash, serde::Serialize)]
#[serde(transparent)]
pub struct EgressPattern(String);

#[derive(Debug, Clone, thiserror::Error)]
#[error("{0}")]
pub struct EgressPatternError(String);

impl EgressPattern {
    /// Validates and constructs an egress destination regex pattern.
    ///
    /// # Errors
    ///
    /// Returns an error when `value` is empty or not a valid regex.
    pub fn try_new(value: &str) -> Result<Self, EgressPatternError> {
        if value.trim().is_empty() {
            return Err(EgressPatternError(
                "`egress.destinations` entries must not be empty".to_string(),
            ));
        }
        regex::Regex::new(value).map_err(|error| {
            EgressPatternError(format!(
                "`egress.destinations` invalid regex `{value}`: {error}"
            ))
        })?;
        Ok(Self(value.to_string()))
    }

    /// Borrow the validated pattern as UTF-8.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::ops::Deref for EgressPattern {
    type Target = str;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl std::fmt::Display for EgressPattern {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::str::FromStr for EgressPattern {
    type Err = EgressPatternError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        Self::try_new(value)
    }
}

impl TryFrom<String> for EgressPattern {
    type Error = EgressPatternError;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::try_new(&value)
    }
}

impl<'de> serde::Deserialize<'de> for EgressPattern {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        Self::try_new(&value).map_err(serde::de::Error::custom)
    }
}

/// Outbound traffic restrictions enforced inside the data-plane.
#[derive(Debug, Clone, Default, serde::Deserialize, serde::Serialize)]
pub struct Egress {
    /// When false, all outbound traffic is allowed regardless of `destinations`.
    #[serde(default)]
    pub enabled: bool,

    /// Regex patterns matched against destination hostnames at DNS query time.
    /// An empty list blocks all DNS-resolved traffic when `enabled` is true.
    #[serde(default)]
    pub destinations: Vec<EgressPattern>,
}

#[cfg(test)]
mod tests {
    use super::{Egress, EgressPattern};

    #[test]
    fn egress_pattern_rejects_invalid_regex() {
        assert!(EgressPattern::try_new("(").is_err());
    }

    #[test]
    fn egress_pattern_accepts_valid_patterns() {
        assert!(EgressPattern::try_new(r"httpbin\.org$").is_ok());
    }

    #[test]
    fn egress_deserializes_valid_destinations() {
        let egress: Egress = toml::from_str(
            r#"
            enabled = true
            destinations = ["httpbin\\.org$"]
            "#,
        )
        .expect("valid egress config");
        assert_eq!(egress.destinations.len(), 1);
    }

    #[test]
    fn egress_rejects_invalid_destinations_on_deserialize() {
        let error = toml::from_str::<Egress>(
            r#"
            enabled = true
            destinations = ["("]
            "#,
        )
        .expect_err("invalid regex should fail at deserialize");
        assert!(error.to_string().contains("invalid regex"));
    }
}
