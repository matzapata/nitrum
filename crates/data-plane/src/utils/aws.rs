//! AWS SDK config helpers.

/// Region string from an SDK config (falls back to `"unknown"` when unset).
#[must_use]
pub fn region_from_sdk(config: &aws_config::SdkConfig) -> &str {
    config
        .region()
        .map_or("unknown", std::convert::AsRef::as_ref)
}
