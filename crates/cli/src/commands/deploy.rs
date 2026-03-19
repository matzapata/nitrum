use clap::Args;

#[derive(Args)]
pub struct DeployArgs {
    /// Path to project directory (default: current directory)
    #[arg(short, long)]
    pub path: Option<std::path::PathBuf>,
}

pub async fn run(_args: DeployArgs) {
    // TODO
}
