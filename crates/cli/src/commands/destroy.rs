use clap::Args;

#[derive(Args)]
pub struct DestroyArgs {
    /// Path to project directory (default: current directory)
    #[arg(short, long)]
    pub path: Option<std::path::PathBuf>,
}

pub fn run(_args: DestroyArgs) {
    // TODO
}
