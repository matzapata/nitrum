//! Resolve `registry/repo:tag` to `registry/repo@sha256:…` via the OCI/Distribution registry API.

use anyhow::{Context, Result, bail};
use reqwest::Client;
use reqwest::header::{ACCEPT, AUTHORIZATION};
use std::time::Duration;

const MANIFEST_ACCEPT: &str = concat!(
    "application/vnd.docker.distribution.manifest.list.v2+json,",
    "application/vnd.oci.image.index.v1+json,",
    "application/vnd.docker.distribution.manifest.v2+json,",
    "application/vnd.oci.image.manifest.v1+json",
);

/// HTTP client configured for registry manifest requests (`nitrum init` digest pinning).
pub struct ImageDigestResolver {
    client: Client,
}

impl ImageDigestResolver {
    /// Builds a client with a Nitrum `User-Agent` and a 60s request timeout.
    pub fn new() -> Result<Self> {
        let client = Client::builder()
            .user_agent(concat!("nitrum/", env!("CARGO_PKG_VERSION")))
            .timeout(Duration::from_secs(60))
            .build()
            .context("build HTTP client for container registry")?;
        Ok(Self { client })
    }

    /// Returns `image_ref` unchanged if it already contains `@sha256:`.
    pub async fn resolve(&self, image_ref: &str) -> Result<String> {
        let image_ref = image_ref.trim();
        if image_ref.contains("@sha256:") {
            return Ok(image_ref.to_string());
        }

        let (registry, repository, tag) = parse_repository_tag(image_ref)
            .with_context(|| format!("parse image reference {image_ref:?}"))?;

        let digest = fetch_manifest_digest(&self.client, registry, repository, tag)
            .await
            .with_context(|| format!("resolve digest for {image_ref}"))?;

        Ok(format!("{registry}/{repository}@{digest}"))
    }
}

fn parse_repository_tag(image_ref: &str) -> Result<(&str, &str, &str)> {
    if image_ref.contains('@') {
        bail!("expected a tagged reference (e.g. :latest), not a digest reference");
    }
    let (name, tag) = image_ref.rsplit_once(':').ok_or_else(|| {
        anyhow::anyhow!("image ref must include a tag (e.g. ghcr.io/org/image:latest)")
    })?;
    let slash = name
        .find('/')
        .ok_or_else(|| anyhow::anyhow!("image ref must be registry/repository:tag"))?;
    let registry = &name[..slash];
    let repository = &name[slash + 1..];
    if repository.is_empty() {
        bail!("image ref must include a repository path under the registry");
    }
    Ok((registry, repository, tag))
}

fn digest_from_response(resp: &reqwest::Response, url: &str) -> Result<String> {
    let digest = resp
        .headers()
        .get("docker-content-digest")
        .and_then(|v| v.to_str().ok())
        .filter(|s| !s.is_empty())
        .ok_or_else(|| {
            anyhow::anyhow!("registry response missing Docker-Content-Digest header for {url}")
        })?;
    Ok(digest.to_string())
}

async fn fetch_manifest_digest(
    client: &Client,
    registry: &str,
    repository: &str,
    tag: &str,
) -> Result<String> {
    let url = format!("https://{registry}/v2/{repository}/manifests/{tag}");

    let mut resp = manifest_get(client, &url, None)
        .await
        .with_context(|| format!("GET {url}"))?;

    if resp.status() == reqwest::StatusCode::UNAUTHORIZED && registry == "ghcr.io" {
        let token = ghcr_token(client, repository)
            .await
            .context("fetch anonymous ghcr.io registry token")?;
        resp = manifest_get(client, &url, Some(&token))
            .await
            .with_context(|| format!("GET {url} (with token)"))?;
    }

    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        bail!("registry returned {status} for {url}: {body}");
    }

    digest_from_response(&resp, &url)
}

async fn manifest_get(
    client: &Client,
    url: &str,
    bearer: Option<&str>,
) -> Result<reqwest::Response> {
    let mut req = client.get(url).header(ACCEPT, MANIFEST_ACCEPT);
    if let Some(t) = bearer {
        req = req.header(AUTHORIZATION, format!("Bearer {t}"));
    }
    Ok(req.send().await?)
}

async fn ghcr_token(client: &Client, repository: &str) -> Result<String> {
    let token_url =
        format!("https://ghcr.io/token?service=ghcr.io&scope=repository:{repository}:pull");
    let resp = client
        .get(&token_url)
        .send()
        .await
        .with_context(|| format!("GET {token_url}"))?;
    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        bail!("ghcr token endpoint returned {status}: {body}");
    }
    let v: serde_json::Value = resp.json().await.context("parse ghcr token JSON")?;
    v.get("token")
        .and_then(|t| t.as_str())
        .map(str::to_owned)
        .ok_or_else(|| anyhow::anyhow!("ghcr token response missing \"token\""))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_repository_tag_splits_ghcr() {
        let (r, repo, t) =
            parse_repository_tag("ghcr.io/matzapata/nitrum/data-plane:latest").unwrap();
        assert_eq!(r, "ghcr.io");
        assert_eq!(repo, "matzapata/nitrum/data-plane");
        assert_eq!(t, "latest");
    }
}
