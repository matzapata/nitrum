use crate::utils;
use crate::utils::image_digest::ImageDigestResolver;
use anyhow::{Context, Result, bail};
use clap::Args;
use config::{
    Cloud, DockerImageRef, Egress, EgressPattern, HealthCheck, Local, NitrumConfig, Project,
    ProjectName, Runtime, Scaling, TlsTermination,
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
            dockerfile: None,
        },
        runtime: Runtime {
            data_plane: DockerImageRef::try_new(&data_plane)?,
            control_plane: DockerImageRef::try_new(&control_plane)?,
            nitro_cli: DockerImageRef::try_new(&nitro_cli)?,
        },
        health_check: HealthCheck::default(),
        scaling: Scaling::default(),
        tls_termination: TlsTermination::default(),
        egress: Egress {
            enabled: true,
            destinations: vec![EgressPattern::try_new(r"ipify\.org$")?],
        },
        cloud: Cloud::default(),
        local: Local::default(),
    };

    let writes: Vec<(&str, &str)> = vec![
        ("src/main.rs", template_main_rs()),
        ("Cargo.toml", template_cargo_toml()),
        ("Dockerfile", template_dockerfile()),
        (
            "tests/integration.test.mjs",
            template_integration_test_mjs(),
        ),
        ("package.json", template_package_json()),
        (".gitignore", "/target\n/Cargo.lock\n"),
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

/// Files under `template/` — customer scaffold + stack YAML for the CLI
/// (not the monorepo `examples/hello` git-`nitrum-sdk` demos).
macro_rules! bundled_template {
    ($rel:literal) => {
        include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/template/", $rel))
    };
}

const fn template_main_rs() -> &'static str {
    bundled_template!("src/main.rs")
}

const fn template_cargo_toml() -> &'static str {
    bundled_template!("Cargo.toml")
}

const fn template_dockerfile() -> &'static str {
    bundled_template!("Dockerfile")
}

const fn template_integration_test_mjs() -> &'static str {
    bundled_template!("tests/integration.test.mjs")
}

const fn template_package_json() -> &'static str {
    bundled_template!("package.json")
}
