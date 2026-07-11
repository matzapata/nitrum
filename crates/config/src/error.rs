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
    #[error("invalid `project.name` override: {message}")]
    NameOverrideInvalid { message: String },
}
