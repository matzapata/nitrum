use clap::Parser;
use tracing_subscriber::EnvFilter;

pub mod commands;
pub mod constants;
pub mod utils;

use commands::{build, deploy, describe, destroy, down, init, logs, up};

#[derive(Parser)]
#[command(name = "nitrum")]
#[command(about = "Nitrum CLI", long_about = None)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(clap::Subcommand)]
enum Commands {
    /// Initialize a new Nitrum project
    Init(init::InitArgs),
    /// Build an enclave image
    Build(build::BuildArgs),
    /// Start local stack (docker compose up -d)
    Up(up::UpArgs),
    /// Stop local stack (docker compose down)
    Down(down::DownArgs),
    /// Tail service logs (docker compose logs)
    Logs(logs::LogsArgs),
    /// Deploy to production
    Deploy(deploy::DeployArgs),
    /// Describe enclave or project state
    Describe(describe::DescribeArgs),
    /// Destroy resources
    Destroy(destroy::DestroyArgs),
}

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env())
        .init();

    let cli = Cli::parse();
    match cli.command {
        Commands::Init(args) => init::run(args).await,
        Commands::Build(args) => build::run(args).await,
        Commands::Up(args) => up::run(args).await,
        Commands::Down(args) => down::run(args).await,
        Commands::Logs(args) => logs::run(args).await,
        Commands::Deploy(args) => deploy::run(args).await,
        Commands::Describe(args) => describe::run(args).await,
        Commands::Destroy(args) => destroy::run(args).await,
    }
}
