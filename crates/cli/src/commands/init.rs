use clap::Args;

#[derive(Args)]
pub struct InitArgs {
    /// Project directory (default: current directory)
    #[arg(short, long)]
    pub directory: Option<std::path::PathBuf>,
}

pub fn run(_args: InitArgs) {
    // TODO: create sample from github, just copy main.js, package.json, nitrum.toml and dockerfile
}
