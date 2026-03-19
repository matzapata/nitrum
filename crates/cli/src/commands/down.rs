use clap::Args;
use indicatif::ProgressBar;
use std::env;

use crate::utils::{compose, console};

#[derive(Args)]
pub struct DownArgs {
    /// Project directory (default: current directory)
    #[arg(short, long)]
    pub root: Option<std::path::PathBuf>,
}

pub async fn run(args: DownArgs) {
    let root = args
        .root
        .unwrap_or_else(|| env::current_dir().expect("current directory"));

    let spinner = console::style_spinner(ProgressBar::new_spinner(), "Stopping local stack…");
    match compose::docker_compose(&root, &["down"]).await {
        Ok(()) => spinner.finish_with_message("Local stack stopped."),
        Err(e) => {
            spinner.finish_and_clear();
            eprintln!("{e:#}");
            std::process::exit(1);
        }
    }
}
