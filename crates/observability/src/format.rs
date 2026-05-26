//! Log output format from `NITRUM_LOG_FORMAT`.

/// Output format for stderr and CloudWatch export.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LogFormat {
    /// Human-readable lines (default).
    Human,
    /// One JSON object per line (production).
    Json,
}

impl LogFormat {
    /// Reads [`LogFormat`] from `NITRUM_LOG_FORMAT` (`json` or `human`, case-insensitive).
    #[must_use]
    pub fn from_env() -> Self {
        match std::env::var("NITRUM_LOG_FORMAT")
            .ok()
            .as_deref()
            .map(str::trim)
            .map(str::to_ascii_lowercase)
            .as_deref()
        {
            Some("json") => Self::Json,
            _ => Self::Human,
        }
    }
}
