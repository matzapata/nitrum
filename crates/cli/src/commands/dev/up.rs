use clap::Args;
use indicatif::ProgressBar;
use std::env;

use crate::constants;
use crate::utils::{compose, console, docker, project};

#[derive(Args)]
pub struct UpArgs {
    /// Project directory (default: current directory)
    #[arg(short, long)]
    pub root: Option<std::path::PathBuf>,
}

pub async fn run(args: UpArgs) {
    let root = args
        .root
        .unwrap_or_else(|| env::current_dir().expect("current directory"));

    let cfg = match project::load_project_config(&root) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("{e:#}");
            std::process::exit(1);
        }
    };
    let dev_image = project::enclave_image_dev(&cfg.name);

    let spinner = console::style_spinner(ProgressBar::new_spinner(), "Building enclave…");
    match docker::build_enclave_image(
        &root,
        constants::ENCLAVE_DEV_BASE_IMAGE,
        &dev_image,
    )
    .await
    {
        Ok(()) => spinner.finish_with_message("Enclave built."),
        Err(e) => {
            spinner.finish_and_clear();
            eprintln!("{e:#}");
            std::process::exit(1);
        }
    }

    let spinner = console::style_spinner(ProgressBar::new_spinner(), "Starting local stack…");
    match compose::docker_compose(&root, &dev_image, &["up", "-d"]).await {
        Ok(()) => spinner.finish_with_message("Local stack started."),
        Err(e) => {
            spinner.finish_and_clear();
            eprintln!("{e:#}");
            std::process::exit(1);
        }
    }

    println!();
    println!("Stack is running.");
    println!();
    println!("  https://nitrum.local/     — main ingress (HTTPS on host port 443)");
    println!(
        "  https://127.0.0.1:443/    — same endpoint; use curl -k if the cert name mismatches"
    );
    println!();
    println!("  If nitrum.local does not resolve, add to /etc/hosts: 127.0.0.1 nitrum.local");
    println!();
}
