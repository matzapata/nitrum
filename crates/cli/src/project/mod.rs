use anyhow::Result;
use config::NitrumConfig;
use std::env;
use std::path::PathBuf;

/// Loaded Nitrum project context for CLI commands.
pub struct CliProject {
    /// Project directory containing `nitrum.toml`.
    pub root: PathBuf,
    /// Parsed config, optionally overridden via `--as`.
    pub config: NitrumConfig,
}

impl CliProject {
    /// Resolves the project root and loads `nitrum.toml`.
    ///
    /// # Errors
    ///
    /// Returns an error when the config file cannot be read or parsed.
    pub fn load(path: Option<PathBuf>, as_name: Option<String>) -> Result<Self> {
        let root = path.unwrap_or_else(|| env::current_dir().expect("current directory"));
        let config =
            NitrumConfig::try_from(root.join("nitrum.toml").as_path())?.with_name(as_name)?;
        Ok(Self { root, config })
    }
}
