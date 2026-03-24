use anyhow::{Context, Result, bail};
use std::path::Path;
use std::process::Stdio;
use tokio::process::Command;

use crate::constants;

/// Build the project Dockerfile with a given data-plane base image and local tag (quiet).
///
/// The image should include `nitrum.toml`; the data-plane reads SSM paths from env or defaults (see `crates/data-plane/src/utils/ssm.rs`).
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

    let status = Command::new("docker")
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
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .status()
        .await
        .context("failed to spawn nitro-cli describe-eif (`docker run` …)")?;

    if !status.success() {
        bail!("nitro-cli describe-eif exited with status {status}");
    }
    Ok(())
}
