//! Manage application environment variables in SSM Parameter Store (`nitrum cloud env set|get|delete`).

use anyhow::{Result, bail};
use clap::Args;
use clap::Subcommand;
use shared::config::NitrumConfig;
use std::env;
use std::path::PathBuf;

use crate::utils::{self, Ssm};

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
    /// Store a value as a SecureString parameter (overwrites if present)
    Set {
        /// Environment variable name (e.g. DATABASE_URL)
        key: String,
        /// Value to store
        value: String,
    },
    /// List all app env parameters under the project prefix (decrypted KEY=value; sensitive)
    Get,
    /// Remove the parameter
    Delete { key: String },
}

pub async fn run(args: EnvArgs) -> Result<()> {
    let root = args
        .path
        .unwrap_or_else(|| env::current_dir().expect("current directory"));
    let config = NitrumConfig::try_from(root.join("nitrum.toml").as_path())?
        .with_name(args.as_name.clone())?;

    let ssm = Ssm::new().await?;

    match args.command {
        EnvCommand::Set { key, value } => {
            validate_env_key(&key)?;
            let name = app_env_parameter_name(&config.project.name, &key);
            ssm.set(&name, value).await?;
            println!("Set {key} in SSM ({name})");
        }
        EnvCommand::Get => {
            let path = app_env_ssm_path_prefix(&config.project.name);
            let rows = ssm.list(&path).await?;
            if rows.is_empty() {
                println!("(no parameters under {path}/)");
            } else {
                for (key, value) in rows {
                    println!("{key}={value}");
                }
            }
        }
        EnvCommand::Delete { key } => {
            validate_env_key(&key)?;
            let name = app_env_parameter_name(&config.project.name, &key);
            if !utils::confirm(&format!("Delete `{key}` from SSM ({name})?")) {
                return Ok(());
            }
            ssm.delete(&name).await?;
            println!("Deleted {key} ({name})");
        }
    }

    Ok(())
}

/// SSM path prefix for app env parameters (`GetParametersByPath`).
#[must_use]
pub fn app_env_ssm_path_prefix(project_name: &str) -> String {
    format!("/nitrum/{project_name}/env")
}

/// SSM name for one app env key under `/nitrum/{project name}/env/`.
#[must_use]
pub fn app_env_parameter_name(project_name: &str, key: &str) -> String {
    format!("/nitrum/{project_name}/env/{key}")
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

#[cfg(test)]
mod tests {
    use super::app_env_parameter_name;

    #[test]
    fn parameter_path_matches_data_plane_prefix() {
        assert_eq!(
            app_env_parameter_name("myapp", "API_KEY"),
            "/nitrum/myapp/env/API_KEY"
        );
    }
}
