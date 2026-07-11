/// Outbound traffic restrictions enforced inside the data-plane.
#[derive(Clone, Default, serde::Deserialize, serde::Serialize)]
pub struct Egress {
    /// When false, all outbound traffic is allowed regardless of `destinations`.
    #[serde(default)]
    pub enabled: bool,

    /// Regex patterns matched against destination hostnames at DNS query time.
    /// An empty list blocks all DNS-resolved traffic when `enabled` is true.
    #[serde(default)]
    pub destinations: Vec<String>,
}

impl Egress {
    /// Validates semantic constraints for `[egress]`.
    ///
    /// # Errors
    ///
    /// Returns `Err` when any destination pattern is empty or not a valid regex.
    pub fn validate(&self) -> Result<(), String> {
        for (index, pattern) in self.destinations.iter().enumerate() {
            if pattern.trim().is_empty() {
                return Err(format!("`egress.destinations[{index}]` must not be empty"));
            }
            regex::Regex::new(pattern).map_err(|error| {
                format!("`egress.destinations[{index}]` invalid regex `{pattern}`: {error}")
            })?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::Egress;

    #[test]
    fn egress_rejects_invalid_regex() {
        let egress = Egress {
            enabled: true,
            destinations: vec!["(".to_string()],
        };
        assert!(egress.validate().is_err());
    }

    #[test]
    fn egress_accepts_valid_patterns() {
        let egress = Egress {
            enabled: true,
            destinations: vec![r"httpbin\.org$".to_string()],
        };
        assert!(egress.validate().is_ok());
    }
}
