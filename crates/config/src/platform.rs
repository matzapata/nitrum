use crate::sections::{Project, ProjectName};

/// AWS naming contract for a Nitrum deployment: SSM paths, log groups, stack, and bucket names.
///
/// Parameterized by [`ProjectName`] so multiple projects can share the same layout template.
/// CloudFormation (`stack.yml`) creates resources at these paths; CLI and data-plane consume them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlatformLayout {
    /// Project identifier from `nitrum.toml` (`project.name`).
    project_name: ProjectName,
}

impl PlatformLayout {
    /// Builds layout paths for `project_name`.
    #[must_use]
    pub const fn new(project_name: ProjectName) -> Self {
        Self { project_name }
    }

    /// Builds layout paths from `[project]` in `nitrum.toml`.
    #[must_use]
    pub fn from_project(project: &Project) -> Self {
        Self::new(project.name.clone())
    }

    /// Borrow the project name used in all path templates.
    #[must_use]
    pub const fn project_name(&self) -> &ProjectName {
        &self.project_name
    }

    /// CloudFormation stack name (`nitrum-{project}`).
    #[must_use]
    pub fn stack_name(&self) -> String {
        format!("nitrum-{}", self.project_name)
    }

    /// EIF artifact S3 bucket (`nitrum-{project}`).
    #[must_use]
    pub fn s3_bucket(&self) -> String {
        format!("nitrum-{}", self.project_name)
    }

    /// SSM parameter for the data-plane KMS key id.
    #[must_use]
    pub fn kms_key_id_param(&self) -> String {
        format!("/nitrum/{}/data-plane/kms_key_id", self.project_name)
    }

    /// SSM parameter for the data-plane DynamoDB table name.
    #[must_use]
    pub fn dynamodb_table_param(&self) -> String {
        format!("/nitrum/{}/data-plane/dynamodb_table", self.project_name)
    }

    /// Prefix for application env parameters (no trailing slash).
    #[must_use]
    pub fn app_env_prefix(&self) -> String {
        format!("/nitrum/{}/env", self.project_name)
    }

    /// Prefix for recursive SSM `GetParametersByPath` on app env (trailing slash).
    #[must_use]
    pub fn app_env_path_prefix(&self) -> String {
        format!("/nitrum/{}/env/", self.project_name)
    }

    /// SSM name for one application env key.
    #[must_use]
    pub fn app_env_key(&self, key: &str) -> String {
        format!("{}/{}", self.app_env_prefix(), key)
    }

    /// Data-plane infra SSM path prefix.
    #[must_use]
    pub fn data_plane_ssm_prefix(&self) -> String {
        format!("/nitrum/{}/data-plane", self.project_name)
    }

    /// CloudWatch log group for the data-plane.
    #[must_use]
    pub fn data_plane_log_group(&self) -> String {
        format!("/nitrum/{}/data-plane", self.project_name)
    }

    /// CloudWatch log group for the control-plane.
    #[must_use]
    pub fn control_plane_log_group(&self) -> String {
        format!("/nitrum/{}/control-plane", self.project_name)
    }

    /// CloudWatch log group for ADOT metrics (EMF).
    #[must_use]
    pub fn metrics_log_group(&self) -> String {
        format!("/nitrum/{}/metrics", self.project_name)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn layout(name: &str) -> PlatformLayout {
        PlatformLayout::new(name.parse().expect("valid test project name"))
    }

    #[test]
    fn paths_match_cloudformation_contract() {
        let layout = layout("myapp");
        assert_eq!(layout.stack_name(), "nitrum-myapp");
        assert_eq!(layout.s3_bucket(), "nitrum-myapp");
        assert_eq!(
            layout.kms_key_id_param(),
            "/nitrum/myapp/data-plane/kms_key_id"
        );
        assert_eq!(
            layout.dynamodb_table_param(),
            "/nitrum/myapp/data-plane/dynamodb_table"
        );
        assert_eq!(layout.app_env_prefix(), "/nitrum/myapp/env");
        assert_eq!(layout.app_env_path_prefix(), "/nitrum/myapp/env/");
        assert_eq!(layout.app_env_key("API_KEY"), "/nitrum/myapp/env/API_KEY");
        assert_eq!(layout.data_plane_ssm_prefix(), "/nitrum/myapp/data-plane");
        assert_eq!(
            layout.control_plane_log_group(),
            "/nitrum/myapp/control-plane"
        );
    }
}
