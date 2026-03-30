//! Local development utilities

use anyhow::{Context, Result, bail};
use shared::config::NitrumConfig;
use std::path::Path;
use std::process::Stdio;
use tokio::process::Command;

use crate::constants;

pub struct EnclaveLocalStack<'a> {
    project_root: &'a Path,
    enclave_image: String,
    data_plane_image: String,
}

impl<'a> EnclaveLocalStack<'a> {
    pub fn new(project_root: &'a Path, cfg: &NitrumConfig) -> Self {
        Self {
            project_root,
            enclave_image: format!("nitrum-{}:dev", cfg.project.name),
            data_plane_image: cfg.runtime.data_plane.clone(),
        }
    }

    pub async fn up(&self) -> Result<()> {
        let compose_file = self.ensure_template()?;

        let output = Command::new("docker")
            .current_dir(self.project_root)
            .env("ENCLAVE_IMAGE", &self.enclave_image)
            .env(
                "DATA_PLANE_IMAGE",
                format!("{}-dev", &self.data_plane_image),
            )
            .arg("compose")
            .arg("--progress")
            .arg("quiet")
            .arg("-f")
            .arg(compose_file)
            .args(["up", "--build", "-d"])
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

    pub async fn down(&self) -> Result<()> {
        let compose_file = self.ensure_template()?;

        let output = Command::new("docker")
            .current_dir(self.project_root)
            .env("ENCLAVE_IMAGE", &self.enclave_image)
            .env("DATA_PLANE_IMAGE", &self.data_plane_image)
            .arg("compose")
            .arg("--progress")
            .arg("quiet")
            .arg("-f")
            .arg(compose_file)
            .args(["down"])
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

    pub async fn logs(&self, tail: Option<u32>, follow: bool) -> Result<()> {
        let compose_file = self.ensure_template()?;

        let mut cmd = Command::new("docker");
        cmd.current_dir(self.project_root)
            .env("ENCLAVE_IMAGE", &self.enclave_image)
            .arg("compose")
            .arg("-f")
            .arg(compose_file)
            .arg("logs")
            .arg("enclave")
            .arg("--no-log-prefix");

        if let Some(n) = tail {
            cmd.arg("--tail").arg(n.to_string());
        }
        if follow {
            cmd.arg("--follow");
        }

        cmd.stdin(Stdio::inherit())
            .stdout(Stdio::inherit())
            .stderr(Stdio::inherit());

        let status = cmd
            .status()
            .await
            .context("failed to spawn `docker compose logs`")?;
        if !status.success() {
            bail!("docker compose logs exited with status {status}");
        }
        Ok(())
    }

    fn local_stack_template() -> &'static str {
        include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/docker-compose.yml"))
    }

    /// Writes the local stack file from the bundled template if it is not already present.
    /// Returns the project-relative path ([`constants::ENCLAVE_LOCAL_STACK_TEMPLATE_FILE`]).
    fn ensure_template(&self) -> Result<&'static str> {
        let compose_path = self
            .project_root
            .join(constants::ENCLAVE_LOCAL_STACK_TEMPLATE_FILE);
        if compose_path.is_file() {
            return Ok(constants::ENCLAVE_LOCAL_STACK_TEMPLATE_FILE);
        }
        if let Some(parent) = compose_path.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("create {}", parent.display()))?;
        }
        std::fs::write(&compose_path, Self::local_stack_template())
            .with_context(|| format!("write {}", compose_path.display()))?;
        Ok(constants::ENCLAVE_LOCAL_STACK_TEMPLATE_FILE)
    }
}
