use anyhow::{Context, Result, bail};
use std::path::Path;
use std::process::Stdio;
use tokio::process::Command;

use crate::bundled;
use crate::constants;

/// Ensures `.nitrum/docker-compose.yml` exists: writes the bundled dev stack from the CLI if missing.
pub fn ensure_compose_file(project_root: &Path) -> Result<()> {
    let p = project_root.join(constants::ENCLAVE_DEV_COMPOSE_FILE);
    if p.is_file() {
        return Ok(());
    }
    if let Some(parent) = p.parent() {
        std::fs::create_dir_all(parent).with_context(|| format!("create {}", parent.display()))?;
    }
    std::fs::write(&p, bundled::docker_compose_yml())
        .with_context(|| format!("write {}", p.display()))?;
    Ok(())
}

/// Runs `docker compose` without streaming to the terminal (spinner-friendly).
/// `enclave_image` is passed as `${ENCLAVE_IMAGE}` (see dev compose file).
/// On failure, stderr/stdout from Compose are included in the error.
pub async fn docker_compose(
    project_root: &Path,
    enclave_image: &str,
    compose_args: &[&str],
) -> Result<()> {
    ensure_compose_file(project_root)?;

    let output = Command::new("docker")
        .current_dir(project_root)
        .env("ENCLAVE_IMAGE", enclave_image)
        .arg("compose")
        .arg("--progress")
        .arg("quiet")
        .arg("-f")
        .arg(constants::ENCLAVE_DEV_COMPOSE_FILE)
        .args(compose_args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .await
        .context("failed to spawn `docker compose`")?;

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
        "docker compose failed ({}){}{}",
        output.status,
        if detail.is_empty() { "" } else { "\n\n" },
        detail,
    );
}
