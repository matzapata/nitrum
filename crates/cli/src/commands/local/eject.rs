//! Write the bundled Compose template into the project.

use crate::constants::ENCLAVE_LOCAL_EJECT_TEMPLATE_FILE;
use crate::local::eject_local_template;
use crate::project::CliProject;
use anyhow::Result;
use clap::Args;
use std::path::PathBuf;

#[derive(Args)]
pub struct EjectArgs {
    /// Use this project name instead of `project.name` in `nitrum.toml`
    #[arg(long = "as", value_name = "NAME")]
    pub as_name: Option<String>,
    /// Project directory (default: current directory)
    #[arg(short, long)]
    pub path: Option<PathBuf>,
    /// Overwrite the destination file if it already exists
    #[arg(long, action = clap::ArgAction::SetTrue)]
    pub force: bool,
    /// Project-relative output path (default: `infra/local-stack.yml`)
    #[arg(long, value_name = "PATH")]
    pub output: Option<PathBuf>,
}

pub fn run(args: EjectArgs) -> Result<()> {
    let project = CliProject::load(args.path, args.as_name)?;
    let rel = args
        .output
        .unwrap_or_else(|| PathBuf::from(ENCLAVE_LOCAL_EJECT_TEMPLATE_FILE));
    if rel.is_absolute() {
        anyhow::bail!("--output must be a project-relative path");
    }
    let dest = project.root.join(&rel);
    let display_rel = dest
        .strip_prefix(&project.root)
        .unwrap_or(dest.as_path())
        .display()
        .to_string()
        .replace('\\', "/");

    eject_local_template(&dest, args.force)?;

    println!("Wrote {display_rel}.");
    println!(
        "Point local Compose at this file by adding to nitrum.toml:\n\n[local]\ntemplate = \"{display_rel}\"\n"
    );
    Ok(())
}
