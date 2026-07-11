//! AWS SDK config helpers.

/// Region string for prompts (`AWS_REGION` / `AWS_DEFAULT_REGION` / `us-east-1`).
#[must_use]
pub fn region_display() -> String {
    std::env::var("AWS_REGION")
        .or_else(|_| std::env::var("AWS_DEFAULT_REGION"))
        .unwrap_or_else(|_| "us-east-1".to_string())
}
