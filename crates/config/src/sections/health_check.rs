#[derive(Clone, serde::Deserialize, serde::Serialize)]
pub struct HealthCheck {
    /// Path to the health check endpoint.
    pub path: String,

    /// Port to use for the health check.
    pub port: u16,

    /// Interval in seconds to wait between health checks.
    pub interval: u32,
}

impl Default for HealthCheck {
    fn default() -> Self {
        Self {
            path: "/health".to_string(),
            port: 8080,
            interval: 10,
        }
    }
}

impl HealthCheck {
    /// Validates semantic constraints for `[health_check]`.
    ///
    /// # Errors
    ///
    /// Returns `Err` with a human-readable message when any field is invalid
    /// (for example an empty path or zero interval).
    pub fn validate(&self) -> Result<(), String> {
        let path = self.path.trim();
        if path.is_empty() {
            return Err("`health_check.path` must not be empty".to_string());
        }
        if path != self.path {
            return Err(
                "`health_check.path` must not have leading or trailing whitespace".to_string(),
            );
        }
        if !path.starts_with('/') {
            return Err("`health_check.path` must start with `/`".to_string());
        }
        if self.port == 0 {
            return Err("`health_check.port` must not be 0".to_string());
        }
        if self.interval == 0 {
            return Err("`health_check.interval` must be greater than 0".to_string());
        }
        Ok(())
    }
}
