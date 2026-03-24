mod down;
mod logs;
mod up;

use clap::{Args, Subcommand};

#[derive(Args)]
pub struct DevArgs {
    #[command(subcommand)]
    pub command: DevCommand,
}

#[derive(Subcommand)]
pub enum DevCommand {
    /// Start local stack (docker compose up -d)
    Up(up::UpArgs),
    /// Stop local stack (docker compose down)
    Down(down::DownArgs),
    /// Tail service logs (docker compose logs)
    Logs(logs::LogsArgs),
}

pub async fn run(args: DevArgs) {
    match args.command {
        DevCommand::Up(a) => up::run(a).await,
        DevCommand::Down(a) => down::run(a).await,
        DevCommand::Logs(a) => logs::run(a).await,
    }
}
