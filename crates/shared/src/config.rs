#[derive(Clone, serde::Deserialize)]
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
#[derive(Clone, serde::Deserialize)]
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

#[derive(Clone, serde::Deserialize)]
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

#[derive(Clone, serde::Deserialize)]
pub struct Egress {
    /// When false, all outbound traffic is allowed regardless of whitelist.
    pub enabled: bool,

    // TODO: implement this
    /// Regex patterns for allowed destination hostnames.
    /// Only evaluated when enabled = true. An empty list allows all DNS-resolved traffic.
    pub destinations: Vec<String>,
}

impl Default for Egress {
    fn default() -> Self {
        Self {
            enabled: false,
            destinations: vec![],
        }
    }
}

#[derive(Clone, serde::Deserialize)]
pub struct Service {
    /// Port to use for the service.
    pub port: u16,
}

impl Default for Service {
    fn default() -> Self {
        Self { port: 8080 }
    }
}

#[derive(Clone, serde::Deserialize)]
pub struct Config {
    /// Project slug: CloudFormation `EnvironmentName`, stack name, bucket `nitrum-{name}` (SSM via `NITRUM_SSM_PREFIX` or `/nitrum/…` defaults).
    pub name: String,
    pub service: Service,
    pub health_check: HealthCheck,
    pub scaling: Scaling,
    pub tls_termination: TlsTermination,
    pub egress: Egress,
}

/// Load `nitrum.toml` without panicking (for CLIs and tools).
pub fn try_load(path: &std::path::Path) -> Result<Config, String> {
    let contents = std::fs::read_to_string(path)
        .map_err(|e| format!("failed to read config file {}: {e}", path.display()))?;
    toml::from_str(&contents).map_err(|e| format!("failed to parse config file {}: {e}", path.display()))
}

pub fn load(path: &std::path::Path) -> Config {
    try_load(path).unwrap_or_else(|e| panic!("{e}"))
}
