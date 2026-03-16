use clap::Args;

#[derive(Args)]
pub struct DevArgs {
    /// Path to project directory (default: current directory)
    #[arg(short, long)]
    pub path: Option<std::path::PathBuf>,
}

pub fn run(_args: DevArgs) {
    // build with data-plane dev image
    
    // if .nitrum/docker-compose.yml doesn't exist create it
    
    // run docker compose up
}
