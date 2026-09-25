//! Control-plane startup orchestration.

pub mod config;

use crate::enclave::{Enclave, RuntimeEif};
use crate::networking;
use crate::storage::Bucket;
use anyhow::Context;
use tracing::info;

pub use config::{ControlPlaneConfig, EifSource};

/// Initializes networking and enclave supervision; shuts down cleanly on SIGINT.
///
/// # Errors
///
/// Returns an error when artifact resolution, networking, or shutdown steps fail.
pub async fn run(config: ControlPlaneConfig) -> anyhow::Result<()> {
    let runtime_eif = resolve_eif(&config).await?;

    info!("starting networking");
    let networking = networking::init().await.context("networking init")?;

    info!("starting enclave supervisor");
    let mut enclave = Enclave::new(
        runtime_eif,
        config.debug_mode,
        config.cpu_count,
        config.memory_mib,
    );
    enclave.run();

    info!("waiting for shutdown signal");
    tokio::signal::ctrl_c()
        .await
        .context("failed to listen for ctrl_c")?;
    info!("received SIGINT, shutting down");

    enclave.shutdown().await.context("enclave shutdown")?;
    networking.shutdown().await.context("networking shutdown")?;

    Ok(())
}

async fn resolve_eif(config: &ControlPlaneConfig) -> anyhow::Result<RuntimeEif> {
    match &config.eif {
        EifSource::Local(path) => RuntimeEif::try_from_local(path.clone()),
        EifSource::Bucket {
            bucket,
            version_label,
        } => {
            let sdk = aws_config::load_defaults(aws_config::BehaviorVersion::latest()).await;
            let bucket = Bucket::new(&sdk, bucket.as_str());
            RuntimeEif::try_from_bucket(bucket, version_label.as_str()).await
        }
    }
}
