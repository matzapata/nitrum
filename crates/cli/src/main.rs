use clap::Parser;
use tracing_subscriber::EnvFilter;

pub mod artifact;
pub mod cloud;
pub mod commands;
pub mod constants;
pub mod local;
pub mod utils;

use commands::{build, cloud as cloud_cmd, describe, init, local as local_cmd};

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
    /// Local development (docker compose)
    Local(local_cmd::LocalArgs),
    /// AWS: deploy, destroy, env, logs
    Cloud(cloud_cmd::CloudArgs),
    /// Describe an EIF (`nitro-cli describe-eif` in Docker)
    Describe(describe::DescribeArgs),
}

#[tokio::main]
async fn main() {
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| {
        // Nitrum CLI progress at info; keep AWS/Hyper noise down unless you raise them via RUST_LOG.
        EnvFilter::new("warn,cli=info")
    });
    tracing_subscriber::fmt().with_env_filter(filter).init();

    let cli = Cli::parse();
    let result = match cli.command {
        Commands::Init(args) => init::run(args),
        Commands::Build(args) => build::run(args).await,
        Commands::Local(args) => local_cmd::run(args).await,
        Commands::Cloud(args) => cloud_cmd::run(args).await,
        Commands::Describe(args) => describe::run(args).await,
    };

    if let Err(e) = result {
        eprintln!("{e:#}");
        std::process::exit(1);
    }
}
