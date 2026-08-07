//! Local development utilities

use anyhow::{Context, Result, bail};
use config::NitrumConfig;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use tokio::process::Command;

use crate::artifact::resolve_docker_build_paths;
use crate::constants;

pub struct EnclaveLocalStack<'a> {
    /// Project directory containing `nitrum.toml` and the enclave `Dockerfile`.
    project_root: &'a Path,
    /// Local compose tag for the app image (`nitrum-{name}:dev`).
    enclave_image: String,
    /// Base image for the enclave Dockerfile (`DATA_PLANE_IMAGE`);
    /// pebble / local Compose build derived from `[runtime].data_plane` with a `-local` tag suffix.
    data_plane_image: String,
    /// Docker build context (workspace root for in-repo examples, else project dir).
    build_context: PathBuf,
    /// Dockerfile path relative to [`Self::build_context`].
    dockerfile: String,
}

impl<'a> EnclaveLocalStack<'a> {
    /// # Errors
    ///
    /// Returns an error when the project path cannot be canonicalized or the
    /// Docker build context cannot be resolved.
    pub fn new(project_root: &'a Path, cfg: &NitrumConfig) -> Result<Self> {
        let (build_context, dockerfile) = resolve_docker_build_paths(project_root)?;
        Ok(Self {
            project_root,
            enclave_image: format!("nitrum-{}:dev", cfg.project.name),
            data_plane_image: cfg.runtime.data_plane.with_tag_suffix("local").to_string(),
            build_context,
            dockerfile,
        })
    }

    fn apply_build_env<'cmd>(&self, cmd: &'cmd mut Command) -> &'cmd mut Command {
        cmd.env("ENCLAVE_IMAGE", &self.enclave_image)
            .env("DATA_PLANE_IMAGE", &self.data_plane_image)
            .env(
                "ENCLAVE_BUILD_CONTEXT",
                self.build_context.to_string_lossy().as_ref(),
            )
            .env("ENCLAVE_DOCKERFILE", &self.dockerfile)
    }

    /// Start the local stack (equivalent to `docker compose up` with the Nitrum template).
    pub async fn up(&self) -> Result<()> {
        let compose_file = self.ensure_template()?;

        let mut cmd = Command::new("docker");
        cmd.current_dir(self.project_root);
        self.apply_build_env(&mut cmd)
            // `docker compose build` under BuildKit / buildx may try to resolve local-only base
            // images from a registry. Disable BuildKit here so local tags such as
            // `nitrum-e2e-data-plane:local` work as `FROM ${DATA_PLANE_IMAGE}` inputs.
            .env("DOCKER_BUILDKIT", "0")
            .env("DOCKER_DEFAULT_PLATFORM", "linux/amd64")
            .env("COMPOSE_DOCKER_CLI_BUILD", "0")
            .arg("compose")
            // Inherit stdio (not `--progress quiet` + pipes): OrbStack's compose plugin can
            // spin forever when stdout is a pipe, and quiet mode hides the enclave image build.
            .arg("--progress")
            .arg("plain")
            .arg("-f")
            .arg(compose_file)
            .args(["up", "--build", "-d"])
            .stdin(Stdio::null())
            .stdout(Stdio::inherit())
            .stderr(Stdio::inherit());

        let status = cmd
            .status()
            .await
            .context("failed to spawn `docker compose`")?;

        if status.success() {
            return Ok(());
        }

        bail!("docker compose failed ({status})");
    }

    /// Stop the local stack (equivalent to `docker compose down` with the Nitrum template).
    pub async fn down(&self) -> Result<()> {
        let compose_file = self.ensure_template()?;

        let mut cmd = Command::new("docker");
        cmd.current_dir(self.project_root);
        self.apply_build_env(&mut cmd)
            .env("DOCKER_DEFAULT_PLATFORM", "linux/amd64")
            .arg("compose")
            .arg("--progress")
            .arg("quiet")
            .arg("-f")
            .arg(compose_file)
            .args(["down"])
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());

        let output = cmd
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

    /// Tail local enclave logs using `docker compose logs`.
    pub async fn logs(&self, tail: Option<u32>, follow: bool) -> Result<()> {
        let compose_file = self.ensure_template()?;

        let mut cmd = Command::new("docker");
        cmd.current_dir(self.project_root);
        self.apply_build_env(&mut cmd)
            .env("DOCKER_DEFAULT_PLATFORM", "linux/amd64")
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

    /// The local stack template file.
    const fn local_stack_template() -> &'static str {
        include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/docker-compose.yml"))
    }

    /// Writes the bundled Compose template (always refreshed so CLI upgrades apply).
    /// Returns the project-relative path ([`constants::ENCLAVE_LOCAL_STACK_TEMPLATE_FILE`]).
    fn ensure_template(&self) -> Result<&'static str> {
        let compose_path = self
            .project_root
            .join(constants::ENCLAVE_LOCAL_STACK_TEMPLATE_FILE);
        if let Some(parent) = compose_path.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("create {}", parent.display()))?;
        }
        std::fs::write(&compose_path, Self::local_stack_template())
            .with_context(|| format!("write {}", compose_path.display()))?;
        Ok(constants::ENCLAVE_LOCAL_STACK_TEMPLATE_FILE)
    }
}
