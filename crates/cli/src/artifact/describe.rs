use anyhow::{Context, Result, bail};
use config::NitrumConfig;
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::path::Path;
use std::process::Stdio;
use tokio::fs;
use tokio::io::AsyncWriteExt;
use tokio::process::Command;

/// Run `nitro-cli describe-eif` using the configured Nitro CLI image; prints JSON to stdout.
///
/// # Errors
///
/// Returns an error when the EIF cannot be resolved, `nitro-cli describe-eif`
/// fails, or the JSON output cannot be formatted or written to stdout.
pub async fn describe_eif(cfg: &NitrumConfig, eif_path: &Path) -> Result<()> {
    let describe_json = describe_eif_json(cfg, eif_path).await?;
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

pub async fn describe_eif_json(cfg: &NitrumConfig, eif_path: &Path) -> Result<Value> {
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
        .arg(cfg.runtime.nitro_cli.as_str())
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

pub fn extract_pcr(v: &Value, key: &str) -> Result<String> {
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

pub async fn sha256_file(path: &Path) -> Result<String> {
    let bytes = fs::read(path)
        .await
        .with_context(|| format!("failed to read EIF file {}", path.display()))?;
    let mut hasher = Sha256::new();
    hasher.update(&bytes);
    let digest = hasher.finalize();
    Ok(format!("{digest:x}"))
}
