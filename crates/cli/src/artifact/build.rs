use anyhow::{Context, Result, bail};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use tokio::fs;
use tokio::process::Command;

/// Build the project Dockerfile with a given data-plane base image and local tag (quiet).
///
/// The image should include `nitrum.toml`; the data-plane reads fixed SSM paths from `project.name`.
///
/// When the project lives inside a Nitrum Cargo workspace (in-repo examples), the Docker
/// build context is the workspace root so path deps like `crates/sdk` resolve. Otherwise
/// the project directory is the context (standalone `nitrum init` projects).
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
    let dockerfile = root.join("Dockerfile");
    if !dockerfile.is_file() {
        bail!(
            "Dockerfile not found at {}\n\
             Run this from your Nitrum project root (where the Dockerfile lives).",
            dockerfile.display()
        );
    }

    let (context_dir, dockerfile_arg) = resolve_docker_build_paths(root)?;

    let output = Command::new("docker")
        .current_dir(&context_dir)
        .arg("build")
        .arg("-q")
        .arg("--platform")
        .arg("linux/amd64")
        .arg("-f")
        .arg(&dockerfile_arg)
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

/// Choose Docker context and Dockerfile path relative to that context.
///
/// Used by [`build_enclave_image`] and `nitrum local` Compose builds.
///
/// Returns `(context_dir, dockerfile_path_relative_to_context)`.
pub fn resolve_docker_build_paths(project_root: &Path) -> Result<(PathBuf, String)> {
    let project_root = project_root
        .canonicalize()
        .with_context(|| format!("canonicalize {}", project_root.display()))?;

    if let Some(workspace_root) = find_nitrum_workspace_root(&project_root) {
        let rel = project_root
            .strip_prefix(&workspace_root)
            .with_context(|| {
                format!(
                    "project {} is not under workspace {}",
                    project_root.display(),
                    workspace_root.display()
                )
            })?;
        let dockerfile_rel = rel.join("Dockerfile");
        return Ok((
            workspace_root,
            dockerfile_rel
                .to_str()
                .ok_or_else(|| anyhow::anyhow!("non-UTF8 Dockerfile path"))?
                .replace('\\', "/"),
        ));
    }

    Ok((project_root, "Dockerfile".to_string()))
}

/// Walk parents for a `Cargo.toml` that both is a workspace and contains `crates/sdk`.
///
/// Projects living under a `target/` directory (e.g. e2e workspaces created at
/// `<repo>/target/nitrum-e2e-workspace/…`) must stay as standalone Docker contexts.
/// Walking past `target/` would incorrectly treat them as in-repo examples and send
/// the entire monorepo — including a huge `target/` tree — as the build context.
fn find_nitrum_workspace_root(start: &Path) -> Option<PathBuf> {
    for dir in start.ancestors() {
        if dir.file_name().is_some_and(|name| name == "target") {
            return None;
        }
        let manifest = dir.join("Cargo.toml");
        let Ok(text) = std::fs::read_to_string(&manifest) else {
            continue;
        };
        if text.contains("[workspace]") && dir.join("crates/sdk").is_dir() {
            return Some(dir.to_path_buf());
        }
    }
    None
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn in_repo_example_uses_workspace_root_context() {
        let workspace = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../..")
            .canonicalize()
            .unwrap();
        let example = workspace.join("examples/hello");
        let (ctx, dockerfile) = resolve_docker_build_paths(&example).unwrap();
        assert_eq!(ctx, workspace);
        assert_eq!(dockerfile, "examples/hello/Dockerfile");
    }

    #[test]
    fn project_under_target_stays_standalone_context() {
        let workspace = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../..")
            .canonicalize()
            .unwrap();
        let project = workspace.join("target/nitrum-docker-context-test/demo");
        fs::create_dir_all(&project).unwrap();
        fs::write(project.join("Dockerfile"), "FROM scratch\n").unwrap();
        let (ctx, dockerfile) = resolve_docker_build_paths(&project).unwrap();
        assert_eq!(ctx, project.canonicalize().unwrap());
        assert_eq!(dockerfile, "Dockerfile");
        let _ = fs::remove_dir_all(workspace.join("target/nitrum-docker-context-test"));
    }
}
