use anyhow::{Context, Result, anyhow, bail};
use std::path::Path;

use shared::config::{self, Config};

fn sanitize_bucket_label(s: &str) -> String {
    s.trim()
        .trim_matches(|c| c == '.' || c == '-')
        .chars()
        .map(|c| match c {
            'A'..='Z' => c.to_ascii_lowercase(),
            'a'..='z' | '0'..='9' | '-' | '.' => c,
            _ => '-',
        })
        .collect()
}

/// Default EIF bucket: `nitrum-{project-name}` (S3 label rules, ≤63 chars).
pub fn derived_eif_bucket_name(project_name: &str) -> Result<String> {
    let slug = sanitize_bucket_label(project_name);
    if slug.is_empty() {
        bail!("`name` in nitrum.toml is empty after sanitization; set a valid project slug");
    }
    let s = format!("nitrum-{slug}");
    if !(3..=63).contains(&s.len()) {
        bail!(
            "derived S3 bucket name `{s}` is not 3–63 characters; shorten `name` in nitrum.toml"
        );
    }
    Ok(s)
}

/// Local enclave image for dev (`docker build` / compose `${ENCLAVE_IMAGE}`): `nitrum-{name}:dev`.
pub fn enclave_image_dev(project_name: &str) -> String {
    format!("nitrum-{project_name}:dev")
}

/// Local enclave image for prod / nitro-cli: `nitrum-{name}:latest`.
pub fn enclave_image_prod(project_name: &str) -> String {
    format!("nitrum-{project_name}:latest")
}

pub fn load_project_config(root: &Path) -> Result<Config> {
    let path = root.join("nitrum.toml");
    config::try_load(&path)
        .map_err(|e| anyhow!(e))
        .with_context(|| format!("loading {}", path.display()))
}
