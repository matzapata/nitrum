#[derive(Clone, serde::Deserialize, serde::Serialize)]
pub struct TlsTermination {
    /// Use ACME (e.g. Let's Encrypt) for certificate issuance instead of self-signed.
    pub acme: bool,

    /// Domain name used for TLS certificate generation (self-signed or ACME).
    pub domain: String,
}

impl Default for TlsTermination {
    fn default() -> Self {
        Self {
            acme: false,
            domain: "nitrum.local".to_string(),
        }
    }
}

impl TlsTermination {
    /// Validates semantic constraints for `[tls_termination]`.
    ///
    /// # Errors
    ///
    /// Returns `Err` with a human-readable message when the domain or ACME
    /// settings are invalid (for example an empty domain or ACME with `.local`).
    pub fn validate(&self) -> Result<(), String> {
        let domain = self.domain.trim();
        if domain.is_empty() {
            return Err("`tls_termination.domain` must not be empty".to_string());
        }
        if domain != self.domain {
            return Err(
                "`tls_termination.domain` must not have leading or trailing whitespace".to_string(),
            );
        }
        if domain.chars().any(char::is_whitespace) {
            return Err("`tls_termination.domain` must not contain whitespace".to_string());
        }
        if domain.len() > 253 {
            return Err("`tls_termination.domain` exceeds 253 characters (DNS limit)".to_string());
        }
        if self.acme
            && std::path::Path::new(domain)
                .extension()
                .is_some_and(|ext| ext.eq_ignore_ascii_case("local"))
        {
            return Err("`tls_termination.domain` cannot end with `.local` when `tls_termination.acme` is true; public CAs do not issue for private-use names".to_string());
        }
        Ok(())
    }
}
