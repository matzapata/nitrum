pub mod artifact;

mod error;
mod nitrum;
mod platform;
mod sections;

pub use error::NitrumConfigError;
pub use nitrum::NitrumConfig;
pub use platform::PlatformLayout;
pub use sections::{
    DockerImageRef, DockerImageRefError, Egress, HealthCheck, Project, ProjectName,
    ProjectNameError, Runtime, Scaling, TlsTermination, WellKnown, validate_project_name,
};
