use thiserror::Error;
use tokio::{
    io::{AsyncBufReadExt, BufReader},
    process::Command,
};

const EIF_PATH: &str = "enclave.eif";
const NITRO_CLI: &str = "nitro-cli";
const ENCLAVE_CID: &str = "16";
const NUM_CPUS: &str = "2";
const RAM_SIZE_MIB: &str = "4320";
const DEBUG_MODE: &str = "true";

#[derive(Error, Debug)]
pub enum EnclaveError {
    #[error("Failed to run command: {0}")]
    CommandFailed(String),
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
    pub async fn shutdown_all_enclaves() -> Result<String, EnclaveError> {
        let command = vec![
            "sh",
            "-c",
            NITRO_CLI,
            NitroCommand::TerminateEnclave.as_str(),
            "--all",
        ];
        Self::run_command_capture_stdout(&command).await
    }


    pub async fn start(&self) {
        let running_enclaves =
        Self::run_command_capture_stdout(&[NITRO_CLI, NitroCommand::DescribeEnclaves.as_str()])
            .await?;
    let enclaves: Value = serde_json::from_str(&running_enclaves)?;
    let enclaves_array = enclaves.as_array().unwrap_or(&EMPTY_VEC);
    if !enclaves_array.is_empty() {
        info!("There's an enclave already running on this host. Terminating it...");
        Self::shutdown_all_enclaves().await?;
        info!("Enclave terminated. Waiting 10s...");
        tokio::time::sleep(std::time::Duration::from_secs(10)).await;
    } else {
        info!("No enclaves currently running on this host.");
    }

    info!("Starting new enclave...");
    let mut run_command = vec![
        NITRO_CLI,
        NitroCommand::RunEnclave.as_str(),
        "--cpu-count",
        &NUM_CPUS, // TODO: get from config
        "--memory",
        &RAM_SIZE_MIB, // TODO: get from config
        "--enclave-cid",
        ENCLAVE_CID,
        "--eif-path",
        EIF_PATH,
    ];

    // TODO: get from config
    if DEBUG_MODE == "true" {
        info!("Debug mode enabled...");
        run_command.push("--debug-mode");
    } else {
        info!("Debug mode disabled...");
    }

    Self::run_command_capture_stdout(&run_command).await?;

    info!("Enclave started... Waiting 5 seconds for warmup.");
    tokio::time::sleep(std::time::Duration::from_secs(5)).await;

    if run_config.debug_mode == "true" {
        Self::send_debug_logs_to_stdout().await?;
    }
    Ok(())
    }

    async fn send_debug_logs_to_stdout() -> Result<(), OrchestrationError> {
        info!("Attaching headless console for running enclaves...");
        let running_enclaves =
            Self::run_command_capture_stdout(&[NITRO_CLI, NitroCommand::DescribeEnclaves.as_str()])
                .await?;
        let enclaves: Value = serde_json::from_str(&running_enclaves)?;
        let enclaves_array = enclaves.as_array().unwrap_or(&EMPTY_VEC).clone();
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

    async fn run_command_capture_stdout(args: &[&str]) -> Result<String, OrchestrationError> {
        let output = Command::new(args[0])
            .args(&args[1..])
            .stderr(Stdio::inherit())
            .output()
            .await?;

        if !output.status.success() {
            return Err(OrchestrationError::CommandFailed(format!(
                "Command {:?} failed with exit status: {}",
                args, output.status
            )));
        }

        Ok(String::from_utf8_lossy(&output.stdout).to_string())
    }
}
