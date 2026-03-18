use clap::Args;
use std::fs;
use std::path::Path;

use crate::constants::{NITRUM_GITHUB_REPO_NAME, NITRUM_GITHUB_REPO_OWNER};
use crate::infrastructure::github;

#[derive(Args)]
pub struct DevArgs {
    /// Path to project directory (default: current directory)
    #[arg(short, long)]
    pub path: Option<std::path::PathBuf>,
}

pub fn run(_args: DevArgs) {
    // build with data-plane dev image

    // check if .nitrum folder exists, create it otherwise
    let nitrum_dir = Path::new(".nitrum");
    if !nitrum_dir.exists() {
        fs::create_dir_all(nitrum_dir).unwrap();
    }

    // if .nitrum/docker-compose.yml doesn't exist create it
    let docker_compose_file = nitrum_dir.join("docker-compose.yml");
    if !docker_compose_file.exists() {
        github::download_file(NITRUM_GITHUB_REPO_OWNER, NITRUM_GITHUB_REPO_NAME, "develop", "samples/hello/docker-compose.yml", &docker_compose_file).unwrap();
    }

    // run docker compose up
}
