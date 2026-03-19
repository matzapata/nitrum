use anyhow::{Context, Result, bail};
use std::path::Path;
use std::process::Stdio;
use tokio::process::Command;

use crate::constants;

/// Build the project Dockerfile as [`constants::ENCLAVE_DEV_LOCAL_IMAGE`] (quiet; use a spinner in the caller).
pub async fn build_enclave(root: &Path) -> Result<()> {
    build_enclave_image(
        root,
        constants::ENCLAVE_DEV_BASE_IMAGE,
        constants::ENCLAVE_DEV_LOCAL_IMAGE,
    )
    .await
}

/// Build the project Dockerfile with a given data-plane base image and local tag (quiet).
/// Always targets [`constants::DOCKER_PLATFORM`].
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
        .arg(constants::DOCKER_PLATFORM)
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
/// so `docker_uri` must be an image available there (e.g. a local tag from [`build_enclave_image`]).
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
        .arg(constants::DOCKER_PLATFORM)
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
