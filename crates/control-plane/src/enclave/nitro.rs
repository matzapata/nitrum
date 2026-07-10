//! `nitro-cli` subprocess helpers.

use crate::constants::NITRO_CLI;
use std::process::Stdio;
use thiserror::Error;
use tokio::process::Command as TokioCommand;

#[derive(Error, Debug)]
pub enum NitroError {
    #[error("Failed to run command: {0}")]
    CommandFailed(String),
}

impl From<std::io::Error> for NitroError {
    fn from(e: std::io::Error) -> Self {
        Self::CommandFailed(e.to_string())
    }
}

impl From<serde_json::Error> for NitroError {
    fn from(e: serde_json::Error) -> Self {
        Self::CommandFailed(e.to_string())
    }
}

pub(super) enum NitroCommand {
    TerminateEnclave,
    DescribeEnclaves,
    RunEnclave,
    Console,
}

impl NitroCommand {
    pub const fn as_str(&self) -> &str {
        match self {
            Self::TerminateEnclave => "terminate-enclave",
            Self::DescribeEnclaves => "describe-enclaves",
            Self::RunEnclave => "run-enclave",
            Self::Console => "console",
        }
    }
}

pub(super) async fn run_command_capture_stdout(args: &[&str]) -> Result<String, NitroError> {
    let output = TokioCommand::new(args[0])
        .args(&args[1..])
        .stderr(Stdio::inherit())
        .output()
        .await?;

    if !output.status.success() {
        return Err(NitroError::CommandFailed(format!(
            "Command {:?} failed with exit status: {}",
            args, output.status
        )));
    }

    Ok(String::from_utf8_lossy(&output.stdout).to_string())
}

pub(super) async fn describe_enclaves_stdout() -> Result<String, NitroError> {
    run_command_capture_stdout(&[NITRO_CLI, NitroCommand::DescribeEnclaves.as_str()]).await
}

pub(super) async fn shutdown_all_enclaves() -> Result<String, NitroError> {
    run_command_capture_stdout(&[NITRO_CLI, NitroCommand::TerminateEnclave.as_str(), "--all"]).await
}

pub(super) async fn run_enclave_once(
    debug_mode: bool,
    cpu_count: u32,
    memory_mib: u32,
    eif_path: &str,
) -> Result<(), NitroError> {
    use tracing::info;

    let cpu = cpu_count.to_string();
    let memory = memory_mib.to_string();
    info!(cpu_count = %cpu, memory_mib = %memory, eif_path, "Starting enclave...");
    let mut run_args: Vec<&str> = vec![
        NITRO_CLI,
        NitroCommand::RunEnclave.as_str(),
        "--cpu-count",
        cpu.as_str(),
        "--memory",
        memory.as_str(),
        "--enclave-cid",
        crate::constants::ENCLAVE_CID,
        "--eif-path",
        eif_path,
    ];
    if debug_mode {
        info!("Debug mode enabled...");
        run_args.push("--debug-mode");
    } else {
        info!("Debug mode disabled...");
    }

    run_command_capture_stdout(&run_args).await?;
    Ok(())
}

pub(super) async fn attach_debug_consoles() -> Result<(), NitroError> {
    use serde_json::Value;
    use std::process::Stdio;
    use tokio::io::{AsyncBufReadExt, BufReader};
    use tracing::info;

    info!("Attaching headless console for running enclaves...");
    let running_enclaves =
        run_command_capture_stdout(&[NITRO_CLI, NitroCommand::DescribeEnclaves.as_str()]).await?;
    let enclaves: Value = serde_json::from_str(&running_enclaves)?;
    let empty: Vec<Value> = vec![];
    let enclaves_array = enclaves.as_array().unwrap_or(&empty).clone();
    for enclave in enclaves_array {
        if let Some(id) = enclave["EnclaveID"].as_str() {
            let mut child = TokioCommand::new(NITRO_CLI)
                .args([NitroCommand::Console.as_str(), "--enclave-id", id])
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .spawn()?;

            if let Some(stdout) = child.stdout.take() {
                tokio::spawn(async move {
                    let mut lines = BufReader::new(stdout).lines();
                    while let Ok(Some(line)) = lines.next_line().await {
                        info!("[ENCLAVE]: {line}");
                    }
                });
            }
        }
    }
    Ok(())
}
