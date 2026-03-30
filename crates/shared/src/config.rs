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
    #[error("invalid config in {path}: {message}")]
    Invalid { path: PathBuf, message: String },
    #[error("invalid `project.name` override: {message}")]
    NameOverrideInvalid { message: String },
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

impl HealthCheck {
    pub fn validate(&self) -> Result<(), String> {
        let path = self.path.trim();
        if path.is_empty() {
            return Err("`health_check.path` must not be empty".to_string());
        }
        if path != self.path {
            return Err(
                "`health_check.path` must not have leading or trailing whitespace".to_string(),
            );
        }
        if !path.starts_with('/') {
            return Err("`health_check.path` must start with `/`".to_string());
        }
        if self.port == 0 {
            return Err("`health_check.port` must not be 0".to_string());
        }
        if self.interval == 0 {
            return Err("`health_check.interval` must be greater than 0".to_string());
        }
        Ok(())
    }
}

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

impl Scaling {
    /// Returns `Err` with a human-readable message when `[scaling]` constraints are violated.
    pub fn validate(&self) -> Result<(), String> {
        if self.min_replicas > self.max_replicas {
            return Err(format!(
                "`scaling.min_replicas` ({}) must be <= `scaling.max_replicas` ({})",
                self.min_replicas, self.max_replicas
            ));
        }
        if self.desired_replicas < self.min_replicas || self.desired_replicas > self.max_replicas {
            return Err(format!(
                "`scaling.desired_replicas` ({}) must be between min ({}) and max ({})",
                self.desired_replicas, self.min_replicas, self.max_replicas
            ));
        }
        if self.num_cpus == 0 {
            return Err("`scaling.num_cpus` must be at least 1".to_string());
        }
        if self.ram_size_mib == 0 {
            return Err("`scaling.ram_size_mib` must be greater than 0".to_string());
        }
        Ok(())
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
            domain: "nitrum.local".to_string(),
        }
    }
}

impl TlsTermination {
    pub fn validate(&self) -> Result<(), String> {
        let domain = self.domain.trim();
        if domain.is_empty() {
            return Err("`tls_termination.domain` must not be empty".to_string());
        }
        if domain != self.domain {
            return Err(
                "`tls_termination.domain` must not have leading or trailing whitespace".to_string(),
            );
        }
        if domain.chars().any(|c| c.is_whitespace()) {
            return Err("`tls_termination.domain` must not contain whitespace".to_string());
        }
        if domain.len() > 253 {
            return Err("`tls_termination.domain` exceeds 253 characters (DNS limit)".to_string());
        }
        if self.acme && domain.ends_with(".local") {
            return Err(
                "`tls_termination.domain` cannot end with `.local` when `tls_termination.acme` is true; public CAs do not issue for private-use names".to_string(),
            );
        }
        Ok(())
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

impl Service {
    pub fn validate(&self) -> Result<(), String> {
        if self.port == 0 {
            return Err("`service.port` must not be 0".to_string());
        }
        Ok(())
    }
}

/// `[project]` in `nitrum.toml`.
#[derive(Clone, serde::Deserialize, serde::Serialize)]
pub struct Project {
    pub name: String,
}

impl Project {
    pub fn validate(&self) -> Result<(), String> {
        validate_project_name(&self.name)
    }
}

/// `[runtime]` in `nitrum.toml`: Docker images for Nitrum platform components.
#[derive(Clone, serde::Deserialize, serde::Serialize)]
pub struct Runtime {
    pub data_plane: String,

    pub control_plane: String,
    /// Docker image for `nitro-cli` (used by `nitrum build` / `nitrum describe` for EIF tooling).
    pub nitro_cli: String,
}

impl Default for Runtime {
    fn default() -> Self {
        Self {
            data_plane: "matzapata/nitrum-data-plane:latest".to_string(),
            control_plane: "matzapata/nitrum-control-plane:latest".to_string(),
            nitro_cli: "matzapata/nitrum-nitro-cli:latest".to_string(),
        }
    }
}

impl Runtime {
    pub fn validate(&self) -> Result<(), String> {
        validate_docker_image_ref(&self.data_plane, "runtime.data_plane")?;
        validate_docker_image_ref(&self.control_plane, "runtime.control_plane")?;
        validate_docker_image_ref(&self.nitro_cli, "runtime.nitro_cli")?;
        Ok(())
    }
}

#[derive(Clone, serde::Deserialize, serde::Serialize)]
pub struct NitrumConfig {
    pub project: Project,
    #[serde(default)]
    pub runtime: Runtime,
    pub service: Service,
    pub health_check: HealthCheck,
    pub scaling: Scaling,
    pub tls_termination: TlsTermination,
}

impl NitrumConfig {
    /// Validates semantic constraints beyond TOML shape.
    pub fn validate(&self) -> Result<(), String> {
        self.project.validate()?;
        self.runtime.validate()?;
        self.service.validate()?;
        self.health_check.validate()?;
        self.scaling.validate()?;
        self.tls_termination.validate()?;
        Ok(())
    }

    /// Replace [`Project::name`] when `override_name` is [`Some`], using the same rules as `project.name` in `nitrum.toml`.
    pub fn with_name(mut self, override_name: Option<String>) -> Result<Self, NitrumConfigError> {
        let Some(n) = override_name else {
            return Ok(self);
        };
        validate_project_name(&n)
            .map_err(|message| NitrumConfigError::NameOverrideInvalid { message })?;
        self.project.name = n;
        Ok(self)
    }
}

fn validate_docker_image_ref(value: &str, key: &str) -> Result<(), String> {
    let s = value.trim();
    if s.is_empty() {
        return Err(format!(
            "`{key}` must not be empty (expected a Docker image, e.g. registry/repo:tag)"
        ));
    }
    if s != value {
        return Err(format!(
            "`{key}` must not have leading or trailing whitespace"
        ));
    }
    if s.contains(char::is_whitespace) {
        return Err(format!("`{key}` must not contain whitespace"));
    }
    if !s.contains('/') {
        return Err(format!(
            "`{key}` should look like `registry/repo:tag` or `user/repo:tag` (must contain `/`)"
        ));
    }
    Ok(())
}

/// Rules for [`Project::name`]: Must be compatible with CloudFormation, SSM, Docker and S3
pub fn validate_project_name(name: &str) -> Result<(), String> {
    if name != name.trim() {
        return Err("`project.name` must not have leading or trailing whitespace".to_string());
    }
    if name.is_empty() {
        return Err("`project.name` must not be empty".to_string());
    }

    let mut chars = name.chars();
    let Some(first) = chars.next() else {
        return Err("`project.name` must not be empty".to_string());
    };
    if !matches!(first, 'a'..='z') {
        return Err("`project.name` must start with a lowercase letter (a-z)".to_string());
    }

    let mut rest_len = 0usize;
    for c in chars {
        rest_len += 1;
        if !matches!(c, 'a'..='z' | '0'..='9' | '-') {
            return Err(format!(
                "`project.name` after the first character must use only lowercase letters, digits, or hyphens (invalid character {c:?})"
            ));
        }
    }
    if !(2..=127).contains(&rest_len) {
        return Err(format!(
            "`project.name` must be 3-128 characters (one leading letter plus 2-127 more); got {} character(s) after the first",
            rest_len
        ));
    }

    Ok(())
}

impl TryFrom<&std::path::Path> for NitrumConfig {
    type Error = NitrumConfigError;

    fn try_from(path: &std::path::Path) -> Result<Self, Self::Error> {
        let path_buf = path.to_path_buf();
        let contents = std::fs::read_to_string(path).map_err(|source| NitrumConfigError::Read {
            path: path_buf.clone(),
            source,
        })?;
        let cfg: NitrumConfig =
            toml::from_str(&contents).map_err(|source| NitrumConfigError::Parse {
                path: path_buf.clone(),
                source,
            })?;
        cfg.validate()
            .map_err(|message| NitrumConfigError::Invalid {
                path: path_buf,
                message,
            })?;
        Ok(cfg)
    }
}
