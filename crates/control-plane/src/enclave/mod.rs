//! Nitro enclave supervisor: keep the enclave running and restart on failure.

mod artifact;
mod nitro;

pub use artifact::RuntimeEif;

use crate::constants::{ENCLAVE_HEALTH_POLL, MAX_BACKOFF_SECS, NITRO_CLI};
use nitro::{
    NitroCommand, attach_debug_consoles, describe_enclaves_stdout, run_enclave_once,
    shutdown_all_enclaves,
};
use serde_json::Value;
use std::process::Stdio;
use std::time::Duration;
use tokio::task::JoinHandle;
use tracing::{error, info, warn};

/// Supervises a Nitro enclave lifecycle on the host.
pub struct Enclave {
    debug_mode: bool,
    cpu_count: u32,
    memory_mib: u32,
    artifact: RuntimeEif,
    supervisor: Option<JoinHandle<()>>,
}

impl Enclave {
    #[must_use]
    pub const fn new(
        artifact: RuntimeEif,
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

    /// Stops the supervisor and terminates all running enclaves.
    pub async fn shutdown(mut self) -> anyhow::Result<()> {
        if let Some(handle) = self.supervisor.take() {
            handle.abort();
        }
        shutdown_all_enclaves()
            .await
            .map_err(|e| anyhow::anyhow!("{e}"))?;
        Ok(())
    }

    async fn run_loop(
        debug_mode: bool,
        cpu_count: u32,
        memory_mib: u32,
        artifact: RuntimeEif,
    ) -> anyhow::Result<()> {
        let mut backoff_secs: u64 = 0;
        let eif_path = artifact.path().display().to_string();

        loop {
            match describe_enclaves_stdout().await {
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
                        sleep_backoff(&mut backoff_secs).await;
                        continue;
                    }
                },
                Err(e) => {
                    warn!(error = %e, "describe-enclaves failed; backing off");
                    sleep_backoff(&mut backoff_secs).await;
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

            match run_enclave_once(debug_mode, cpu_count, memory_mib, eif_path.as_str()).await {
                Ok(()) => {
                    telemetry::metrics::record_enclave_restart("started");
                    info!("Enclave started... Waiting 5 seconds for warmup.");
                    tokio::time::sleep(Duration::from_secs(5)).await;
                    if debug_mode && let Err(e) = attach_debug_consoles().await {
                        warn!(error = %e, "debug log attach failed; continuing supervision");
                    }
                    backoff_secs = 1;
                }
                Err(e) => {
                    telemetry::metrics::record_enclave_restart("failed");
                    warn!(error = %e, "run-enclave failed");
                    advance_backoff(&mut backoff_secs);
                }
            }
        }
    }
}

async fn sleep_backoff(backoff_secs: &mut u64) {
    let wait = (*backoff_secs).clamp(1, MAX_BACKOFF_SECS);
    tokio::time::sleep(Duration::from_secs(wait)).await;
    advance_backoff(backoff_secs);
}

fn advance_backoff(backoff_secs: &mut u64) {
    *backoff_secs = if *backoff_secs == 0 {
        1
    } else {
        (*backoff_secs * 2).min(MAX_BACKOFF_SECS)
    };
}

impl Drop for Enclave {
    fn drop(&mut self) {
        if let Some(handle) = self.supervisor.take() {
            handle.abort();
        }

        if let Ok(rt) = tokio::runtime::Handle::try_current() {
            rt.spawn(async {
                if let Err(e) = shutdown_all_enclaves().await {
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

#[cfg(test)]
mod tests {
    use super::advance_backoff;
    use crate::constants::MAX_BACKOFF_SECS;

    #[test]
    fn backoff_doubles_until_cap() {
        let mut secs = 0;
        advance_backoff(&mut secs);
        assert_eq!(secs, 1);
        advance_backoff(&mut secs);
        assert_eq!(secs, 2);
        advance_backoff(&mut secs);
        assert_eq!(secs, 4);
        for _ in 0..20 {
            advance_backoff(&mut secs);
        }
        assert_eq!(secs, MAX_BACKOFF_SECS);
    }
}
