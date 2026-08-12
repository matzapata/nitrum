/// Validated `[tls_termination].domain` from `nitrum.toml`.
#[derive(Debug, Clone, PartialEq, Eq, Hash, serde::Serialize)]
#[serde(transparent)]
pub struct TlsDomain(String);

#[derive(Debug, Clone, thiserror::Error)]
#[error("{0}")]
pub struct TlsDomainError(String);

impl TlsDomain {
    /// Validates and constructs a TLS domain name.
    ///
    /// # Errors
    ///
    /// Returns an error when `value` is empty, contains whitespace, or exceeds DNS length limits.
    pub fn try_new(value: &str) -> Result<Self, TlsDomainError> {
        if value != value.trim() {
            return Err(TlsDomainError(
                "`tls_termination.domain` must not have leading or trailing whitespace".to_string(),
            ));
        }
        if value.is_empty() {
            return Err(TlsDomainError(
                "`tls_termination.domain` must not be empty".to_string(),
            ));
        }
        if value.chars().any(char::is_whitespace) {
            return Err(TlsDomainError(
                "`tls_termination.domain` must not contain whitespace".to_string(),
            ));
        }
        if value.len() > 253 {
            return Err(TlsDomainError(
                "`tls_termination.domain` exceeds 253 characters (DNS limit)".to_string(),
            ));
        }
        Ok(Self(value.to_string()))
    }

    /// Returns true when the domain uses a `.local` private-use suffix.
    #[must_use]
    pub fn is_local(&self) -> bool {
        std::path::Path::new(self.as_str())
            .extension()
            .is_some_and(|ext| ext.eq_ignore_ascii_case("local"))
    }

    /// Borrow the validated domain as UTF-8.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::ops::Deref for TlsDomain {
    type Target = str;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl std::fmt::Display for TlsDomain {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::str::FromStr for TlsDomain {
    type Err = TlsDomainError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        Self::try_new(value)
    }
}

impl TryFrom<String> for TlsDomain {
    type Error = TlsDomainError;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::try_new(&value)
    }
}

impl<'de> serde::Deserialize<'de> for TlsDomain {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        Self::try_new(&value).map_err(serde::de::Error::custom)
    }
}

#[derive(Debug, Clone, thiserror::Error)]
#[error("{0}")]
pub struct TlsTerminationError(String);

/// `[tls_termination]` in `nitrum.toml`.
#[derive(Debug, Clone, serde::Serialize)]
pub struct TlsTermination {
    /// Use ACME (e.g. Let's Encrypt) for certificate issuance instead of self-signed.
    pub acme: bool,

    /// Domain name used for TLS certificate generation (self-signed or ACME).
    pub domain: TlsDomain,
}

impl TlsTermination {
    /// Validates and constructs TLS termination settings.
    ///
    /// # Errors
    ///
    /// Returns an error when ACME is enabled with a private-use `.local` domain.
    pub fn try_new(acme: bool, domain: TlsDomain) -> Result<Self, TlsTerminationError> {
        if acme && domain.is_local() {
            return Err(TlsTerminationError(
                "`tls_termination.domain` cannot end with `.local` when `tls_termination.acme` is true; public CAs do not issue for private-use names".to_string(),
            ));
        }
        Ok(Self { acme, domain })
    }
}

impl Default for TlsTermination {
    fn default() -> Self {
        Self::try_new(
            false,
            TlsDomain::try_new("nitrum.localhost").expect("default tls domain is valid"),
        )
        .expect("default tls termination is valid")
    }
}

#[derive(serde::Deserialize)]
struct TlsTerminationRaw {
    acme: bool,
    domain: TlsDomain,
}

impl<'de> serde::Deserialize<'de> for TlsTermination {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let raw = TlsTerminationRaw::deserialize(deserializer)?;
        Self::try_new(raw.acme, raw.domain).map_err(serde::de::Error::custom)
    }
}

#[cfg(test)]
mod tests {
    use super::{TlsDomain, TlsTermination};

    #[test]
    fn rejects_acme_with_local_domain() {
        let domain = TlsDomain::try_new("nitrum.local").expect("valid domain");
        assert!(TlsTermination::try_new(true, domain).is_err());
    }

    #[test]
    fn accepts_acme_with_localhost_domain() {
        let domain = TlsDomain::try_new("nitrum.localhost").expect("valid domain");
        assert!(TlsTermination::try_new(true, domain).is_ok());
    }

    #[test]
    fn accepts_acme_with_public_domain() {
        let domain = TlsDomain::try_new("example.com").expect("valid domain");
        assert!(TlsTermination::try_new(true, domain).is_ok());
    }
}
