mod build;
mod describe;

use anyhow::{Context, Result, bail};
use config::NitrumConfig;
use std::path::{Path, PathBuf};

use describe::describe_eif_json;

pub use build::{build_enclave_eif, build_enclave_image};
pub use describe::describe_eif;

#[derive(Debug, Clone)]
pub struct EnclaveArtifact {
    /// Path to the EIF file on disk.
    pub eif_path: PathBuf,
    /// SHA-256 digest of the EIF bytes.
    pub hash: String,
    /// PCR0 measurement from `describe-eif`.
    pub pcr0: String,
    /// PCR1 measurement from `describe-eif`.
    pub pcr1: String,
    /// PCR2 measurement from `describe-eif`.
    pub pcr2: String,
}

impl EnclaveArtifact {
    /// Build or load an enclave artifact from the given path.
    ///
    /// - If `path` is a directory: build enclave image + EIF from project source, then resolve hash and PCRs.
    /// - If `path` is a file: treat it as an existing EIF and resolve hash and PCRs.
    ///
    /// # Errors
    ///
    /// Returns an error when the path cannot be read, Docker image or EIF
    /// builds fail, or `nitro-cli describe-eif` cannot be executed or parsed.
    pub async fn try_from(path: &Path, cfg: &NitrumConfig) -> Result<Self> {
        let canonical = path
            .canonicalize()
            .with_context(|| format!("path not found or not readable: {}", path.display()))?;

        let eif_path = if canonical.is_dir() {
            let image_tag = format!("nitrum-{}:latest", cfg.project.name);
            let eif_path = project_eif_path(&canonical, &cfg.project.name);
            build::build_enclave_image(
                &canonical,
                cfg.project.dockerfile_path(),
                cfg.runtime.data_plane.as_str(),
                &image_tag,
            )
            .await?;
            build::build_enclave_eif(
                &canonical,
                &image_tag,
                &eif_path,
                cfg.runtime.nitro_cli.as_str(),
            )
            .await?;
            eif_path
        } else if canonical.is_file() {
            canonical
        } else {
            bail!(
                "path is neither a directory nor a file: {}",
                canonical.display()
            );
        };

        let hash = describe::sha256_file(&eif_path).await?;
        let describe_json = describe_eif_json(cfg, &eif_path).await?;
        let pcr0 = describe::extract_pcr(&describe_json, "PCR0")?;
        let pcr1 = describe::extract_pcr(&describe_json, "PCR1")?;
        let pcr2 = describe::extract_pcr(&describe_json, "PCR2")?;

        Ok(Self {
            eif_path,
            hash,
            pcr0,
            pcr1,
            pcr2,
        })
    }
}

/// Default EIF output path for a Nitrum project: `.nitrum/artifacts/{project_name}.eif`.
#[must_use]
pub fn project_eif_path(project_root: &Path, project_name: &str) -> PathBuf {
    project_root
        .join(".nitrum")
        .join("artifacts")
        .join(format!("{project_name}.eif"))
}
