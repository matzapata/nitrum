use std::process::Stdio;
use serde_json::Value;
use thiserror::Error;
use tokio::{
    io::{AsyncBufReadExt, BufReader},
    process::Command,
};
use tracing::info;

const NITRO_CLI: &str = "nitro-cli";
const EIF_PATH: &str = "/app/enclave.eif";
const ENCLAVE_CID: &str = "16";
const NUM_CPUS: &str = "2";
const RAM_SIZE_MIB: &str = "4320";

#[derive(Error, Debug)]
pub enum EnclaveError {
    #[error("Failed to run command: {0}")]
    CommandFailed(String),
    #[error("Failed to send debug logs to stdout: {0}")]
    SendDebugLogsFailed(String),
}

// TODO: cleanup
impl From<std::io::Error> for EnclaveError {
    fn from(e: std::io::Error) -> Self {
        EnclaveError::CommandFailed(e.to_string())
    }
}

impl From<serde_json::Error> for EnclaveError {
    fn from(e: serde_json::Error) -> Self {
        EnclaveError::CommandFailed(e.to_string())
    }
}

enum NitroCommand {
    TerminateEnclave,
    DescribeEnclaves,
    RunEnclave,
    Console,
}

impl NitroCommand {
    pub fn as_str(&self) -> &str {
        match self {
            NitroCommand::TerminateEnclave => "terminate-enclave",
            NitroCommand::DescribeEnclaves => "describe-enclaves",
            NitroCommand::RunEnclave => "run-enclave",
            NitroCommand::Console => "console",
        }
    }
}

pub struct Enclave;

impl Enclave {
    pub async fn run(debug_mode: bool) -> Result<(), EnclaveError> {
        let running_enclaves =
            Self::run_command_capture_stdout(&[NITRO_CLI, NitroCommand::DescribeEnclaves.as_str()])
                .await?;
        let enclaves: Value = serde_json::from_str(&running_enclaves)?;
        let empty: Vec<Value> = vec![];
        let enclaves_array = enclaves.as_array().unwrap_or(&empty);
        if !enclaves_array.is_empty() {
            info!("There's an enclave already running on this host. Terminating it...");
            Self::shutdown_all_enclaves().await?;
            info!("Enclave terminated. Waiting 10s...");
            tokio::time::sleep(std::time::Duration::from_secs(10)).await;
        } else {
            info!("No enclaves currently running on this host.");
        }

        info!("Starting new enclave...");
        let mut run_args = vec![
            NITRO_CLI,
            NitroCommand::RunEnclave.as_str(),
            "--cpu-count",
            NUM_CPUS,
            "--memory",
            RAM_SIZE_MIB,
            "--enclave-cid",
            ENCLAVE_CID,
            "--eif-path",
            EIF_PATH,
        ];
        if debug_mode {
            info!("Debug mode enabled...");
            run_args.push("--debug-mode");
        } else {
            info!("Debug mode disabled...");
        }

        Self::run_command_capture_stdout(&run_args).await?;

        info!("Enclave started... Waiting 5 seconds for warmup.");
        tokio::time::sleep(std::time::Duration::from_secs(5)).await;

        if debug_mode {
            Self::send_debug_logs_to_stdout().await?;
        }

        Ok(())
    }

    pub async fn shutdown_all_enclaves() -> Result<String, EnclaveError> {
        Self::run_command_capture_stdout(&[
            NITRO_CLI,
            NitroCommand::TerminateEnclave.as_str(),
            "--all",
        ])
        .await
    }

    async fn send_debug_logs_to_stdout() -> Result<(), EnclaveError> {
        info!("Attaching headless console for running enclaves...");
        let running_enclaves =
            Self::run_command_capture_stdout(&[NITRO_CLI, NitroCommand::DescribeEnclaves.as_str()])
                .await?;
        let enclaves: Value = serde_json::from_str(&running_enclaves)?;
        let empty: Vec<Value> = vec![];
        let enclaves_array = enclaves.as_array().unwrap_or(&empty).clone();
        for enclave in enclaves_array {
            if let Some(id) = enclave["EnclaveID"].as_str() {
                let mut child = Command::new(NITRO_CLI)
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

    async fn run_command_capture_stdout(args: &[&str]) -> Result<String, EnclaveError> {
        let output = Command::new(args[0])
            .args(&args[1..])
            .stderr(Stdio::inherit())
            .output()
            .await?;

        if !output.status.success() {
            return Err(EnclaveError::CommandFailed(format!(
                "Command {:?} failed with exit status: {}",
                args, output.status
            )));
        }

        Ok(String::from_utf8_lossy(&output.stdout).to_string())
    }
}
