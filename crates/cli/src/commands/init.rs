use crate::utils;
use crate::utils::image_digest::ImageDigestResolver;
use anyhow::{Context, Result, bail};
use clap::Args;
use config::{
    HealthCheck, NitrumConfig, Project, Runtime, Scaling, Service, TlsTermination,
    validate_project_name,
};
use futures_util::future::try_join3;
use indicatif::ProgressBar;
use serde_json::Value;
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
    validate_project_name(&args.name).map_err(|msg| {
        anyhow::anyhow!(
            "{msg} (project directory name is used as `project.name` in nitrum.toml for `nitrum cloud deploy`)"
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
            name: args.name.clone(),
        },
        runtime: Runtime {
            data_plane,
            control_plane,
            nitro_cli,
        },
        service: Service::default(),
        health_check: HealthCheck::default(),
        scaling: Scaling::default(),
        tls_termination: TlsTermination::default(),
    };

    let writes: Vec<(&str, String)> = vec![
        ("src/main.js", sample_main_js().to_string()),
        ("package.json", sample_package_json(&args.name)?),
        ("Dockerfile", sample_dockerfile().to_string()),
        (
            "tests/integration.test.mjs",
            sample_integration_test_mjs().to_string(),
        ),
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
    config
        .validate()
        .map_err(|e| anyhow::anyhow!("invalid init template: {e}"))?;
    let body = toml::to_string_pretty(config).context("serialize nitrum.toml")?;
    let contents = format!(
        "# Default template generated with `nitrum init`\n\
         # For details check https://github.com/matzapata/nitrum/blob/develop/crates/config/src/lib.rs\n\
         \n\
         {body}"
    );
    fs::write(path, contents).with_context(|| format!("write {}", path.display()))
}

const fn sample_main_js() -> &'static str {
    include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../",
        "samples/hello/src/main.js"
    ))
}

fn sample_package_json(name: &str) -> Result<String> {
    let mut value: Value = serde_json::from_str(sample_package_json_template())
        .context("parse sample package.json")?;
    if let Value::Object(map) = &mut value {
        map.insert("name".to_string(), Value::String(name.to_string()));
    }
    serde_json::to_string_pretty(&value).context("serialize package.json")
}

const fn sample_package_json_template() -> &'static str {
    include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../",
        "samples/hello/package.json"
    ))
}

const fn sample_dockerfile() -> &'static str {
    include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../",
        "samples/hello/Dockerfile"
    ))
}

const fn sample_integration_test_mjs() -> &'static str {
    include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../",
        "samples/hello/tests/integration.test.mjs"
    ))
}
