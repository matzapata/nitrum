use crate::error::NitrumConfigError;
use crate::sections::{
    Egress, HealthCheck, Project, ProjectName, Runtime, Scaling, TlsTermination, WellKnown,
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
        Ok(())
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
        let cfg: Self = toml::from_str(&contents).map_err(|source| NitrumConfigError::Parse {
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
