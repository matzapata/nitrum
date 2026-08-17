mod down;
mod eject;
mod logs;
mod up;

use anyhow::Result;
use clap::{Args, Subcommand};

#[derive(Args)]
pub struct LocalArgs {
    #[command(subcommand)]
    pub command: LocalCommand,
}

#[derive(Subcommand)]
pub enum LocalCommand {
    /// Start local stack (docker compose up -d)
    Up(up::UpArgs),
    /// Stop local stack (docker compose down)
    Down(down::DownArgs),
    /// Tail service logs (docker compose logs)
    Logs(logs::LogsArgs),
    /// Write the bundled Compose template into the project
    Eject(eject::EjectArgs),
}

pub async fn run(args: LocalArgs) -> Result<()> {
    match args.command {
        LocalCommand::Up(a) => up::run(a).await,
        LocalCommand::Down(a) => down::run(a).await,
        LocalCommand::Logs(a) => logs::run(a).await,
        LocalCommand::Eject(a) => eject::run(a),
    }
}
