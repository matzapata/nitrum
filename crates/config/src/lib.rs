pub mod artifact;

mod error;
mod platform;
mod sections;

pub use error::NitrumConfigError;
pub use platform::PlatformLayout;
pub use sections::{
    ALLOWED_INSTANCE_TYPES, ALLOWED_LOG_RETENTION_DAYS, Cloud, CloudError, DockerImageRef,
    DockerImageRefError, ENV_RUNTIME_CONTROL_PLANE_IMAGE, ENV_RUNTIME_DATA_PLANE_IMAGE,
    ENV_RUNTIME_NITRO_CLI_IMAGE, Egress, EgressPattern, EgressPatternError,
    HOST_MEMORY_RESERVE_MIB, HOST_VCPU_RESERVE, HealthCheck, HealthCheckPath, HealthCheckPathError,
    InstanceTypeCapacity, Project, ProjectName, ProjectNameError, Runtime, Scaling, ScalingError,
    TlsDomain, TlsDomainError, TlsTermination, TlsTerminationError, lookup_instance_type,
    validate_enclave_fit,
};

#[derive(Clone, serde::Deserialize, serde::Serialize)]
pub struct NitrumConfig {
    pub project: Project,
    #[serde(default)]
    pub runtime: Runtime,
    pub health_check: HealthCheck,
    pub scaling: Scaling,
    pub tls_termination: TlsTermination,
    #[serde(default)]
    pub egress: Egress,
    /// CloudFormation-only settings (`nitrum local` ignores this section).
    #[serde(default)]
    pub cloud: Cloud,
}

impl NitrumConfig {
    /// Validate `[cloud]` rules that depend on `[scaling]` (e.g. safe rolling headroom).
    ///
    /// Call before cloud deploy. Not enforced on parse so local-only configs with
    /// `max_replicas == desired` still load when `[cloud]` uses defaults.
    ///
    /// # Errors
    ///
    /// Returns [`CloudError`] when cloud settings are inconsistent with scaling.
    pub fn validate_cloud(&self) -> Result<(), CloudError> {
        self.cloud.validate_with_scaling(&self.scaling)
    }

    /// Replace [`Project::name`] when `override_name` is [`Some`], using the same
    /// rules as `project.name` in `nitrum.toml`.
    ///
    /// # Errors
    ///
    /// Returns [`NitrumConfigError::NameOverrideInvalid`] when the provided name
    /// does not satisfy [`ProjectName::try_new`].
    pub fn with_name(mut self, override_name: Option<String>) -> Result<Self, NitrumConfigError> {
        let Some(n) = override_name else {
            return Ok(self);
        };
        self.project.name =
            ProjectName::try_new(&n).map_err(|error| NitrumConfigError::NameOverrideInvalid {
                message: error.to_string(),
            })?;
        Ok(self)
    }
}

impl TryFrom<&std::path::Path> for NitrumConfig {
    type Error = NitrumConfigError;

    fn try_from(path: &std::path::Path) -> Result<Self, Self::Error> {
        let path_buf = path.to_path_buf();
        let contents = std::fs::read_to_string(path).map_err(|source| NitrumConfigError::Read {
            path: path_buf.clone(),
            source,
        })?;
        let mut config: Self =
            toml::from_str(&contents).map_err(|source| NitrumConfigError::Parse {
                path: path_buf,
                source,
            })?;
        config.runtime.apply_env_overrides()?;
        Ok(config)
    }
}
