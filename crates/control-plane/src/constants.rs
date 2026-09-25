//! Shared control-plane configuration (paths, timeouts, enclave defaults).

use std::time::Duration;

/// Path to the nitro-cli binary.
pub const NITRO_CLI: &str = "nitro-cli";

/// Directory for S3-downloaded EIFs (relative to process cwd): `{ARTIFACTS_DIR}/{hash}.eif`.
pub const ARTIFACTS_DIR: &str = ".nitrum/artifacts";

/// CID passed to `nitro-cli run-enclave --enclave-cid`.
pub const ENCLAVE_CID: &str = "16";

/// How often we check whether the enclave is still listed by `describe-enclaves`.
pub const ENCLAVE_HEALTH_POLL: Duration = Duration::from_secs(5);

/// Max delay before a restart attempt after the enclave is gone or `run-enclave` fails.
pub const MAX_BACKOFF_SECS: u64 = 300;

/// After `run-enclave`, how long the enclave must stay listed before the launch
/// counts as stable (backoff reset). Disappear earlier → crash-loop backoff.
/// Ingress readiness is left to the NLB; the control-plane only watches nitro lifecycle.
pub const ENCLAVE_SETTLE_TIMEOUT: Duration = Duration::from_secs(30);
