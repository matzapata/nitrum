use std::path::PathBuf;

/// Environment variable overriding [`NitrumConfig::imds_latest_base_url`]; ignored when unset.
///
/// Lets the same `nitrum.toml` work in local dev (docker-compose metadata mocks) and in real
/// deployments, where the IMDS base URL is always the standard EC2 address.
pub const ENV_IMDS_BASE_URL: &str = "NITRUM_IMDS_BASE_URL";

/// Environment variable overriding [`NitrumConfig::otlp_endpoint`]; ignored when unset. An empty
/// value explicitly disables OTLP export (see [`NitrumConfig::otlp_endpoint`]).
pub const ENV_OTLP_ENDPOINT: &str = "NITRUM_OTLP_ENDPOINT";

/// Default IMDS base URL when [`NitrumConfig::imds_latest_base_url`] is not set in `nitrum.toml`
/// and not overridden by [`ENV_IMDS_BASE_URL`] (includes `/latest`, no trailing slash).
pub const DEFAULT_IMDS_LATEST_BASE_URL: &str = "http://169.254.169.254/latest";

fn default_imds_latest_base_url() -> String {
    DEFAULT_IMDS_LATEST_BASE_URL.to_string()
}

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
    /// Validates semantic constraints for `[health_check]`.
    ///
    /// # Errors
    ///
    /// Returns `Err` with a human-readable message when any field is invalid
    /// (for example an empty path or zero interval).
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
    /// Validates semantic constraints for `[scaling]`.
    ///
    /// # Errors
    ///
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
    /// Validates semantic constraints for `[tls_termination]`.
    ///
    /// # Errors
    ///
    /// Returns `Err` with a human-readable message when the domain or ACME
    /// settings are invalid (for example an empty domain or ACME with `.local`).
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
        if domain.chars().any(char::is_whitespace) {
            return Err("`tls_termination.domain` must not contain whitespace".to_string());
        }
        if domain.len() > 253 {
            return Err("`tls_termination.domain` exceeds 253 characters (DNS limit)".to_string());
        }
        if self.acme
            && std::path::Path::new(domain)
                .extension()
                .is_some_and(|ext| ext.eq_ignore_ascii_case("local"))
        {
            return Err("`tls_termination.domain` cannot end with `.local` when `tls_termination.acme` is true; public CAs do not issue for private-use names".to_string());
        }
        Ok(())
    }
}

/// Outbound traffic restrictions enforced inside the data-plane.
#[derive(Clone, Default, serde::Deserialize, serde::Serialize)]
pub struct Egress {
    /// When false, all outbound traffic is allowed regardless of `destinations`.
    #[serde(default)]
    pub enabled: bool,

    /// Regex patterns matched against destination hostnames at DNS query time.
    /// An empty list blocks all DNS-resolved traffic when `enabled` is true.
    #[serde(default)]
    pub destinations: Vec<String>,
}

impl Egress {
    /// Validates semantic constraints for `[egress]`.
    ///
    /// # Errors
    ///
    /// Returns `Err` when any destination pattern is empty or not a valid regex.
    pub fn validate(&self) -> Result<(), String> {
        for (index, pattern) in self.destinations.iter().enumerate() {
            if pattern.trim().is_empty() {
                return Err(format!("`egress.destinations[{index}]` must not be empty"));
            }
            regex::Regex::new(pattern).map_err(|error| {
                format!("`egress.destinations[{index}]` invalid regex `{pattern}`: {error}")
            })?;
        }
        Ok(())
    }
}

/// Controls exposure of `/.well-known/enclave/*` routes on the ingress (TLS) listener.
#[derive(Clone, serde::Deserialize, serde::Serialize)]
pub struct WellKnown {
    /// When true, serve `GET /.well-known/enclave/status`.
    pub enclave_status: bool,
    /// When true, serve `GET /.well-known/enclave/attestation`.
    pub enclave_attestation: bool,
}

impl Default for WellKnown {
    fn default() -> Self {
        Self {
            enclave_status: true,
            enclave_attestation: true,
        }
    }
}

/// `[project]` in `nitrum.toml`.
#[derive(Clone, serde::Deserialize, serde::Serialize)]
pub struct Project {
    pub name: String,
    /// TCP port your application listens on (`127.0.0.1`); the ingress proxies here after TLS.
    pub port: u16,
    /// Process argv for the user workload (read from `nitrum.toml` by the data-plane).
    #[serde(default, alias = "command")]
    pub start_command: Vec<String>,
}

impl Project {
    /// Validates semantic constraints for `[project]`.
    ///
    /// # Errors
    ///
    /// Returns `Err` with a human-readable message when the project name is
    /// invalid according to [`validate_project_name`].
    pub fn validate(&self) -> Result<(), String> {
        validate_project_name(&self.name)?;
        if self.port == 0 {
            return Err("`project.port` must not be 0".to_string());
        }
        Ok(())
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
            data_plane: "ghcr.io/matzapata/nitrum/data-plane:latest".to_string(),
            control_plane: "ghcr.io/matzapata/nitrum/control-plane:latest".to_string(),
            nitro_cli: "ghcr.io/matzapata/nitrum/nitro-cli:latest".to_string(),
        }
    }
}

impl Runtime {
    /// Validates semantic constraints for `[runtime]`.
    ///
    /// # Errors
    ///
    /// Returns `Err` with a human-readable message when any image reference is
    /// empty, contains whitespace, or is otherwise malformed.
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
    #[serde(default)]
    pub well_known: WellKnown,
    pub health_check: HealthCheck,
    pub scaling: Scaling,
    pub tls_termination: TlsTermination,
    #[serde(default)]
    pub egress: Egress,

    /// IMDS base URL used to fetch the AWS region, instance ID, and credentials (includes
    /// `/latest`; trailing slash is stripped). Defaults to the standard EC2 IMDS address;
    /// override with [`ENV_IMDS_BASE_URL`] for local dev or metadata mocks.
    #[serde(default = "default_imds_latest_base_url")]
    pub imds_latest_base_url: String,

    /// OTLP/gRPC collector endpoint for telemetry export (e.g. `http://127.0.0.1:4317`).
    /// `None` means "not explicitly configured": callers may apply their own platform default
    /// or fall back to stdout-only logging. `Some("")` explicitly disables OTLP export.
    /// Override with [`ENV_OTLP_ENDPOINT`].
    #[serde(default)]
    pub otlp_endpoint: Option<String>,
}

impl NitrumConfig {
    /// Validates semantic constraints beyond TOML shape.
    ///
    /// # Errors
    ///
    /// Returns `Err` with a human-readable message when any nested section
    /// contains invalid values.
    pub fn validate(&self) -> Result<(), String> {
        self.project.validate()?;
        self.runtime.validate()?;
        self.health_check.validate()?;
        self.scaling.validate()?;
        self.tls_termination.validate()?;
        self.egress.validate()?;
        if self.imds_latest_base_url.trim().is_empty() {
            return Err("`imds_latest_base_url` must not be empty".to_string());
        }
        Ok(())
    }

    /// Applies [`ENV_IMDS_BASE_URL`] / [`ENV_OTLP_ENDPOINT`] overrides on top of the values read
    /// from `nitrum.toml`, so the same file works unmodified across local dev (docker-compose
    /// metadata/collector mocks) and real deployments.
    fn apply_env_overrides(&mut self) {
        if let Ok(value) = std::env::var(ENV_IMDS_BASE_URL) {
            let trimmed = value.trim();
            if !trimmed.is_empty() {
                self.imds_latest_base_url = trimmed.trim_end_matches('/').to_string();
            }
        }
        if let Ok(value) = std::env::var(ENV_OTLP_ENDPOINT) {
            self.otlp_endpoint = Some(value);
        }
    }

    /// Replace [`Project::name`] when `override_name` is [`Some`], using the same
    /// rules as `project.name` in `nitrum.toml`.
    ///
    /// # Errors
    ///
    /// Returns [`NitrumConfigError::NameOverrideInvalid`] when the provided name
    /// does not satisfy [`validate_project_name`].
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

/// Rules for [`Project::name`]: Must be compatible with `CloudFormation`, SSM, Docker and S3
///
/// # Errors
///
/// Returns `Err` with a human-readable message when `name` does not satisfy
/// Nitrum project naming rules.
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
    if !first.is_ascii_lowercase() {
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
            "`project.name` must be 3-128 characters (one leading letter plus 2-127 more); got {rest_len} character(s) after the first",
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
        let mut cfg: Self =
            toml::from_str(&contents).map_err(|source| NitrumConfigError::Parse {
                path: path_buf.clone(),
                source,
            })?;
        cfg.apply_env_overrides();
        cfg.validate()
            .map_err(|message| NitrumConfigError::Invalid {
                path: path_buf,
                message,
            })?;
        Ok(cfg)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn egress_rejects_invalid_regex() {
        let egress = Egress {
            enabled: true,
            destinations: vec!["(".to_string()],
        };
        assert!(egress.validate().is_err());
    }

    #[test]
    fn egress_accepts_valid_patterns() {
        let egress = Egress {
            enabled: true,
            destinations: vec![r"httpbin\.org$".to_string()],
        };
        assert!(egress.validate().is_ok());
    }

    fn minimal_config() -> NitrumConfig {
        NitrumConfig {
            project: Project {
                name: "nitrum-test".to_string(),
                port: 8080,
                start_command: vec![],
            },
            runtime: Runtime::default(),
            well_known: WellKnown::default(),
            health_check: HealthCheck::default(),
            scaling: Scaling::default(),
            tls_termination: TlsTermination::default(),
            egress: Egress::default(),
            imds_latest_base_url: default_imds_latest_base_url(),
            otlp_endpoint: None,
        }
    }

    #[test]
    fn env_overrides_apply_on_top_of_toml_defaults() {
        let imds_key = ENV_IMDS_BASE_URL;
        let otlp_key = ENV_OTLP_ENDPOINT;
        unsafe {
            std::env::remove_var(imds_key);
            std::env::remove_var(otlp_key);
        }

        let mut cfg = minimal_config();
        cfg.apply_env_overrides();
        assert_eq!(cfg.imds_latest_base_url, DEFAULT_IMDS_LATEST_BASE_URL);
        assert_eq!(cfg.otlp_endpoint, None);

        unsafe {
            std::env::set_var(imds_key, "http://imds:1338/latest/");
            std::env::set_var(otlp_key, "http://observability:4317");
        }
        let mut cfg = minimal_config();
        cfg.apply_env_overrides();
        assert_eq!(cfg.imds_latest_base_url, "http://imds:1338/latest");
        assert_eq!(
            cfg.otlp_endpoint,
            Some("http://observability:4317".to_string())
        );

        unsafe {
            std::env::set_var(otlp_key, "");
        }
        let mut cfg = minimal_config();
        cfg.apply_env_overrides();
        assert_eq!(cfg.otlp_endpoint, Some(String::new()));

        unsafe {
            std::env::remove_var(imds_key);
            std::env::remove_var(otlp_key);
        }
    }
}
