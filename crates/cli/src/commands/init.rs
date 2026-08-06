use crate::utils;
use crate::utils::image_digest::ImageDigestResolver;
use anyhow::{Context, Result, bail};
use clap::Args;
use config::{
    DockerImageRef, Egress, EgressPattern, HealthCheck, NitrumConfig, Project, ProjectName,
    Runtime, Scaling, TlsTermination, WellKnown,
};
use futures_util::future::try_join3;
use indicatif::ProgressBar;
use std::env;
use std::fs;
use std::path::Path;

/// Matches `.github/workflows/release.yml` (`GHCR_PREFIX` = `ghcr.io/<github.repository>`).
const DEFAULT_DATA_PLANE: &str = "ghcr.io/matzapata/nitrum/data-plane:latest";
const DEFAULT_CONTROL_PLANE: &str = "ghcr.io/matzapata/nitrum/control-plane:latest";
const DEFAULT_NITRO_CLI: &str = "ghcr.io/matzapata/nitrum/nitro-cli:latest";

#[derive(Args)]
pub struct InitArgs {
    /// Project name; creates a subdirectory with this name in the current directory
    #[arg(value_name = "NAME", default_value = "nitrum-hello")]
    pub name: String,
}

pub async fn run(args: InitArgs) -> Result<()> {
    let project_name: ProjectName = args.name.parse().map_err(|error| {
        anyhow::anyhow!(
            "{error} (project directory name is used as `project.name` in nitrum.toml for `nitrum cloud deploy`)"
        )
    })?;

    let cwd = env::current_dir().expect("current directory");
    let directory = cwd.join(&args.name);
    if directory.exists()
        && directory
            .read_dir()
            .with_context(|| format!("read {}", directory.display()))?
            .next()
            .is_some()
    {
        bail!("Directory is not empty");
    }

    fs::create_dir_all(directory.join("src"))
        .with_context(|| format!("create {}", directory.join("src").display()))?;
    fs::create_dir_all(directory.join("tests"))
        .with_context(|| format!("create {}", directory.join("tests").display()))?;

    let spinner = utils::style_spinner(
        ProgressBar::new_spinner(),
        "Resolving runtime image digests…",
    );

    let resolver = ImageDigestResolver::new().context("container registry HTTP client")?;

    let (data_plane, control_plane, nitro_cli) = try_join3(
        resolver.resolve(DEFAULT_DATA_PLANE),
        resolver.resolve(DEFAULT_CONTROL_PLANE),
        resolver.resolve(DEFAULT_NITRO_CLI),
    )
    .await
    .context("pin runtime images to registry digests (needs network access to ghcr.io)")?;

    spinner.set_message("Writing bundled sample project…");

    let nitrum_config = NitrumConfig {
        project: Project {
            name: project_name,
            port: std::num::NonZeroU16::new(8080).expect("8080 is non-zero"),
            start_command: vec!["/app/hello".to_string()],
        },
        runtime: Runtime {
            data_plane: DockerImageRef::try_new(&data_plane)?,
            control_plane: DockerImageRef::try_new(&control_plane)?,
            nitro_cli: DockerImageRef::try_new(&nitro_cli)?,
        },
        well_known: WellKnown::default(),
        health_check: HealthCheck::default(),
        scaling: Scaling::default(),
        tls_termination: TlsTermination::default(),
        egress: Egress {
            enabled: true,
            destinations: vec![EgressPattern::try_new(r"ipify\.org$")?],
        },
    };

    let writes: Vec<(&str, String)> = vec![
        ("src/main.rs", sample_main_rs().to_string()),
        ("Cargo.toml", init_cargo_toml()),
        ("Dockerfile", init_dockerfile().to_string()),
        (
            "tests/integration.test.mjs",
            sample_integration_test_mjs().to_string(),
        ),
        ("package.json", sample_package_json().to_string()),
        (".gitignore", "/target\n/Cargo.lock\n".into()),
    ];

    for (relative_path, contents) in writes {
        spinner.set_message(format!("Writing {relative_path}…"));
        let dest = directory.join(relative_path);
        if let Some(parent) = dest.parent() {
            fs::create_dir_all(parent).with_context(|| format!("create {}", parent.display()))?;
        }
        fs::write(dest, contents).with_context(|| format!("write {relative_path}"))?;
    }

    spinner.set_message("Writing nitrum.toml…");
    write_sample_config(&directory.join("nitrum.toml"), &nitrum_config)
        .context("write nitrum.toml")?;
    spinner.finish_with_message("Project initialized.");

    Ok(())
}

fn write_sample_config(path: &Path, config: &NitrumConfig) -> Result<()> {
    let body = toml::to_string_pretty(config).context("serialize nitrum.toml")?;
    let contents = format!(
        "# Default template generated with `nitrum init`\n\
         # For details check https://github.com/matzapata/nitrum/blob/develop/crates/config/src/sections/\n\
         \n\
         {body}"
    );
    fs::write(path, contents).with_context(|| format!("write {}", path.display()))
}

const fn sample_main_rs() -> &'static str {
    include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../",
        "samples/hello/src/main.rs"
    ))
}

/// Standalone Cargo.toml for `nitrum init` (git dep on `sdk`, not a monorepo path).
fn init_cargo_toml() -> String {
    r#"[package]
name = "hello"
version = "0.1.0"
edition = "2024"
rust-version = "1.95"
license = "MIT"
publish = false

[[bin]]
name = "hello"
path = "src/main.rs"

[dependencies]
axum = "0.8"
base64 = "0.22"
serde = { version = "1", features = ["derive"] }
serde_json = "1"
sdk = { git = "https://github.com/matzapata/nitrum.git", package = "sdk", branch = "develop" }
tokio = { version = "1", features = ["full"] }
tracing = "0.1"
tracing-subscriber = { version = "0.3", features = ["env-filter"] }
reqwest = { version = "0.12", default-features = false, features = ["rustls-tls", "json"] }
opentelemetry = "0.28"
opentelemetry_sdk = { version = "0.28", features = ["rt-tokio"] }
opentelemetry-otlp = { version = "0.28", features = ["grpc-tonic", "metrics"] }
"#
    .to_string()
}

/// Standalone Dockerfile for `nitrum init` (project-dir build context).
const fn init_dockerfile() -> &'static str {
    r#"ARG DATA_PLANE_IMAGE=ghcr.io/matzapata/nitrum/data-plane:latest-dev
ARG RUST_IMAGE=rust:1.95-bookworm

FROM --platform=linux/amd64 ${RUST_IMAGE} AS builder
WORKDIR /build
COPY Cargo.toml ./
COPY src ./src
RUN cargo build --release

FROM --platform=linux/amd64 ${DATA_PLANE_IMAGE}
WORKDIR /app
COPY --from=builder /build/target/release/hello /app/hello
COPY nitrum.toml /app/nitrum.toml
CMD ["/app/data-plane", "--config", "/app/nitrum.toml"]
"#
}

const fn sample_integration_test_mjs() -> &'static str {
    include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../",
        "samples/hello/tests/integration.test.mjs"
    ))
}

const fn sample_package_json() -> &'static str {
    include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../",
        "samples/hello/package.json"
    ))
}
