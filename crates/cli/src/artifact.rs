use anyhow::{Context, Result, bail};
use serde_json::Value;
use sha2::{Digest, Sha256};
use shared::config::NitrumConfig;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use tokio::process::Command;
use tokio::{fs, io::AsyncWriteExt};

use crate::constants;

#[derive(Debug, Clone)]
pub struct EnclaveArtifact {
    pub eif_path: PathBuf,
    pub hash: String,
    pub pcr0: String,
    pub pcr1: String,
    pub pcr2: String,
}

impl EnclaveArtifact {
    /// Build or load an enclave artifact from the given path.
    ///
    /// - If `path` is a directory: build enclave image + EIF from project source, then resolve hash and PCRs.
    /// - If `path` is a file: treat it as an existing EIF and resolve hash and PCRs.
    pub async fn try_from(path: &Path, cfg: &NitrumConfig) -> Result<Self> {
        let canonical = path
            .canonicalize()
            .with_context(|| format!("path not found or not readable: {}", path.display()))?;

        let eif_path = if canonical.is_dir() {
            let image_tag = format!("nitrum-{}:latest", cfg.name);
            build_enclave_image(&canonical, cfg.data_plane.as_str(), &image_tag).await?;
            build_enclave_eif(&canonical, &image_tag).await?;
            canonical.join("enclave.eif")
        } else if canonical.is_file() {
            canonical
        } else {
            bail!(
                "path is neither a directory nor a file: {}",
                canonical.display()
            );
        };

        let hash = sha256_file(&eif_path).await?;
        let describe_json = describe_eif_json(&eif_path).await?;
        let pcr0 = extract_pcr(&describe_json, "PCR0")?;
        let pcr1 = extract_pcr(&describe_json, "PCR1")?;
        let pcr2 = extract_pcr(&describe_json, "PCR2")?;

        Ok(Self {
            eif_path,
            hash,
            pcr0,
            pcr1,
            pcr2,
        })
    }
}

/// Build the project Dockerfile with a given data-plane base image and local tag (quiet).
///
/// The image should include `nitrum.toml`; the data-plane reads fixed SSM paths from `nitrum.toml` `name` (see `crates/data-plane/src/utils/ssm.rs`).
pub async fn build_enclave_image(
    root: &Path,
    data_plane_image: &str,
    image_tag: &str,
) -> Result<()> {
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
        .arg("--platform")
        .arg("linux/amd64")
        .arg("-f")
        .arg("Dockerfile")
        .arg("-t")
        .arg(image_tag)
        .arg("--build-arg")
        .arg(format!("DATA_PLANE_IMAGE={data_plane_image}"))
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

/// Run `nitro-cli build-enclave` inside [`constants::NITRO_CLI_DOCKER_IMAGE`], writing `enclave.eif` under `project_root`.
///
/// Requires a Docker socket mount (same pattern as the Nitrum README): the CLI container talks to the host daemon,
/// so `docker_uri` must be an image available there (e.g. `nitrum-{name}:latest` from [`build_enclave_image`]).
pub async fn build_enclave_eif(project_root: &Path, docker_uri: &str) -> Result<()> {
    let host_dir = project_root.canonicalize().with_context(|| {
        format!(
            "could not resolve project directory {}",
            project_root.display()
        )
    })?;

    let output = Command::new("docker")
        .arg("run")
        .arg("--rm")
        .arg("--platform")
        .arg("linux/amd64")
        .arg("-v")
        .arg("/var/run/docker.sock:/var/run/docker.sock")
        .arg("-v")
        .arg(format!("{}:/output", host_dir.display()))
        .arg(constants::NITRO_CLI_DOCKER_IMAGE)
        .arg("build-enclave")
        .arg("--docker-uri")
        .arg(docker_uri)
        .arg("--output-file")
        .arg("/output/enclave.eif")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .await
        .context("failed to spawn nitro-cli (`docker run` …)")?;

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
        "nitro-cli build-enclave failed ({}){}{}",
        output.status,
        if detail.is_empty() { "" } else { "\n\n" },
        detail,
    );
}

/// Run `nitro-cli describe-eif` in [`constants::NITRO_CLI_DOCKER_IMAGE`]; prints JSON to stdout.
pub async fn describe_eif(eif_path: &Path) -> Result<()> {
    let describe_json = describe_eif_json(eif_path).await?;
    let mut stdout = tokio::io::stdout();
    let mut rendered = serde_json::to_vec_pretty(&describe_json)
        .context("failed to format describe-eif output")?;
    rendered.push(b'\n');
    stdout
        .write_all(&rendered)
        .await
        .context("failed writing describe-eif output")?;
    Ok(())
}

async fn describe_eif_json(eif_path: &Path) -> Result<Value> {
    let eif_path = eif_path
        .canonicalize()
        .with_context(|| format!("EIF not found or path not readable: {}", eif_path.display()))?;
    if !eif_path.is_file() {
        bail!("not a file: {}", eif_path.display());
    }

    let parent = eif_path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    let parent = parent
        .canonicalize()
        .with_context(|| format!("could not resolve directory {}", parent.display()))?;

    let file_name = eif_path
        .file_name()
        .and_then(|n| n.to_str())
        .context("EIF path must end with a file name")?;
    let container_path = format!("/nitrum-eif/{file_name}");

    let output = Command::new("docker")
        .arg("run")
        .arg("--rm")
        .arg("--platform")
        .arg("linux/amd64")
        .arg("-v")
        .arg(format!("{}:/nitrum-eif:ro", parent.display()))
        .arg(constants::NITRO_CLI_DOCKER_IMAGE)
        .arg("describe-eif")
        .arg("--eif-path")
        .arg(&container_path)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .await
        .context("failed to spawn nitro-cli describe-eif (`docker run` …)")?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let stdout = String::from_utf8_lossy(&output.stdout);
        let detail = [&*stderr, &*stdout]
            .into_iter()
            .map(str::trim)
            .find(|s| !s.is_empty())
            .unwrap_or("");
        bail!(
            "nitro-cli describe-eif failed ({}){}{}",
            output.status,
            if detail.is_empty() { "" } else { "\n\n" },
            detail
        );
    }

    serde_json::from_slice::<Value>(&output.stdout).with_context(|| {
        let stdout = String::from_utf8_lossy(&output.stdout);
        format!(
            "failed to parse describe-eif JSON output: {}",
            stdout.trim()
        )
    })
}

fn extract_pcr(v: &Value, key: &str) -> Result<String> {
    let lower = key.to_ascii_lowercase();
    if let Some(s) = find_string_by_key(v, key).or_else(|| find_string_by_key(v, &lower)) {
        let normalized = s.trim();
        if !normalized.is_empty() {
            return Ok(normalized.to_string());
        }
    }
    bail!("{key} not found in describe-eif output")
}

fn find_string_by_key(v: &Value, key: &str) -> Option<String> {
    match v {
        Value::Object(map) => {
            for (k, val) in map {
                if k.eq_ignore_ascii_case(key)
                    && let Some(s) = val.as_str()
                {
                    return Some(s.to_string());
                }
                if let Some(found) = find_string_by_key(val, key) {
                    return Some(found);
                }
            }
            None
        }
        Value::Array(items) => items.iter().find_map(|item| find_string_by_key(item, key)),
        _ => None,
    }
}

async fn sha256_file(path: &Path) -> Result<String> {
    let bytes = fs::read(path)
        .await
        .with_context(|| format!("failed to read EIF file {}", path.display()))?;
    let mut hasher = Sha256::new();
    hasher.update(&bytes);
    let digest = hasher.finalize();
    Ok(format!("{digest:x}"))
}
