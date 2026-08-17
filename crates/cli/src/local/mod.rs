//! Local development utilities

use anyhow::{Context, Result, bail};
use config::NitrumConfig;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use tokio::process::Command;

use crate::artifact::build_enclave_image;
use crate::cloud::template::{warn_template_skew, write_bundled_template};
use crate::constants::{ENCLAVE_LOCAL_EJECT_TEMPLATE_FILE, ENCLAVE_LOCAL_STACK_TEMPLATE_FILE};

/// Bundled `local-stack.yml` compiled into the CLI.
#[must_use]
pub const fn bundled_local_stack_template() -> &'static str {
    include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/template/local-stack.yml"
    ))
}

/// Write the bundled Compose template to `dest`.
///
/// # Errors
///
/// See [`write_bundled_template`].
pub fn eject_local_template(dest: &Path, force: bool) -> Result<()> {
    write_bundled_template(dest, force, bundled_local_stack_template())
}

/// Fail if the default eject path exists but `[local].template` is unset.
pub fn reject_unejected_default_local_template(
    project_root: &Path,
    template: Option<&Path>,
) -> Result<()> {
    if template.is_some() {
        return Ok(());
    }
    let ejected = project_root.join(ENCLAVE_LOCAL_EJECT_TEMPLATE_FILE);
    if ejected.is_file() {
        bail!(
            "found `{ENCLAVE_LOCAL_EJECT_TEMPLATE_FILE}` but `[local].template` is unset; \
             add this to nitrum.toml so local uses your ejected Compose file (the CLI would \
             otherwise keep rewriting `.nitrum/local-stack.yml` from the bundled template):\n\n\
             [local]\n\
             template = \"{ENCLAVE_LOCAL_EJECT_TEMPLATE_FILE}\"\n"
        );
    }
    Ok(())
}

pub struct EnclaveLocalStack<'a> {
    /// Project directory containing `nitrum.toml` and the enclave `Dockerfile`.
    project_root: &'a Path,
    /// Optional project-relative Compose template from `[local].template`.
    template: Option<&'a Path>,
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
    /// Returns an error when the project path cannot be used to construct the stack,
    /// or when `infra/local-stack.yml` exists without `[local].template`.
    pub fn new(project_root: &'a Path, cfg: &'a NitrumConfig) -> Result<Self> {
        reject_unejected_default_local_template(project_root, cfg.local.template.as_deref())?;
        let ram_mib = cfg.scaling.ram_size_mib.get();
        Ok(Self {
            project_root,
            template: cfg.local.template.as_deref(),
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

        let compose_file = self.resolve_compose_file()?;

        let mut cmd = Command::new("docker");
        cmd.current_dir(self.project_root);
        self.apply_stack_env(&mut cmd)
            .env("DOCKER_DEFAULT_PLATFORM", "linux/amd64")
            .arg("compose")
            .arg("--progress")
            .arg("quiet")
            .arg("-f")
            .arg(&compose_file)
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
        let compose_file = self.resolve_compose_file()?;

        let mut cmd = Command::new("docker");
        cmd.current_dir(self.project_root);
        self.apply_stack_env(&mut cmd)
            .env("DOCKER_DEFAULT_PLATFORM", "linux/amd64")
            .arg("compose")
            .arg("--progress")
            .arg("quiet")
            .arg("-f")
            .arg(&compose_file)
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
        let compose_file = self.resolve_compose_file()?;

        let mut cmd = Command::new("docker");
        cmd.current_dir(self.project_root);
        self.apply_stack_env(&mut cmd)
            .env("DOCKER_DEFAULT_PLATFORM", "linux/amd64")
            .arg("compose")
            .arg("-f")
            .arg(&compose_file)
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

    /// Resolve the Compose file: custom `[local].template` (never overwritten) or
    /// rewrite [`.nitrum/local-stack.yml`](ENCLAVE_LOCAL_STACK_TEMPLATE_FILE).
    fn resolve_compose_file(&self) -> Result<PathBuf> {
        if let Some(rel) = self.template {
            let path = self.project_root.join(rel);
            let yaml = std::fs::read_to_string(&path)
                .with_context(|| format!("read Compose template {}", path.display()))?;
            warn_template_skew(&yaml, true, "Compose", "nitrum local eject");
            return Ok(PathBuf::from(rel));
        }

        let compose_path = self.project_root.join(ENCLAVE_LOCAL_STACK_TEMPLATE_FILE);
        if let Some(parent) = compose_path.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("create {}", parent.display()))?;
        }
        std::fs::write(&compose_path, bundled_local_stack_template())
            .with_context(|| format!("write {}", compose_path.display()))?;
        Ok(PathBuf::from(ENCLAVE_LOCAL_STACK_TEMPLATE_FILE))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cloud::template::{BUNDLED_CLOUD_TEMPLATE_VERSION, parse_template_version};
    use config::NitrumConfig;
    use std::path::Path;
    use std::time::{SystemTime, UNIX_EPOCH};

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

    #[test]
    fn bundled_has_version_marker() {
        assert_eq!(
            parse_template_version(bundled_local_stack_template()),
            Some(BUNDLED_CLOUD_TEMPLATE_VERSION)
        );
    }

    #[test]
    fn eject_refuses_without_force() {
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("time")
            .as_nanos();
        let dir = std::env::temp_dir().join(format!("nitrum-local-eject-{stamp}"));
        std::fs::create_dir_all(&dir).expect("mkdir");
        let dest = dir.join("local-stack.yml");
        eject_local_template(&dest, false).expect("first write");
        let err = eject_local_template(&dest, false).expect_err("second write");
        assert!(err.to_string().contains("--force"));
        eject_local_template(&dest, true).expect("force overwrite");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn reject_unejected_when_default_file_exists() {
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("time")
            .as_nanos();
        let root = std::env::temp_dir().join(format!("nitrum-local-uneject-{stamp}"));
        std::fs::create_dir_all(root.join("infra")).expect("mkdir");
        std::fs::write(root.join("infra/local-stack.yml"), "services: {}\n").expect("write");
        let err = reject_unejected_default_local_template(&root, None).expect_err("must fail");
        assert!(err.to_string().contains("local].template"));
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn resolve_custom_template_does_not_rewrite() {
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("time")
            .as_nanos();
        let root = std::env::temp_dir().join(format!("nitrum-local-custom-{stamp}"));
        let infra = root.join("infra");
        std::fs::create_dir_all(&infra).expect("mkdir");
        let custom = "# nitrum-template-version: 0.3.0\nservices: { custom: true }\n";
        std::fs::write(infra.join("local-stack.yml"), custom).expect("write");

        let mut cfg: NitrumConfig =
            toml::from_str(include_str!("../../../../examples/hello/nitrum.toml"))
                .expect("hello nitrum.toml");
        cfg.local.template = Some(PathBuf::from("infra/local-stack.yml"));

        let stack = EnclaveLocalStack::new(&root, &cfg).expect("stack");
        let rel = stack.resolve_compose_file().expect("resolve");
        assert_eq!(rel, PathBuf::from("infra/local-stack.yml"));
        let after = std::fs::read_to_string(infra.join("local-stack.yml")).expect("read");
        assert_eq!(after, custom);
        assert!(!root.join(ENCLAVE_LOCAL_STACK_TEMPLATE_FILE).exists());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn resolve_unmanaged_rewrites_dot_nitrum() {
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("time")
            .as_nanos();
        let root = std::env::temp_dir().join(format!("nitrum-local-rewrite-{stamp}"));
        std::fs::create_dir_all(&root).expect("mkdir");
        let cfg: NitrumConfig =
            toml::from_str(include_str!("../../../../examples/hello/nitrum.toml"))
                .expect("hello nitrum.toml");
        let stack = EnclaveLocalStack::new(&root, &cfg).expect("stack");
        let rel = stack.resolve_compose_file().expect("resolve");
        assert_eq!(rel, PathBuf::from(ENCLAVE_LOCAL_STACK_TEMPLATE_FILE));
        let written =
            std::fs::read_to_string(root.join(ENCLAVE_LOCAL_STACK_TEMPLATE_FILE)).expect("read");
        assert_eq!(written, bundled_local_stack_template());
        let _ = std::fs::remove_dir_all(&root);
    }
}
