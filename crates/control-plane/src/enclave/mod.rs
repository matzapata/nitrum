//! Nitro enclave supervisor: keep the enclave running and restart on failure.

mod artifact;
mod nitro;

pub use artifact::RuntimeEif;

use crate::constants::{
    ENCLAVE_HEALTH_POLL, ENCLAVE_SETTLE_TIMEOUT, MAX_BACKOFF_SECS, NITRO_CLI,
};
use nitro::{
    NitroCommand, attach_debug_consoles, describe_enclaves_stdout, run_enclave_once,
    shutdown_all_enclaves,
};
use serde_json::Value;
use std::process::Stdio;
use std::time::{Duration, Instant};
use tokio::task::JoinHandle;
use tracing::{error, info, warn};

/// Outcome of watching `describe-enclaves` after a successful `run-enclave`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LaunchOutcome {
    /// Enclave stayed listed for the settle window.
    Stable,
    /// `describe-enclaves` went empty before the settle window elapsed.
    ExitedBeforeStable { uptime_ms: u128 },
}

/// How a launch outcome updates backoff / fast-crash counters.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct LaunchCounters {
    backoff_secs: u64,
    consecutive_fast_crashes: u64,
}

/// Apply [`LaunchOutcome`] to supervisor counters (pure; unit-tested).
fn apply_launch_outcome(outcome: LaunchOutcome, mut counters: LaunchCounters) -> LaunchCounters {
    match outcome {
        LaunchOutcome::Stable => {
            counters.consecutive_fast_crashes = 0;
            counters.backoff_secs = 0;
        }
        LaunchOutcome::ExitedBeforeStable { .. } => {
            counters.consecutive_fast_crashes = counters.consecutive_fast_crashes.saturating_add(1);
            advance_backoff(&mut counters.backoff_secs);
        }
    }
    counters
}

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
        let mut counters = LaunchCounters {
            backoff_secs: 0,
            consecutive_fast_crashes: 0,
        };
        let eif_path = artifact.path().display().to_string();

        loop {
            match describe_enclaves_stdout().await {
                Ok(raw) => match serde_json::from_str::<Value>(&raw) {
                    Ok(enclaves) => {
                        let empty: Vec<Value> = vec![];
                        let enclaves_array = enclaves.as_array().unwrap_or(&empty);
                        if !enclaves_array.is_empty() {
                            // While listed, keep a mild backoff floor for the next restart.
                            if counters.backoff_secs == 0 {
                                counters.backoff_secs = 1;
                            }
                            tokio::time::sleep(ENCLAVE_HEALTH_POLL).await;
                            continue;
                        }
                    }
                    Err(e) => {
                        warn!(error = %e, "describe-enclaves returned invalid JSON; backing off");
                        sleep_backoff(&mut counters.backoff_secs).await;
                        continue;
                    }
                },
                Err(e) => {
                    warn!(error = %e, "describe-enclaves failed; backing off");
                    sleep_backoff(&mut counters.backoff_secs).await;
                    continue;
                }
            }

            if counters.backoff_secs > 0 {
                info!(
                    seconds = counters.backoff_secs.min(MAX_BACKOFF_SECS),
                    "waiting before enclave (re)start"
                );
                tokio::time::sleep(Duration::from_secs(
                    counters.backoff_secs.min(MAX_BACKOFF_SECS),
                ))
                .await;
            }

            match run_enclave_once(debug_mode, cpu_count, memory_mib, eif_path.as_str()).await {
                Ok(()) => {
                    info!("enclave launched; watching for early exit");
                    let started = Instant::now();
                    let outcome = wait_until_settled_or_gone(started).await;
                    counters = apply_launch_outcome(outcome, counters);

                    match outcome {
                        LaunchOutcome::Stable => {
                            telemetry::metrics::record_enclave_restart("started");
                            info!("enclave stayed listed through settle window");
                            if debug_mode && let Err(e) = attach_debug_consoles().await {
                                warn!(error = %e, "debug log attach failed; continuing supervision");
                            }
                        }
                        LaunchOutcome::ExitedBeforeStable { uptime_ms } => {
                            telemetry::metrics::record_enclave_restart("exited_before_stable");
                            error!(
                                uptime_ms,
                                consecutive_fast_crashes = counters.consecutive_fast_crashes,
                                last_reason = "exited_before_stable",
                                "enclave exited before settle window"
                            );
                        }
                    }
                }
                Err(e) => {
                    telemetry::metrics::record_enclave_restart("failed");
                    warn!(error = %e, "run-enclave failed");
                    advance_backoff(&mut counters.backoff_secs);
                }
            }
        }
    }
}

/// Poll `describe-enclaves` until the settle window elapses or the enclave disappears.
async fn wait_until_settled_or_gone(started: Instant) -> LaunchOutcome {
    let deadline = started + ENCLAVE_SETTLE_TIMEOUT;
    loop {
        match describe_enclaves_listed().await {
            Ok(true) => {}
            Ok(false) => {
                return LaunchOutcome::ExitedBeforeStable {
                    uptime_ms: started.elapsed().as_millis(),
                };
            }
            Err(e) => {
                warn!(error = %e, "describe-enclaves failed during settle watch");
            }
        }

        if Instant::now() >= deadline {
            return LaunchOutcome::Stable;
        }
        tokio::time::sleep(ENCLAVE_HEALTH_POLL).await;
    }
}

async fn describe_enclaves_listed() -> Result<bool, nitro::NitroError> {
    let raw = describe_enclaves_stdout().await?;
    let enclaves: Value = serde_json::from_str(&raw)?;
    Ok(enclaves.as_array().is_some_and(|a| !a.is_empty()))
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
    use super::{LaunchCounters, LaunchOutcome, advance_backoff, apply_launch_outcome};
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

    #[test]
    fn exited_before_stable_does_not_reset_backoff() {
        let before = LaunchCounters {
            backoff_secs: 4,
            consecutive_fast_crashes: 1,
        };
        let after = apply_launch_outcome(
            LaunchOutcome::ExitedBeforeStable { uptime_ms: 800 },
            before,
        );
        assert_eq!(after.backoff_secs, 8);
        assert_eq!(after.consecutive_fast_crashes, 2);
    }

    #[test]
    fn stable_resets_backoff_and_fast_crashes() {
        let before = LaunchCounters {
            backoff_secs: 16,
            consecutive_fast_crashes: 5,
        };
        let after = apply_launch_outcome(LaunchOutcome::Stable, before);
        assert_eq!(after.backoff_secs, 0);
        assert_eq!(after.consecutive_fast_crashes, 0);
    }
}
