//! Manage application environment variables in SSM Parameter Store (`nitrum cloud env set|get|delete`).

use anyhow::{Result, bail};
use clap::Args;
use clap::Subcommand;
use config::PlatformLayout;
use std::path::PathBuf;

use crate::cloud::Ssm;
use crate::project::CliProject;
use crate::utils;

#[derive(Args)]
pub struct EnvArgs {
    /// Use this project name instead of `project.name` in nitrum.toml (CloudFormation/S3/SSM/Docker tag)
    #[arg(long = "as", value_name = "NAME")]
    pub as_name: Option<String>,
    /// Project directory (default: current directory)
    #[arg(short, long)]
    pub path: Option<PathBuf>,

    #[command(subcommand)]
    pub command: EnvCommand,
}

#[derive(Subcommand)]
pub enum EnvCommand {
    /// Store a value as a `SecureString` parameter (overwrites if present)
    Set {
        /// Environment variable name (e.g. `DATABASE_URL`)
        key: String,
        /// Value to store
        value: String,
    },
    /// List all app env parameters under the project prefix (decrypted KEY=value; sensitive)
    Get,
    /// Remove the parameter
    Delete {
        key: String,
        /// Skip the confirmation prompt
        #[arg(long)]
        force: bool,
    },
}

pub async fn run(args: EnvArgs) -> Result<()> {
    let project = CliProject::load(args.path, args.as_name)?;
    let ssm = Ssm::new().await?;
    let layout = PlatformLayout::from_project(&project.config.project);

    match args.command {
        EnvCommand::Set { key, value } => {
            validate_env_key(&key)?;
            let name = layout.app_env_key(&key);
            ssm.set(&name, value).await?;
            println!("Set {key} in SSM ({name})");
        }
        EnvCommand::Get => {
            let path = layout.app_env_prefix();
            let rows = ssm.list(&path).await?;
            if rows.is_empty() {
                println!("(no parameters under {path}/)");
            } else {
                for (key, value) in rows {
                    println!("{key}={value}");
                }
            }
        }
        EnvCommand::Delete { key, force } => {
            validate_env_key(&key)?;
            let name = layout.app_env_key(&key);
            if !force && !utils::confirm(&format!("Delete `{key}` from SSM ({name})?")) {
                return Ok(());
            }

            ssm.delete(&name).await?;
            println!("Deleted {key} ({name})");
        }
    }

    Ok(())
}

fn validate_env_key(key: &str) -> Result<()> {
    let mut chars = key.chars();
    let Some(first) = chars.next() else {
        bail!("env key must not be empty");
    };
    if !matches!(first, 'a'..='z' | 'A'..='Z' | '_') {
        bail!("env key must start with a letter or underscore");
    }
    for c in chars {
        if !matches!(c, 'a'..='z' | 'A'..='Z' | '0'..='9' | '_') {
            bail!("env key may contain only letters, digits, and underscores");
        }
    }
    Ok(())
}
