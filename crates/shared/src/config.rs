use std::path::PathBuf;

#[derive(Debug, thiserror::Error)]
pub enum NitrumConfigError {
    #[error("failed to read config file {path}: {source}")]
    Read {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("failed to parse config file {path}: {source}")]
    Parse {
        path: PathBuf,
        #[source]
        source: toml::de::Error,
    },
}

#[derive(Clone, serde::Deserialize, serde::Serialize)]
pub struct HealthCheck {
    /// Path to the health check endpoint.
    pub path: String,

    /// Port to use for the health check.
    pub port: u16,

    /// Interval in seconds to wait between health checks.
    pub interval: u32,
}

impl Default for HealthCheck {
    fn default() -> Self {
        Self {
            path: "/health".to_string(),
            port: 8080,
            interval: 10,
        }
    }
}

// TODO: contemplate this in deployment
#[derive(Clone, serde::Deserialize, serde::Serialize)]
pub struct Scaling {
    /// Desired number of replicas.
    pub desired_replicas: u32,

    /// Maximum number of replicas.
    pub max_replicas: u32,

    /// Minimum number of replicas.
    pub min_replicas: u32,

    /// Number of CPUs.
    pub num_cpus: u32,

    /// RAM size in MB for the enclave.
    pub ram_size_mib: u32,
}

impl Default for Scaling {
    fn default() -> Self {
        Self {
            desired_replicas: 1,
            max_replicas: 1,
            min_replicas: 1,
            num_cpus: 2,
            ram_size_mib: 4320,
        }
    }
}

#[derive(Clone, serde::Deserialize, serde::Serialize)]
pub struct TlsTermination {
    /// Use ACME (e.g. Let's Encrypt) for certificate issuance instead of self-signed.
    pub acme: bool,

    /// Domain name used for TLS certificate generation (self-signed or ACME).
    pub domain: String,
}

impl Default for TlsTermination {
    fn default() -> Self {
        Self {
            acme: false,
            domain: "nitrum.local".to_string(), // TODO: add instructions to add this to /etc/hosts
        }
    }
}

#[derive(Clone, serde::Deserialize, serde::Serialize)]
pub struct Service {
    /// Port to use for the service.
    pub port: u16,
}

impl Default for Service {
    fn default() -> Self {
        Self { port: 8080 }
    }
}

#[derive(Clone, serde::Deserialize, serde::Serialize)]
pub struct NitrumConfig {
    pub name: String,
    pub service: Service,
    pub health_check: HealthCheck,
    pub scaling: Scaling,
    pub tls_termination: TlsTermination,
}

impl TryFrom<&std::path::Path> for NitrumConfig {
    type Error = NitrumConfigError;

    fn try_from(path: &std::path::Path) -> Result<Self, Self::Error> {
        let path_buf = path.to_path_buf();
        let contents = std::fs::read_to_string(path).map_err(|source| NitrumConfigError::Read {
            path: path_buf.clone(),
            source,
        })?;
        toml::from_str(&contents).map_err(|source| NitrumConfigError::Parse {
            path: path_buf,
            source,
        })
    }
}
