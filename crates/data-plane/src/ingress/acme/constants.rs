//! ACME certificate provisioning timing and directory URL configuration.

use crate::utils::env::{optional_nonempty, var_or_nonempty_default};
use std::time::Duration;

/// Delay between polls while waiting for the ACME leader lock (another instance may be provisioning).
pub const ACME_LOCK_RETRY_INTERVAL: Duration = Duration::from_secs(2);

/// Default certificate renewal fraction (2/3 of the certificate lifetime).
pub const CERTIFICATE_RENEWAL_FRACTION: f64 = 2.0 / 3.0;

/// Default Let's Encrypt production directory.
pub const DEFAULT_ACME_DIRECTORY_URL: &str = "https://acme-v02.api.letsencrypt.org/directory";

/// Environment variable for the ACME directory URL.
pub const ENV_ACME_DIRECTORY_URL: &str = "NITRUM_ACME_DIRECTORY_URL";

/// Effective ACME directory URL: non-empty [`ENV_ACME_DIRECTORY_URL`] when set, otherwise
/// [`DEFAULT_ACME_DIRECTORY_URL`] (Let's Encrypt production).
#[must_use]
pub fn acme_directory_url() -> String {
    var_or_nonempty_default(ENV_ACME_DIRECTORY_URL, DEFAULT_ACME_DIRECTORY_URL)
}

/// Non-empty ACME directory URL override from [`ENV_ACME_DIRECTORY_URL`], if any.
#[must_use]
pub fn acme_directory_url_override() -> Option<String> {
    optional_nonempty(ENV_ACME_DIRECTORY_URL)
}
