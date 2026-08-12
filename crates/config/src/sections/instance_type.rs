//! EC2 instance types allowed for Nitro Enclave cloud deploys.

use super::scaling::ScalingError;

/// Host vCPUs and RAM reserved for the parent (control-plane, gvproxy, OS).
pub const HOST_VCPU_RESERVE: u32 = 2;
pub const HOST_MEMORY_RESERVE_MIB: u32 = 2048;

/// Capacity of a Nitro Enclave–capable instance type in the allowlist.
#[derive(Debug, Clone, Copy)]
pub struct InstanceTypeCapacity {
    /// EC2 instance type name (e.g. `m6i.xlarge`).
    pub name: &'static str,
    /// Total instance vCPUs.
    pub vcpus: u32,
    /// Total instance memory in MiB.
    pub memory_mib: u32,
}

impl InstanceTypeCapacity {
    /// Maximum enclave vCPUs that fit after reserving [`HOST_VCPU_RESERVE`] for the parent.
    #[must_use]
    pub const fn max_enclave_cpus(&self) -> u32 {
        self.vcpus.saturating_sub(HOST_VCPU_RESERVE)
    }

    /// Maximum enclave RAM (MiB) after reserving [`HOST_MEMORY_RESERVE_MIB`] for the parent.
    #[must_use]
    pub const fn max_enclave_memory_mib(&self) -> u32 {
        self.memory_mib.saturating_sub(HOST_MEMORY_RESERVE_MIB)
    }
}

/// Nitro Enclave–capable instance types Nitrum validates against `[scaling]`.
///
/// AWS requires at least 4 vCPUs for Nitro Enclaves; types below that are omitted.
pub const ALLOWED_INSTANCE_TYPES: &[InstanceTypeCapacity] = &[
    InstanceTypeCapacity {
        name: "m6i.xlarge",
        vcpus: 4,
        memory_mib: 16_384,
    },
    InstanceTypeCapacity {
        name: "m6i.2xlarge",
        vcpus: 8,
        memory_mib: 32_768,
    },
    InstanceTypeCapacity {
        name: "m6i.4xlarge",
        vcpus: 16,
        memory_mib: 65_536,
    },
    InstanceTypeCapacity {
        name: "m6a.xlarge",
        vcpus: 4,
        memory_mib: 16_384,
    },
    InstanceTypeCapacity {
        name: "m6a.2xlarge",
        vcpus: 8,
        memory_mib: 32_768,
    },
    InstanceTypeCapacity {
        name: "m6a.4xlarge",
        vcpus: 16,
        memory_mib: 65_536,
    },
    InstanceTypeCapacity {
        name: "c6i.xlarge",
        vcpus: 4,
        memory_mib: 8_192,
    },
    InstanceTypeCapacity {
        name: "c6i.2xlarge",
        vcpus: 8,
        memory_mib: 16_384,
    },
    InstanceTypeCapacity {
        name: "c6i.4xlarge",
        vcpus: 16,
        memory_mib: 32_768,
    },
    InstanceTypeCapacity {
        name: "m5.xlarge",
        vcpus: 4,
        memory_mib: 16_384,
    },
    InstanceTypeCapacity {
        name: "m5.2xlarge",
        vcpus: 8,
        memory_mib: 32_768,
    },
];

/// Look up an allowlisted instance type by name.
#[must_use]
pub fn lookup_instance_type(name: &str) -> Option<&'static InstanceTypeCapacity> {
    ALLOWED_INSTANCE_TYPES.iter().find(|t| t.name == name)
}

/// Validate that `instance_type` is allowlisted and that enclave CPU/RAM fit.
///
/// # Errors
///
/// Returns [`ScalingError`] when the type is unknown or enclave resources exceed capacity.
pub fn validate_enclave_fit(
    instance_type: &str,
    num_cpus: u32,
    ram_size_mib: u32,
) -> Result<(), ScalingError> {
    let Some(cap) = lookup_instance_type(instance_type) else {
        let names: Vec<_> = ALLOWED_INSTANCE_TYPES.iter().map(|t| t.name).collect();
        return Err(ScalingError(format!(
            "`scaling.instance_type` ({instance_type}) is not in the Nitro Enclave allowlist; \
             choose one of: {}",
            names.join(", ")
        )));
    };
    let max_cpus = cap.max_enclave_cpus();
    if num_cpus > max_cpus {
        return Err(ScalingError(format!(
            "`scaling.num_cpus` ({num_cpus}) exceeds max enclave vCPUs ({max_cpus}) for \
             `{instance_type}` (instance has {} vCPUs, reserves {HOST_VCPU_RESERVE} for the parent)",
            cap.vcpus
        )));
    }
    let max_mem = cap.max_enclave_memory_mib();
    if ram_size_mib > max_mem {
        return Err(ScalingError(format!(
            "`scaling.ram_size_mib` ({ram_size_mib}) exceeds max enclave memory ({max_mem} MiB) for \
             `{instance_type}` (instance has {} MiB, reserves {HOST_MEMORY_RESERVE_MIB} MiB for the parent)",
            cap.memory_mib
        )));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn m6i_xlarge_fits_default_enclave() {
        validate_enclave_fit("m6i.xlarge", 2, 4320).expect("default enclave fits m6i.xlarge");
    }

    #[test]
    fn rejects_unknown_type() {
        let err = validate_enclave_fit("t3.micro", 2, 4320).expect_err("t3.micro not allowed");
        assert!(err.to_string().contains("allowlist"));
    }

    #[test]
    fn rejects_too_many_cpus() {
        let err = validate_enclave_fit("m6i.xlarge", 4, 4320).expect_err("4 cpus leave no parent");
        assert!(err.to_string().contains("num_cpus"));
    }

    #[test]
    fn rejects_too_much_memory() {
        let err =
            validate_enclave_fit("c6i.xlarge", 2, 7000).expect_err("c6i.xlarge only has 8 GiB");
        assert!(err.to_string().contains("ram_size_mib"));
    }
}
