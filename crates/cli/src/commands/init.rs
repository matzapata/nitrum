use crate::utils;
use anyhow::{Context, Result, bail};
use clap::Args;
use indicatif::ProgressBar;
use serde_json::Value;
use shared::config::{
    HealthCheck, NitrumConfig, Project, Runtime, Scaling, Service, TlsTermination,
    validate_project_name,
};
use std::env;
use std::fs;

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
        "Writing bundled sample project…",
    );

    let writes: Vec<(&str, String)> = vec![
        ("src/main.js", sample_main_js().to_string()),
        ("package.json", sample_package_json(&args.name)?),
        ("Dockerfile", sample_dockerfile().to_string()),
        ("nitrum.toml", sample_nitro_config(&args.name)),
    ];

    for (relative_path, contents) in writes {
        spinner.set_message(format!("Writing {relative_path}…"));
        let dest = directory.join(relative_path);
        if let Some(parent) = dest.parent() {
            fs::create_dir_all(parent).with_context(|| format!("create {}", parent.display()))?;
        }
        fs::write(dest, contents).with_context(|| format!("write {}", relative_path))?;
    }
    spinner.finish_with_message("Project initialized.");

    Ok(())
}

fn sample_nitro_config(name: &str) -> String {
    let config = NitrumConfig {
        project: Project {
            name: name.to_string(),
        },
        runtime: Runtime {
            data_plane: "matzapata/nitrum-data-plane:latest".to_string(),
            control_plane: "matzapata/nitrum-control-plane:latest".to_string(),
            nitro_cli: "matzapata/nitrum-nitro-cli:latest".to_string(),
        },
        service: Service::default(),
        health_check: HealthCheck::default(),
        scaling: Scaling::default(),
        tls_termination: TlsTermination::default(),
    };
    config
        .validate()
        .expect("init template must satisfy NitrumConfig::validate");
    let body = toml::to_string_pretty(&config).expect("serialize nitrum.toml for init");
    format!(
        "# Default template generated with `nitrum init`\n\
         # For details check https://github.com/matzapata/nitrum/blob/develop/crates/shared/src/config.rs\n\
         \n\
         {body}"
    )
}

fn sample_main_js() -> &'static str {
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

fn sample_package_json_template() -> &'static str {
    include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../",
        "samples/hello/package.json"
    ))
}

fn sample_dockerfile() -> &'static str {
    include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../",
        "samples/hello/Dockerfile"
    ))
}
