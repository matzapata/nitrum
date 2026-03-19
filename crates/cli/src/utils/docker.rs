use anyhow::{Context, Result, bail};
use std::path::Path;
use std::process::Stdio;
use tokio::process::Command;

use crate::constants;

/// Build the project Dockerfile as [`constants::ENCLAVE_DEV_LOCAL_IMAGE`] (quiet; use a spinner in the caller).
pub async fn build_enclave(root: &Path) -> Result<()> {
    let dockerfile = root.join("Dockerfile");
    if !dockerfile.is_file() {
        bail!(
            "Dockerfile not found at {}\n\
             Run this from your Nitrum project root (where the Dockerfile lives).",
            dockerfile.display()
        );
    }

    let output = Command::new("docker")
        .current_dir(root)
        .arg("build")
        .arg("-q")
        .arg("-f")
        .arg("Dockerfile")
        .arg("-t")
        .arg(constants::ENCLAVE_DEV_LOCAL_IMAGE)
        .arg("--build-arg")
        .arg(format!(
            "DATA_PLANE_IMAGE={}",
            constants::ENCLAVE_DEV_BASE_IMAGE
        ))
        .arg(".")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .await
        .context("failed to spawn `docker build`")?;

    if output.status.success() {
        return Ok(());
    }

    let stderr = String::from_utf8_lossy(&output.stderr);
    let stdout = String::from_utf8_lossy(&output.stdout);
    let detail = [&*stderr, &*stdout]
        .into_iter()
        .map(str::trim)
        .find(|s| !s.is_empty())
        .unwrap_or("");

    bail!(
        "docker build failed ({}){}{}",
        output.status,
        if detail.is_empty() { "" } else { "\n\n" },
        detail,
    );
}
