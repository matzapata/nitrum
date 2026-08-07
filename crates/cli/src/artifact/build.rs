use anyhow::{Context, Result, bail};
use std::path::Path;
use std::process::Stdio;
use tokio::fs;
use tokio::process::Command;

/// Build the project Dockerfile with a given data-plane base image and local tag (quiet).
///
/// Build context is always the project directory (where `Dockerfile` and `nitrum.toml` live).
/// The image should include `nitrum.toml`; the data-plane reads fixed SSM paths from `project.name`.
///
/// # Errors
///
/// Returns an error when `docker build` fails to start or exits with a
/// non-success status.
pub async fn build_enclave_image(
    root: &Path,
    data_plane_image: &str,
    image_tag: &str,
) -> Result<()> {
    let root = root
        .canonicalize()
        .with_context(|| format!("canonicalize {}", root.display()))?;
    let dockerfile = root.join("Dockerfile");
    if !dockerfile.is_file() {
        bail!(
            "Dockerfile not found at {}\n\
             Run this from your Nitrum project root (where the Dockerfile lives).",
            dockerfile.display()
        );
    }

    let output = Command::new("docker")
        .current_dir(&root)
        // BuildKit / buildx may try to pull local-only tags from a registry.
        // Disable it so tags such as `…:latest-local` work as `FROM` inputs.
        .env("DOCKER_BUILDKIT", "0")
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

/// Run `nitro-cli build-enclave` inside `nitro_cli_image`, writing the EIF to `eif_path`.
///
/// Requires a Docker socket mount: the CLI container talks to the host daemon,
/// so `docker_uri` must be an image available there (e.g. `nitrum-{name}:latest` from [`build_enclave_image`]).
///
/// # Errors
///
/// Returns an error when the host or container paths cannot be resolved,
/// directories cannot be created, or the `docker run` for `nitro-cli` fails.
pub async fn build_enclave_eif(
    project_root: &Path,
    docker_uri: &str,
    eif_path: &Path,
    nitro_cli_image: &str,
) -> Result<()> {
    let host_dir = project_root.canonicalize().with_context(|| {
        format!(
            "could not resolve project directory {}",
            project_root.display()
        )
    })?;
    let output_path = if eif_path.is_absolute() {
        eif_path.to_path_buf()
    } else {
        host_dir.join(eif_path)
    };
    let output_dir = output_path.parent().with_context(|| {
        format!(
            "EIF output path must have a parent directory: {}",
            output_path.display()
        )
    })?;
    fs::create_dir_all(output_dir).await.with_context(|| {
        format!(
            "failed to create EIF artifact directory {}",
            output_dir.display()
        )
    })?;
    let output_file_name = output_path
        .file_name()
        .and_then(|n| n.to_str())
        .context("EIF output path must end with a file name")?;

    let output = Command::new("docker")
        .arg("run")
        .arg("--rm")
        .arg("--platform")
        .arg("linux/amd64")
        .arg("-v")
        .arg("/var/run/docker.sock:/var/run/docker.sock")
        .arg("-v")
        .arg(format!("{}:/output", output_dir.display()))
        .arg(nitro_cli_image)
        .arg("build-enclave")
        .arg("--docker-uri")
        .arg(docker_uri)
        .arg("--output-file")
        .arg(format!("/output/{output_file_name}"))
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
