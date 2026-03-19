//! Build enclave Docker image (prod data-plane base) and produce `enclave.eif` via nitro-cli in Docker.

use clap::Args;
use indicatif::ProgressBar;
use std::env;

use crate::constants;
use crate::utils::{console, docker};

#[derive(Args)]
pub struct BuildArgs {
    /// Path to project directory (default: current directory)
    #[arg(short, long)]
    pub path: Option<std::path::PathBuf>,
}

pub async fn run(args: BuildArgs) {
    let root = args
        .path
        .unwrap_or_else(|| env::current_dir().expect("current directory"));

    let spinner = console::style_spinner(
        ProgressBar::new_spinner(),
        "Building enclave image (prod data-plane base)…",
    );
    match docker::build_enclave_image(
        &root,
        constants::ENCLAVE_PROD_BASE_IMAGE,
        constants::ENCLAVE_PROD_LOCAL_IMAGE,
    )
    .await
    {
        Ok(()) => spinner.finish_with_message(format!(
            "Docker image built as `{}`.",
            constants::ENCLAVE_PROD_LOCAL_IMAGE
        )),
        Err(e) => {
            spinner.finish_and_clear();
            eprintln!("{e:#}");
            std::process::exit(1);
        }
    }

    let spinner = console::style_spinner(
        ProgressBar::new_spinner(),
        "Building enclave.eif (nitro-cli in Docker)…",
    );
    match docker::build_enclave_eif(&root, constants::ENCLAVE_PROD_LOCAL_IMAGE).await {
        Ok(()) => spinner.finish_with_message("enclave.eif written."),
        Err(e) => {
            spinner.finish_and_clear();
            eprintln!("{e:#}");
            std::process::exit(1);
        }
    }

    let eif = root.join("enclave.eif");
    println!();
    println!("{}", eif.display());
}
