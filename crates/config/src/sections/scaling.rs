#[derive(Debug, Clone, thiserror::Error)]
#[error("{0}")]
pub struct ScalingError(String);

/// `[scaling]` in `nitrum.toml`.
#[derive(Debug, Clone, serde::Serialize)]
pub struct Scaling {
    /// Desired number of replicas.
    pub desired_replicas: u32,

    /// Maximum number of replicas.
    pub max_replicas: u32,

    /// Minimum number of replicas.
    pub min_replicas: u32,

    /// Number of CPUs.
    pub num_cpus: std::num::NonZeroU32,

    /// RAM size in MB for the enclave.
    pub ram_size_mib: std::num::NonZeroU32,
}

impl Scaling {
    fn try_from_raw(raw: ScalingRaw) -> Result<Self, ScalingError> {
        if raw.min_replicas > raw.max_replicas {
            return Err(ScalingError(format!(
                "`scaling.min_replicas` ({}) must be <= `scaling.max_replicas` ({})",
                raw.min_replicas, raw.max_replicas
            )));
        }
        if raw.desired_replicas < raw.min_replicas || raw.desired_replicas > raw.max_replicas {
            return Err(ScalingError(format!(
                "`scaling.desired_replicas` ({}) must be between min ({}) and max ({})",
                raw.desired_replicas, raw.min_replicas, raw.max_replicas
            )));
        }
        let num_cpus = std::num::NonZeroU32::new(raw.num_cpus)
            .ok_or_else(|| ScalingError("`scaling.num_cpus` must be at least 1".to_string()))?;
        let ram_size_mib = std::num::NonZeroU32::new(raw.ram_size_mib).ok_or_else(|| {
            ScalingError("`scaling.ram_size_mib` must be greater than 0".to_string())
        })?;
        Ok(Self {
            desired_replicas: raw.desired_replicas,
            max_replicas: raw.max_replicas,
            min_replicas: raw.min_replicas,
            num_cpus,
            ram_size_mib,
        })
    }
}

impl Default for Scaling {
    fn default() -> Self {
        Self::try_from_raw(ScalingRaw {
            desired_replicas: 1,
            max_replicas: 1,
            min_replicas: 1,
            num_cpus: 2,
            ram_size_mib: 4320,
        })
        .expect("default scaling is valid")
    }
}

#[derive(serde::Deserialize)]
struct ScalingRaw {
    desired_replicas: u32,
    max_replicas: u32,
    min_replicas: u32,
    num_cpus: u32,
    ram_size_mib: u32,
}

impl<'de> serde::Deserialize<'de> for Scaling {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let raw = ScalingRaw::deserialize(deserializer)?;
        Self::try_from_raw(raw).map_err(serde::de::Error::custom)
    }
}

#[cfg(test)]
mod tests {
    use super::Scaling;

    #[test]
    fn rejects_invalid_replica_bounds() {
        let error = toml::from_str::<Scaling>(
            r"
            desired_replicas = 2
            max_replicas = 1
            min_replicas = 1
            num_cpus = 2
            ram_size_mib = 4320
            ",
        )
        .expect_err("desired outside min/max should fail");
        assert!(error.to_string().contains("desired_replicas"));
    }
}
