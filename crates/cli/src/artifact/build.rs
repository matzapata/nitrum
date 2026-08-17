use anyhow::{Context, Result, bail};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use tokio::fs;
use tokio::process::Command;

/// Resolve `dockerfile` against `root` and require it to exist as a file.
fn resolve_dockerfile(root: &Path, dockerfile: &Path) -> Result<PathBuf> {
    let path = root.join(dockerfile);
    if !path.is_file() {
        bail!(
            "Dockerfile not found at {}\n\
             Set `[project].dockerfile` in nitrum.toml to a project-relative path, \
             or place a Dockerfile at the project root.",
            path.display()
        );
    }
    Ok(path)
}

/// Build the project Dockerfile with a given data-plane base image and local tag (quiet).
///
/// Build context is always the project directory (where `nitrum.toml` lives).
/// `dockerfile` is a project-relative path (`Dockerfile` by default; see `[project].dockerfile`).
/// The image should include `nitrum.toml`; the data-plane reads fixed SSM paths from `project.name`.
///
/// # Errors
///
/// Returns an error when the Dockerfile is missing, or when `docker build` fails to
/// start or exits with a non-success status.
pub async fn build_enclave_image(
    root: &Path,
    dockerfile: &Path,
    data_plane_image: &str,
    image_tag: &str,
) -> Result<()> {
    let root = root
        .canonicalize()
        .with_context(|| format!("canonicalize {}", root.display()))?;
    let dockerfile_path = resolve_dockerfile(&root, dockerfile)?;

    // BuildKit is required for `--platform linux/amd64` cross-builds (e.g. Apple Silicon).
    // Local-only `FROM` tags (such as `…:dev-local`) resolve from the daemon.
    let output = Command::new("docker")
        .current_dir(&root)
        .arg("build")
        .arg("-q")
        .arg("--platform")
        .arg("linux/amd64")
        .arg("-f")
        .arg(&dockerfile_path)
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

#[cfg(test)]
mod tests {
    use super::resolve_dockerfile;
    use std::path::Path;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temp_root(prefix: &str) -> std::path::PathBuf {
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("time")
            .as_nanos();
        let dir = std::env::temp_dir().join(format!("{prefix}-{stamp}"));
        std::fs::create_dir_all(&dir).expect("mkdir");
        dir
    }

    #[test]
    fn resolve_default_dockerfile() {
        let root = temp_root("nitrum-df-default");
        std::fs::write(root.join("Dockerfile"), "FROM scratch\n").expect("write");
        let path = resolve_dockerfile(&root, Path::new("Dockerfile")).expect("exists");
        assert_eq!(path, root.join("Dockerfile"));
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn resolve_nested_dockerfile() {
        let root = temp_root("nitrum-df-nested");
        std::fs::create_dir_all(root.join("docker")).expect("mkdir");
        std::fs::write(root.join("docker/app.Dockerfile"), "FROM scratch\n").expect("write");
        let path = resolve_dockerfile(&root, Path::new("docker/app.Dockerfile")).expect("exists");
        assert_eq!(path, root.join("docker/app.Dockerfile"));
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn resolve_missing_mentions_config_key() {
        let root = temp_root("nitrum-df-missing");
        let err =
            resolve_dockerfile(&root, Path::new("docker/app.Dockerfile")).expect_err("missing");
        let msg = err.to_string();
        assert!(msg.contains("docker/app.Dockerfile"));
        assert!(msg.contains("[project].dockerfile"));
        let _ = std::fs::remove_dir_all(&root);
    }
}
