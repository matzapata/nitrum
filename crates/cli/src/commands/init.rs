use crate::constants::{NITRUM_GITHUB_REPO_NAME, NITRUM_GITHUB_REPO_OWNER};
use crate::utils::{console, github};
use clap::Args;
use indicatif::ProgressBar;
use std::env;
use std::fs;
use std::path::PathBuf;

const SAMPLE_REF: &str = "develop";

#[derive(Args)]
pub struct InitArgs {
    /// Project directory (default: current directory)
    #[arg(short, long)]
    pub directory: Option<PathBuf>,
}

pub async fn run(args: InitArgs) {
    let directory = args
        .directory
        .unwrap_or_else(|| env::current_dir().expect("current directory"));

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

    spinner.finish_with_message("Project initialized.");
}
