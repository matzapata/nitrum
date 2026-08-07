//! `[cloud]` in `nitrum.toml` — CloudFormation-only knobs (ignored by `nitrum local`).

use super::Scaling;

#[derive(Debug, Clone, thiserror::Error)]
#[error("{0}")]
pub struct CloudError(pub String);

/// CloudWatch Logs retention values accepted by `AWS::Logs::LogGroup`.
pub const ALLOWED_LOG_RETENTION_DAYS: &[u16] = &[
    1, 3, 5, 7, 14, 30, 60, 90, 120, 150, 180, 365, 400, 545, 731, 1096, 1827, 2192, 2557, 2922,
    3288, 3653,
];

/// `[cloud]` section: production vs cheap-dev CloudFormation settings.
#[derive(Debug, Clone, serde::Serialize)]
pub struct Cloud {
    /// When true, ADOT exports traces to X-Ray (costly; default off for development).
    pub xray_tracing: bool,

    /// CloudWatch Logs retention in days for data-plane, control-plane, and metrics groups.
    pub log_retention_days: u16,

    /// Optional SNS topic ARN for CloudWatch alarms. Empty / omitted → no alarms.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sns_alarm_topic_arn: Option<String>,

    /// When true, ASG rolling updates keep ≥1 instance in service and pause for enclave boot.
    pub safe_rolling: bool,

    /// KMS key administrator principal. Empty / omitted → account root (`AWS_ACCOUNT_ROOT`).
    /// Must be re-passed on every deploy (durable in TOML) so CFN does not reset to root.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub kms_administrator_role_arn: Option<String>,
}

impl Default for Cloud {
    fn default() -> Self {
        Self {
            xray_tracing: false,
            log_retention_days: 7,
            sns_alarm_topic_arn: None,
            safe_rolling: true,
            kms_administrator_role_arn: None,
        }
    }
}

impl Cloud {
    fn try_from_raw(raw: CloudRaw) -> Result<Self, CloudError> {
        if !ALLOWED_LOG_RETENTION_DAYS.contains(&raw.log_retention_days) {
            return Err(CloudError(format!(
                "`cloud.log_retention_days` ({}) is not a valid CloudWatch Logs retention; \
                 choose one of: {}",
                raw.log_retention_days,
                ALLOWED_LOG_RETENTION_DAYS
                    .iter()
                    .map(ToString::to_string)
                    .collect::<Vec<_>>()
                    .join(", ")
            )));
        }

        let sns_alarm_topic_arn = normalize_optional_arn(raw.sns_alarm_topic_arn);
        let kms_administrator_role_arn = normalize_optional_arn(raw.kms_administrator_role_arn);

        if let Some(ref arn) = sns_alarm_topic_arn
            && !arn.starts_with("arn:aws:sns:")
        {
            return Err(CloudError(format!(
                "`cloud.sns_alarm_topic_arn` must be an SNS topic ARN (got {arn})"
            )));
        }

        if let Some(ref arn) = kms_administrator_role_arn
            && arn != "AWS_ACCOUNT_ROOT"
            && !arn.starts_with("arn:aws:iam::")
        {
            return Err(CloudError(format!(
                "`cloud.kms_administrator_role_arn` must be an IAM principal ARN or empty for \
                 account root (got {arn})"
            )));
        }

        Ok(Self {
            xray_tracing: raw.xray_tracing,
            log_retention_days: raw.log_retention_days,
            sns_alarm_topic_arn,
            safe_rolling: raw.safe_rolling,
            kms_administrator_role_arn,
        })
    }

    /// CFN `KmsAdministratorRoleArn` value: configured ARN or `AWS_ACCOUNT_ROOT`.
    #[must_use]
    pub fn kms_administrator_cfn_value(&self) -> &str {
        self.kms_administrator_role_arn
            .as_deref()
            .unwrap_or("AWS_ACCOUNT_ROOT")
    }

    /// Validate cross-field rules against `[scaling]` (safe rolling needs max headroom).
    ///
    /// # Errors
    ///
    /// Returns [`CloudError`] when `safe_rolling` is true but `max_replicas` has no spare capacity.
    pub fn validate_with_scaling(&self, scaling: &Scaling) -> Result<(), CloudError> {
        if self.safe_rolling && scaling.max_replicas < scaling.desired_replicas.saturating_add(1) {
            return Err(CloudError(format!(
                "`cloud.safe_rolling` requires `scaling.max_replicas` >= desired_replicas + 1 \
                 (got max={}, desired={}); raise max_replicas so ASG can launch a replacement \
                 before terminating the old instance",
                scaling.max_replicas, scaling.desired_replicas
            )));
        }
        Ok(())
    }
}

fn normalize_optional_arn(value: Option<String>) -> Option<String> {
    value.and_then(|s| {
        let t = s.trim().to_string();
        if t.is_empty() { None } else { Some(t) }
    })
}

const fn default_true() -> bool {
    true
}

const fn default_log_retention_days() -> u16 {
    7
}

#[derive(serde::Deserialize)]
struct CloudRaw {
    #[serde(default)]
    xray_tracing: bool,
    #[serde(default = "default_log_retention_days")]
    log_retention_days: u16,
    #[serde(default)]
    sns_alarm_topic_arn: Option<String>,
    #[serde(default = "default_true")]
    safe_rolling: bool,
    #[serde(default)]
    kms_administrator_role_arn: Option<String>,
}

impl<'de> serde::Deserialize<'de> for Cloud {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let raw = CloudRaw::deserialize(deserializer)?;
        Self::try_from_raw(raw).map_err(serde::de::Error::custom)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Scaling;

    #[test]
    fn defaults_are_dev_cheap() {
        let cloud = Cloud::default();
        assert!(!cloud.xray_tracing);
        assert_eq!(cloud.log_retention_days, 7);
        assert!(cloud.sns_alarm_topic_arn.is_none());
        assert!(cloud.safe_rolling);
        assert!(cloud.kms_administrator_role_arn.is_none());
        assert_eq!(cloud.kms_administrator_cfn_value(), "AWS_ACCOUNT_ROOT");
    }

    #[test]
    fn safe_rolling_requires_max_headroom() {
        let cloud = Cloud {
            safe_rolling: true,
            ..Cloud::default()
        };
        let scaling = Scaling {
            desired_replicas: 1,
            max_replicas: 1,
            min_replicas: 1,
            num_cpus: std::num::NonZeroU32::new(2).unwrap(),
            ram_size_mib: std::num::NonZeroU32::new(4320).unwrap(),
            instance_type: "m6i.xlarge".into(),
        };
        let err = cloud
            .validate_with_scaling(&scaling)
            .expect_err("max==desired blocks safe rolling");
        assert!(err.to_string().contains("safe_rolling"));
    }

    #[test]
    fn safe_rolling_ok_with_headroom() {
        let cloud = Cloud::default();
        let scaling = Scaling::default();
        cloud
            .validate_with_scaling(&scaling)
            .expect("default scaling has max=2");
    }

    #[test]
    fn rejects_invalid_retention() {
        let err = toml::from_str::<Cloud>("log_retention_days = 2").expect_err("2 is invalid");
        assert!(err.to_string().contains("log_retention_days"));
    }

    #[test]
    fn empty_arns_become_none() {
        let cloud: Cloud = toml::from_str(
            r#"
            sns_alarm_topic_arn = ""
            kms_administrator_role_arn = ""
            "#,
        )
        .expect("empty strings ok");
        assert!(cloud.sns_alarm_topic_arn.is_none());
        assert!(cloud.kms_administrator_role_arn.is_none());
    }
}
