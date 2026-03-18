use anyhow::Result;
use reqwest::blocking::Client;
use reqwest::header::{HeaderMap, HeaderValue, AUTHORIZATION, USER_AGENT};
use std::env;
use std::fs;
use std::io::Write;
use std::path::Path;
use tracing::info;

/// Downloads a full repo archive (tar.gz) at the given commit and saves it to `output_path`.
///
/// # Example
/// ```
/// download_repo("owner", "repo", "abc123def456", "repo.tar.gz")?;
/// ```
pub fn download_repo(owner: &str, name: &str, commit_hash: &str, output_path: &Path) -> Result<()> {
    // GitHub raw archive URL: /repos/{owner}/{repo}/tarball/{ref}
    let url = format!(
        "https://api.github.com/repos/{}/{}/tarball/{}",
        owner, name, commit_hash
    );

    let client = build_client()?;
    let response = client.get(&url).send()?.error_for_status()?;
    let bytes = response.bytes()?;

    if let Some(parent) = output_path.parent() {
        fs::create_dir_all(parent)?;
    }

    let mut file = fs::File::create(output_path)?;
    file.write_all(&bytes)?;

    Ok(())
}

/// Downloads a single raw file from a GitHub repo at the given commit and saves it to `output_path`.
///
/// # Example
/// ```
/// download_file("owner", "repo", "abc123def456", "src/main.rs", Path::new("main.rs"))?;
/// ```
pub fn download_file(
    owner: &str,
    name: &str,
    commit_hash: &str,
    file_path: &str,
    output_path: &Path,
) -> Result<()> {
    // raw.githubusercontent.com serves raw file content directly
    let url = format!(
        "https://raw.githubusercontent.com/{}/{}/{}/{}",
        owner, name, commit_hash, file_path
    );

    let client = build_client()?;
    let response = client.get(&url).send()?.error_for_status()?;
    let bytes = response.bytes()?;

    if let Some(parent) = output_path.parent() {
        fs::create_dir_all(parent)?;
    }

    let mut file = fs::File::create(output_path)?;
    file.write_all(&bytes)?;

    Ok(())
}

fn build_client() -> Result<Client> {
    let mut headers = HeaderMap::new();

    // GitHub requires a User-Agent header
    headers.insert(
        USER_AGENT,
        HeaderValue::from_static("rust-github-downloader"),
    );

    // Attach PAT if available via env var
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
