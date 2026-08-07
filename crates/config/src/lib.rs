pub mod artifact;

mod error;
mod platform;
mod sections;

pub use error::NitrumConfigError;
pub use platform::PlatformLayout;
pub use sections::{
    DockerImageRef, DockerImageRefError, ENV_RUNTIME_CONTROL_PLANE_IMAGE,
    ENV_RUNTIME_DATA_PLANE_IMAGE, ENV_RUNTIME_NITRO_CLI_IMAGE, Egress, EgressPattern,
    EgressPatternError, HealthCheck, HealthCheckPath, HealthCheckPathError, Project, ProjectName,
    ProjectNameError, Runtime, Scaling, ScalingError, TlsDomain, TlsDomainError, TlsTermination,
    TlsTerminationError, WellKnown,
};

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
}

impl NitrumConfig {
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
