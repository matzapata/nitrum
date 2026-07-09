//! Small helpers for reading process environment variables.
//!
//! **Empty-string policy:** for most `NITRUM_*` variables an empty value is treated the same as
//! unset. Exceptions document their own semantics (for example
//! [`config::ENV_OTLP_ENDPOINT`], where `""` explicitly disables OTLP export).

/// Whether `key` is present in the environment (including when set to an empty value).
///
/// Prefer [`optional_nonempty`] when an empty value should behave like unset.
#[must_use]
pub fn is_set(key: &str) -> bool {
    std::env::var(key).is_ok()
}

/// Returns the variable value when set and non-empty.
#[must_use]
pub fn optional_nonempty(key: &str) -> Option<String> {
    std::env::var(key).ok().filter(|value| !value.is_empty())
}

/// Returns the variable value when set and non-empty, otherwise `default`.
#[must_use]
pub fn var_or_nonempty_default(key: &str, default: &str) -> String {
    optional_nonempty(key).unwrap_or_else(|| default.to_string())
}

/// Returns the variable value when set, otherwise `default`.
///
/// Prefer [`var_or_nonempty_default`] unless an empty override must be preserved.
#[must_use]
pub fn var_or_default(key: &str, default: &str) -> String {
    std::env::var(key).unwrap_or_else(|_| default.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn env_helpers() {
        let optional_key = "NITRUM_TEST_OPTIONAL_NONEMPTY";
        unsafe { std::env::set_var(optional_key, "") };
        assert_eq!(optional_nonempty(optional_key), None);
        unsafe { std::env::set_var(optional_key, "value") };
        assert_eq!(optional_nonempty(optional_key).as_deref(), Some("value"));
        unsafe { std::env::remove_var(optional_key) };

        let default_key = "NITRUM_TEST_VAR_OR_DEFAULT";
        unsafe { std::env::remove_var(default_key) };
        assert_eq!(var_or_default(default_key, "fallback"), "fallback");
        unsafe { std::env::set_var(default_key, "override") };
        assert_eq!(var_or_default(default_key, "fallback"), "override");
        unsafe { std::env::remove_var(default_key) };

        let nonempty_default_key = "NITRUM_TEST_VAR_OR_NONEMPTY_DEFAULT";
        unsafe { std::env::remove_var(nonempty_default_key) };
        assert_eq!(
            var_or_nonempty_default(nonempty_default_key, "fallback"),
            "fallback"
        );
        unsafe { std::env::set_var(nonempty_default_key, "") };
        assert_eq!(
            var_or_nonempty_default(nonempty_default_key, "fallback"),
            "fallback"
        );
        unsafe { std::env::remove_var(nonempty_default_key) };
    }
}
