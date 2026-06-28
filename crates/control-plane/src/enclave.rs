use crate::artifact::EnclaveArtifact;
use crate::constants::{ENCLAVE_CID, ENCLAVE_HEALTH_POLL, MAX_BACKOFF_SECS, NITRO_CLI};
use serde_json::Value;
use std::process::Stdio;
use std::time::Duration;
use thiserror::Error;
use tokio::{
    io::{AsyncBufReadExt, BufReader},
    process::Command as TokioCommand,
    task::JoinHandle,
};
use tracing::{error, info, warn};

#[derive(Error, Debug)]
pub enum EnclaveError {
    #[error("Failed to run command: {0}")]
    CommandFailed(String),
}

impl From<std::io::Error> for EnclaveError {
    fn from(e: std::io::Error) -> Self {
        Self::CommandFailed(e.to_string())
    }
}

impl From<serde_json::Error> for EnclaveError {
    fn from(e: serde_json::Error) -> Self {
        Self::CommandFailed(e.to_string())
    }
}

enum NitroCommand {
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

pub struct Enclave {
    debug_mode: bool,
    cpu_count: u32,
    memory_mib: u32,
    artifact: EnclaveArtifact,
    supervisor: Option<JoinHandle<()>>,
}

impl Enclave {
    #[must_use]
    pub const fn new(
        artifact: EnclaveArtifact,
        debug_mode: bool,
        cpu_count: u32,
        memory_mib: u32,
    ) -> Self {
        Self {
            debug_mode,
            cpu_count,
            memory_mib,
            artifact,
            supervisor: None,
        }
    }

    /// Starts a supervisor task that keeps the Nitro enclave running.
    pub fn run(&mut self) {
        if self.supervisor.is_some() {
            return;
        }
        let debug_mode = self.debug_mode;
        let cpu_count = self.cpu_count;
        let memory_mib = self.memory_mib;
        let artifact = self.artifact.clone();
        self.supervisor = Some(tokio::spawn(async move {
            if let Err(e) = Self::run_loop(debug_mode, cpu_count, memory_mib, artifact).await {
                error!(error = %e, "enclave supervisor exited with error");
            }
        }));
    }

    async fn run_loop(
        debug_mode: bool,
        cpu_count: u32,
        memory_mib: u32,
        artifact: EnclaveArtifact,
    ) -> Result<(), EnclaveError> {
        let mut backoff_secs: u64 = 0;
        let eif_path = artifact.path().display().to_string();

        loop {
            match Self::describe_enclaves_stdout().await {
                Ok(raw) => match serde_json::from_str::<Value>(&raw) {
                    Ok(enclaves) => {
                        let empty: Vec<Value> = vec![];
                        let enclaves_array = enclaves.as_array().unwrap_or(&empty);
                        if !enclaves_array.is_empty() {
                            backoff_secs = 1;
                            tokio::time::sleep(ENCLAVE_HEALTH_POLL).await;
                            continue;
                        }
                    }
                    Err(e) => {
                        warn!(error = %e, "describe-enclaves returned invalid JSON; backing off");
                        Self::sleep_backoff(&mut backoff_secs).await;
                        continue;
                    }
                },
                Err(e) => {
                    warn!(error = %e, "describe-enclaves failed; backing off");
                    Self::sleep_backoff(&mut backoff_secs).await;
                    continue;
                }
            }

            if backoff_secs > 0 {
                info!(
                    seconds = backoff_secs.min(MAX_BACKOFF_SECS),
                    "waiting before enclave (re)start"
                );
                tokio::time::sleep(Duration::from_secs(backoff_secs.min(MAX_BACKOFF_SECS))).await;
            }

            match Self::run_enclave_once(debug_mode, cpu_count, memory_mib, eif_path.as_str()).await
            {
                Ok(()) => {
                    telemetry::metrics::record_enclave_restart("started");
                    info!("Enclave started... Waiting 5 seconds for warmup.");
                    tokio::time::sleep(Duration::from_secs(5)).await;
                    if debug_mode && let Err(e) = Self::send_debug_logs_to_stdout().await {
                        warn!(error = %e, "debug log attach failed; continuing supervision");
                    }
                    backoff_secs = 1;
                }
                Err(e) => {
                    telemetry::metrics::record_enclave_restart("failed");
                    warn!(error = %e, "run-enclave failed");
                    Self::advance_backoff(&mut backoff_secs);
                }
            }
        }
    }

    async fn sleep_backoff(backoff_secs: &mut u64) {
        let wait = (*backoff_secs).clamp(1, MAX_BACKOFF_SECS);
        tokio::time::sleep(Duration::from_secs(wait)).await;
        Self::advance_backoff(backoff_secs);
    }

    fn advance_backoff(backoff_secs: &mut u64) {
        *backoff_secs = if *backoff_secs == 0 {
            1
        } else {
            (*backoff_secs * 2).min(MAX_BACKOFF_SECS)
        };
    }

    async fn describe_enclaves_stdout() -> Result<String, EnclaveError> {
        Self::run_command_capture_stdout(&[NITRO_CLI, NitroCommand::DescribeEnclaves.as_str()])
            .await
    }

    async fn run_enclave_once(
        debug_mode: bool,
        cpu_count: u32,
        memory_mib: u32,
        eif_path: &str,
    ) -> Result<(), EnclaveError> {
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
            ENCLAVE_CID,
            "--eif-path",
            eif_path,
        ];
        if debug_mode {
            info!("Debug mode enabled...");
            run_args.push("--debug-mode");
        } else {
            info!("Debug mode disabled...");
        }

        Self::run_command_capture_stdout(&run_args).await?;
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

    async fn run_command_capture_stdout(args: &[&str]) -> Result<String, EnclaveError> {
        let output = TokioCommand::new(args[0])
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

impl Drop for Enclave {
    fn drop(&mut self) {
        if let Some(handle) = self.supervisor.take() {
            handle.abort();
        }

        if let Ok(rt) = tokio::runtime::Handle::try_current() {
            rt.spawn(async {
                if let Err(e) = Self::shutdown_all_enclaves().await {
                    warn!(error = %e, "failed to terminate enclaves on shutdown");
                }
            });
        } else {
            let _ = std::process::Command::new(NITRO_CLI)
                .args([NitroCommand::TerminateEnclave.as_str(), "--all"])
                .stderr(Stdio::inherit())
                .output();
        }
    }
}
