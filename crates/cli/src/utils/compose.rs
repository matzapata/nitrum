use anyhow::{Context, Result, bail};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use tokio::process::Command;

use crate::constants;

pub fn compose_file_path(project_root: &Path) -> PathBuf {
    project_root.join(constants::ENCLAVE_DEV_COMPOSE_FILE)
}

pub fn require_compose_file(project_root: &Path) -> Result<()> {
    let p = compose_file_path(project_root);
    if p.is_file() {
        return Ok(());
    }
    bail!(
        "compose file not found: {}\n\
         Expected `{}` under the project root (run `nitrum init` or add the compose file).",
        p.display(),
        constants::ENCLAVE_DEV_COMPOSE_FILE,
    );
}

/// Runs `docker compose` without streaming to the terminal (spinner-friendly).
/// On failure, stderr/stdout from Compose are included in the error.
pub async fn docker_compose(project_root: &Path, compose_args: &[&str]) -> Result<()> {
    require_compose_file(project_root)?;

    let output = Command::new("docker")
        .current_dir(project_root)
        .env("ENCLAVE_IMAGE", constants::ENCLAVE_DEV_LOCAL_IMAGE)
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
