//! Runtime configuration for the control-plane process.

use anyhow::{Result, bail};
use std::path::PathBuf;

/// Source of the EIF the control-plane should run.
pub enum EifSource {
    /// Local EIF path (`--eif`).
    Local(PathBuf),
    /// S3 bucket and version label (`--eif-bucket` + `--eif-hash`).
    Bucket {
        /// S3 bucket containing the EIF object.
        bucket: String,
        /// Version label (first 12 hex chars of EIF SHA-256 from deploy).
        version_label: String,
    },
}

/// Resolved control-plane runtime settings.
pub struct ControlPlaneConfig {
    /// EIF to pass to `nitro-cli run-enclave`.
    pub eif: EifSource,
    /// Passed to `nitro-cli run-enclave --debug-mode`.
    pub debug_mode: bool,
    /// vCPU count for `--cpu-count`.
    pub cpu_count: u32,
    /// Memory in MiB for `--memory`.
    pub memory_mib: u32,
}

impl ControlPlaneConfig {
    /// Builds runtime config from parsed CLI flags.
    ///
    /// # Errors
    ///
    /// Returns an error when EIF flags are missing or mutually exclusive.
    pub fn from_cli(
        eif: Option<PathBuf>,
        eif_bucket: Option<String>,
        eif_hash: Option<String>,
        debug_mode: bool,
        cpu_count: u32,
        memory_mib: u32,
    ) -> Result<Self> {
        let eif = match (eif, eif_bucket, eif_hash) {
            (Some(path), None, None) => EifSource::Local(path),
            (None, Some(bucket), Some(version_label)) => EifSource::Bucket {
                bucket,
                version_label,
            },
            (None, None, None) => {
                bail!("provide --eif PATH or both --eif-bucket and --eif-hash");
            }
            _ => bail!("invalid EIF flags combination"),
        };

        Ok(Self {
            eif,
            debug_mode,
            cpu_count,
            memory_mib,
        })
    }
}
