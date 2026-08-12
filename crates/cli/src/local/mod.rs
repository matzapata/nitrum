//! Local development utilities

use anyhow::{Context, Result, bail};
use config::NitrumConfig;
use std::path::Path;
use std::process::Stdio;
use tokio::process::Command;

use crate::artifact::build_enclave_image;
use crate::constants;

pub struct EnclaveLocalStack<'a> {
    /// Project directory containing `nitrum.toml` and the enclave `Dockerfile`.
    project_root: &'a Path,
    /// Local compose tag for the app image (`nitrum-{name}:dev`).
    enclave_image: String,
    /// Base image for the enclave Dockerfile (`DATA_PLANE_IMAGE`);
    /// derived from `[runtime].data_plane` with a `-local` tag suffix.
    data_plane_image: String,
    /// From `[scaling].num_cpus` — Compose `cpus` on the enclave service.
    enclave_cpus: u32,
    /// From `[scaling].ram_size_mib` — Compose `mem_limit` (Docker `Nm` form).
    enclave_memory: String,
}

impl<'a> EnclaveLocalStack<'a> {
    /// # Errors
    ///
    /// Returns an error when the project path cannot be used to construct the stack.
    pub fn new(project_root: &'a Path, cfg: &NitrumConfig) -> Result<Self> {
        let ram_mib = cfg.scaling.ram_size_mib.get();
        Ok(Self {
            project_root,
            enclave_image: format!("nitrum-{}:dev", cfg.project.name),
            data_plane_image: cfg.runtime.data_plane.with_tag_suffix("local").to_string(),
            enclave_cpus: cfg.scaling.num_cpus.get(),
            enclave_memory: format!("{ram_mib}m"),
        })
    }

    /// Enclave Compose resource limits from `[scaling]` (for status messages).
    #[must_use]
    pub const fn resource_limits(&self) -> (u32, &str) {
        (self.enclave_cpus, self.enclave_memory.as_str())
    }

    fn apply_stack_env<'cmd>(&self, cmd: &'cmd mut Command) -> &'cmd mut Command {
        cmd.env("ENCLAVE_IMAGE", &self.enclave_image)
            .env("ENCLAVE_CPUS", self.enclave_cpus.to_string())
            .env("ENCLAVE_MEMORY", &self.enclave_memory)
    }

    /// Start the local stack (build enclave image, then `docker compose up`).
    pub async fn up(&self) -> Result<()> {
        // Compose only runs a pre-built image; build here so layout stays
        // project-dir + Dockerfile (same as `nitrum build` / customer projects).
        build_enclave_image(
            self.project_root,
            &self.data_plane_image,
            &self.enclave_image,
        )
        .await?;

        let compose_file = self.ensure_template()?;

        let mut cmd = Command::new("docker");
        cmd.current_dir(self.project_root);
        self.apply_stack_env(&mut cmd)
            .env("DOCKER_DEFAULT_PLATFORM", "linux/amd64")
            .arg("compose")
            .arg("--progress")
            .arg("quiet")
            .arg("-f")
            .arg(compose_file)
            .args(["up", "-d"])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
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
        self.apply_stack_env(&mut cmd)
            .env("DOCKER_DEFAULT_PLATFORM", "linux/amd64")
            .arg("compose")
            .arg("--progress")
            .arg("quiet")
            .arg("-f")
            .arg(compose_file)
            .args(["down"])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
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
        self.apply_stack_env(&mut cmd)
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
        include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/template/local-stack.yml"
        ))
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

#[cfg(test)]
mod tests {
    use super::EnclaveLocalStack;
    use config::NitrumConfig;
    use std::path::Path;

    #[test]
    fn resource_limits_follow_scaling() {
        let mut cfg: NitrumConfig =
            toml::from_str(include_str!("../../../../examples/hello/nitrum.toml"))
                .expect("hello nitrum.toml");
        cfg.scaling.num_cpus = std::num::NonZeroU32::new(4).unwrap();
        cfg.scaling.ram_size_mib = std::num::NonZeroU32::new(8192).unwrap();

        let stack = EnclaveLocalStack::new(Path::new("/tmp"), &cfg).expect("stack");
        assert_eq!(stack.resource_limits(), (4, "8192m"));
    }
}
