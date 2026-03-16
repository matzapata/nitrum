use clap::Parser;

pub mod commands;
pub mod infrastructure;
pub mod constants;

use commands::{build, deploy, describe, destroy, dev, init};

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
    /// Run local development environment
    Dev(dev::DevArgs),
    /// Deploy to production
    Deploy(deploy::DeployArgs),
    /// Describe enclave or project state
    Describe(describe::DescribeArgs),
    /// Destroy resources
    Destroy(destroy::DestroyArgs),
}

fn main() {
    let cli = Cli::parse();
    match cli.command {
        Commands::Init(args) => init::run(args),
        Commands::Build(args) => build::run(args),
        Commands::Dev(args) => dev::run(args),
        Commands::Deploy(args) => deploy::run(args),
        Commands::Describe(args) => describe::run(args),
        Commands::Destroy(args) => destroy::run(args),
    }
}
