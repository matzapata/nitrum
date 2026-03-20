use anyhow::{Context, Result, bail};
use flate2::read::GzDecoder;
use reqwest::Client;
use reqwest::header::{AUTHORIZATION, HeaderMap, HeaderValue, USER_AGENT};
use std::env;
use std::fs;
use std::io::{Cursor, Write};
use std::path::{Component, Path};
use tar::Archive;
use tracing::info;

/// Downloads a single raw file from a GitHub repo at the given commit and saves it to `output_path`.
///
/// # Example
/// ```ignore
/// download_file("owner", "repo", "abc123def456", "src/main.rs", Path::new("main.rs")).await?;
/// ```
pub async fn download_file(
    owner: &str,
    name: &str,
    commit_hash: &str,
    file_path: &str,
    output_path: &Path,
) -> Result<()> {
    let url = format!(
        "https://raw.githubusercontent.com/{}/{}/{}/{}",
        owner, name, commit_hash, file_path
    );

    let client = build_client()?;
    let response = client.get(&url).send().await?.error_for_status()?;
    let bytes = response.bytes().await?;

    if let Some(parent) = output_path.parent() {
        fs::create_dir_all(parent)?;
    }

    let mut file = fs::File::create(output_path)?;
    file.write_all(&bytes)?;

    Ok(())
}

fn build_client() -> Result<Client> {
    let mut headers = HeaderMap::new();

    headers.insert(
        USER_AGENT,
        HeaderValue::from_static("rust-github-downloader"),
    );

    if let Ok(token) = env::var("GITHUB_TOKEN") {
        info!("Using GitHub token from environment variable");
        let auth_value = format!("Bearer {}", token);
        headers.insert(
            AUTHORIZATION,
            HeaderValue::from_str(&auth_value).expect("Invalid token format"),
        );
    }

    Ok(Client::builder().default_headers(headers).build()?)
}

fn is_safe_rel_path(rel: &str) -> bool {
    Path::new(rel)
        .components()
        .all(|c| matches!(c, Component::Normal(_)))
}

/// Downloads the repo tarball for `git_ref` and copies only the `infra/` tree into `dest`
/// (no top-level `{owner}-{repo}-{sha}/` prefix).
pub async fn extract_infra_folder(
    owner: &str,
    name: &str,
    git_ref: &str,
    dest: &Path,
) -> Result<()> {
    let url = format!(
        "https://api.github.com/repos/{}/{}/tarball/{}",
        owner, name, git_ref
    );

    let client = build_client()?;
    let response = client
        .get(&url)
        .send()
        .await
        .context("failed to download GitHub tarball")?
        .error_for_status()
        .context("GitHub returned an error for the tarball URL")?;
    let bytes = response
        .bytes()
        .await
        .context("failed to read tarball body")?;

    if dest.exists() {
        fs::remove_dir_all(dest).with_context(|| format!("could not remove {}", dest.display()))?;
    }
    fs::create_dir_all(dest).with_context(|| format!("could not create {}", dest.display()))?;

    let decoder = GzDecoder::new(Cursor::new(bytes.as_ref()));
    let mut archive = Archive::new(decoder);

    let mut extracted_any = false;

    for entry in archive.entries().context("failed to read tarball entries")? {
        let mut entry = entry.context("bad tarball entry")?;
        if entry.header().entry_type().is_symlink() {
            continue;
        }

        let path = entry.path().context("bad path in tarball")?.into_owned();
        let path_str = path.to_string_lossy();
        let needle = "/infra/";
        let Some(idx) = path_str.find(needle) else {
            continue;
        };
        let rel = path_str[idx + needle.len()..].trim_start_matches('/');

        if rel.is_empty() {
            continue;
        }

        if !is_safe_rel_path(rel) {
            continue;
        }

        let out_path = dest.join(rel);

        if entry.header().entry_type().is_dir() {
            fs::create_dir_all(&out_path)?;
            extracted_any = true;
        } else {
            if let Some(parent) = out_path.parent() {
                fs::create_dir_all(parent)?;
            }
            entry
                .unpack(&out_path)
                .with_context(|| format!("unpack failed for {}", out_path.display()))?;
            extracted_any = true;
        }
    }

    if !extracted_any {
        bail!(
            "no `infra/` directory found in {owner}/{name} @ {git_ref}; check the ref and repo layout"
        );
    }

    Ok(())
}
