use crate::bundled;
use crate::utils::console;
use clap::Args;
use indicatif::ProgressBar;
use serde_json::Value;
use shared::config::{Config, Egress, HealthCheck, Scaling, Service, TlsTermination};
use std::env;
use std::fs;
use std::path::PathBuf;

#[derive(Args)]
pub struct InitArgs {
    /// Project name; creates a subdirectory with this name in the current directory
    #[arg(value_name = "NAME")]
    pub name: Option<String>,
}

pub async fn run(args: InitArgs) {
    let cwd = env::current_dir().expect("current directory");
    let directory = match &args.name {
        Some(name) => cwd.join(name),
        None => cwd,
    };

    if directory.exists() && directory.read_dir().unwrap().next().is_some() {
        eprintln!("Directory is not empty");
        std::process::exit(1);
    }

    fs::create_dir_all(directory.join("src")).unwrap();

    let spinner = console::style_spinner(
        ProgressBar::new_spinner(),
        "Writing bundled sample project…",
    );

    let nitrum_name = args.name.as_deref();
    let writes: &[(&str, &str, PathBuf)] = &[
        (
            "main.js",
            bundled::sample_main_js(),
            directory.join("src/main.js"),
        ),
        (
            "package.json",
            bundled::sample_package_json(),
            directory.join("package.json"),
        ),
        (
            "Dockerfile",
            bundled::sample_dockerfile(),
            directory.join("Dockerfile"),
        )
    ];

    for (label, contents, dest) in writes {
        spinner.set_message(format!("Writing {label}…"));
        if let Some(parent) = dest.parent() {
            fs::create_dir_all(parent).unwrap_or_else(|e| panic!("create {}: {e}", parent.display()));
        }
        fs::write(dest, contents).unwrap_or_else(|e| panic!("write {label}: {e}"));
    }

    spinner.set_message("Writing nitrum.toml…");
    let nitrum_path = directory.join("nitrum.toml");
    fs::write(
        &nitrum_path,
        nitrum_toml_for_init(nitrum_name.unwrap_or("nitrum-hello")),
    )
    .unwrap_or_else(|e| panic!("write nitrum.toml: {e}"));

    if let Some(name) = &args.name {
        let package_json_path = directory.join("package.json");
        let raw = fs::read_to_string(&package_json_path).expect("read package.json");
        let mut value: Value = serde_json::from_str(&raw).expect("parse package.json");
        if let Value::Object(map) = &mut value {
            map.insert("name".to_string(), Value::String(name.clone()));
        }
        fs::write(
            &package_json_path,
            serde_json::to_string_pretty(&value).expect("serialize package.json"),
        )
        .expect("write package.json");
    }

    spinner.finish_with_message("Project initialized.");
}

/// [`Config`] defaults plus hello-sample egress (`enabled = true`, `httpbin\.org$`).
fn nitrum_toml_for_init(name: &str) -> String {
    let config = Config {
        name: name.to_string(),
        service: Service::default(),
        health_check: HealthCheck::default(),
        scaling: Scaling::default(),
        tls_termination: TlsTermination::default(),
        egress: Egress {
            enabled: true,
            destinations: vec![r"httpbin\.org$".to_string()],
        },
    };
    toml::to_string_pretty(&config).expect("serialize nitrum.toml for init")
}
