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
