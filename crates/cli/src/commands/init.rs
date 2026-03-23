use crate::constants::{NITRUM_GITHUB_REPO_NAME, NITRUM_GITHUB_REPO_OWNER};
use crate::utils::{console, github};
use clap::Args;
use indicatif::ProgressBar;
use serde_json::Value;
use std::env;
use std::fs;
use std::path::PathBuf;

const SAMPLE_REF: &str = "develop";

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
        "Fetching sample files from GitHub…",
    );

    let files: &[(&str, PathBuf)] = &[
        ("samples/hello/src/main.js", directory.join("src/main.js")),
        ("samples/hello/package.json", directory.join("package.json")),
        ("samples/hello/nitrum.toml", directory.join("nitrum.toml")),
        ("samples/hello/Dockerfile", directory.join("Dockerfile")),
    ];

    for (repo_path, dest) in files {
        spinner.set_message(format!(
            "Downloading {}…",
            dest.file_name().unwrap().to_string_lossy()
        ));
        if let Err(e) = github::download_file(
            NITRUM_GITHUB_REPO_OWNER,
            NITRUM_GITHUB_REPO_NAME,
            SAMPLE_REF,
            repo_path,
            dest.as_path(),
        )
        .await
        {
            eprintln!("{e:#}");
            std::process::exit(1);
        }
    }

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
