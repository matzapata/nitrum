//! `CloudFormation` stack helpers.

use anyhow::{Context, Result, bail};
use aws_sdk_cloudformation::error::SdkError as CfSdkError;
use aws_sdk_cloudformation::operation::RequestId;
use aws_sdk_cloudformation::operation::describe_stacks::DescribeStacksError;
use aws_sdk_cloudformation::types::{Capability, Parameter, StackStatus};
use std::collections::BTreeMap;
use std::time::Duration;
use tokio::time::sleep;
use tracing::{debug, info, warn};

use super::template::StackTemplate;

/// `CloudFormation` stack scoped to a client and stack name.
pub struct CloudFormation {
    client: aws_sdk_cloudformation::Client,
    stack_name: String,
}

impl CloudFormation {
    #[must_use]
    pub fn new(aws_sdk_config: &aws_config::SdkConfig, stack_name: impl Into<String>) -> Self {
        Self {
            client: aws_sdk_cloudformation::Client::new(aws_sdk_config),
            stack_name: stack_name.into(),
        }
    }

    #[must_use]
    pub fn stack_name(&self) -> &str {
        &self.stack_name
    }

    /// Returns whether the stack exists and is not `DELETE_COMPLETE`.
    ///
    /// # Errors
    ///
    /// Returns an error when `DescribeStacks` fails for reasons other than a
    /// missing stack.
    pub async fn exists(&self) -> Result<bool> {
        let stack_name = self.stack_name.as_str();
        let describe = self
            .client
            .describe_stacks()
            .stack_name(stack_name)
            .send()
            .await;
        match describe {
            Ok(ref resp) => {
                let st = resp.stacks().first();
                let status = st.and_then(|s| s.stack_status());
                let exists = status.is_some_and(|s| *s != StackStatus::DeleteComplete);
                info!(
                    %stack_name,
                    ?status,
                    stacks_returned = resp.stacks().len(),
                    exists,
                    "DescribeStacks OK"
                );
                Ok(exists)
            }
            Err(e) => {
                if describe_stacks_reports_missing_stack(&e) {
                    info!(%stack_name, "stack not found");
                    Ok(false)
                } else {
                    warn!(%stack_name, error = ?e, "DescribeStacks failed");
                    Err(e.into())
                }
            }
        }
    }

    /// Creates the stack if missing; otherwise runs `UpdateStack` and treats “no updates” as success.
    /// Returns `true` when the caller should [`Self::wait_until_stable`].
    ///
    /// # Errors
    ///
    /// Returns an error when `CreateStack`/`UpdateStack` or `DescribeStacks`
    /// fail in a way that cannot be recovered automatically.
    pub async fn update_if_needed(
        &self,
        params: &[(String, String)],
        template: &StackTemplate,
    ) -> Result<bool> {
        let stack_name = self.stack_name.as_str();
        let parameters = stack_parameters(params);

        info!(%stack_name, param_count = params.len(), "DescribeStacks (create vs update)");
        let exists = self.exists().await?;

        if !exists {
            info!(%stack_name, "CreateStack");
            apply_create_template(
                self.client
                    .create_stack()
                    .stack_name(stack_name)
                    .set_parameters(Some(parameters))
                    .capabilities(Capability::CapabilityIam)
                    .capabilities(Capability::CapabilityNamedIam),
                template,
            )
            .send()
            .await
            .context("CreateStack failed")?;
            info!(%stack_name, "CreateStack accepted; waiting for completion");
            return Ok(true);
        }

        info!(%stack_name, "UpdateStack");
        let upd = apply_update_template(
            self.client
                .update_stack()
                .stack_name(stack_name)
                .set_parameters(Some(parameters))
                .capabilities(Capability::CapabilityIam)
                .capabilities(Capability::CapabilityNamedIam),
            template,
        )
        .send()
        .await;

        match upd {
            Ok(_) => {
                info!(%stack_name, "UpdateStack accepted; waiting for completion");
                Ok(true)
            }
            Err(e) => {
                let s = e.to_string();
                let dbg = format!("{e:?}");
                if s.contains("No updates are to be performed")
                    || dbg.contains("No updates are to be performed")
                {
                    info!(%stack_name, "no template/parameter changes — skipping wait");
                    Ok(false)
                } else {
                    warn!(%stack_name, display = %s, debug = %dbg, "UpdateStack failed");
                    Err(e.into())
                }
            }
        }
    }

    /// Polls until `CREATE_COMPLETE` or `UPDATE_COMPLETE`, or returns an error on failure.
    ///
    /// # Errors
    ///
    /// Returns an error when the stack enters a terminal failure state or when
    /// polling fails due to AWS API errors.
    pub async fn wait_until_stable(&self) -> Result<()> {
        wait_stack_stable(&self.client, self.stack_name.as_str()).await
    }

    /// Output keys mapped to values (non-empty keys only).
    ///
    /// # Errors
    ///
    /// Returns an error when `DescribeStacks` fails or when the response is
    /// missing a stack or outputs unexpectedly.
    pub async fn outputs(&self) -> Result<BTreeMap<String, String>> {
        let stack_name = self.stack_name.as_str();
        let resp = self
            .client
            .describe_stacks()
            .stack_name(stack_name)
            .send()
            .await
            .with_context(|| format!("DescribeStacks for stack `{stack_name}` outputs"))?;
        let stack = resp
            .stacks()
            .first()
            .with_context(|| format!("DescribeStacks returned no stacks for `{stack_name}`"))?;
        let mut map = BTreeMap::new();
        for o in stack.outputs() {
            let key = o.output_key().unwrap_or("").trim();
            if key.is_empty() {
                continue;
            }
            if let Some(v) = o.output_value() {
                map.insert(key.to_string(), v.to_string());
            }
        }
        Ok(map)
    }

    /// Deletes the stack (if present and not already gone) and waits until deletion finishes.
    ///
    /// # Errors
    ///
    /// Returns an error when `DescribeStacks`, `DeleteStack`, or subsequent
    /// polling fails, or when the stack transitions to a delete failure state.
    pub async fn destroy(&self) -> Result<()> {
        let stack_name = self.stack_name.as_str();

        info!(%stack_name, "destroy: DescribeStacks");
        let describe = self
            .client
            .describe_stacks()
            .stack_name(stack_name)
            .send()
            .await;
        let need_cf_delete = match describe {
            Ok(resp) => resp.stacks().first().map_or_else(
                || {
                    info!(%stack_name, "DescribeStacks returned no stacks — treating as gone");
                    false
                },
                |st| {
                    let status = st.stack_status();
                    if matches!(status, Some(StackStatus::DeleteComplete)) {
                        info!(%stack_name, "stack already DELETE_COMPLETE — nothing to do");
                        false
                    } else {
                        info!(%stack_name, ?status, "stack exists — will DeleteStack");
                        true
                    }
                },
            ),
            Err(e) => {
                if describe_stacks_reports_missing_stack(&e) {
                    info!(%stack_name, "stack does not exist — nothing to delete");
                    false
                } else {
                    warn!(%stack_name, error = ?e, "DescribeStacks failed");
                    return Err(e.into());
                }
            }
        };

        if need_cf_delete {
            info!(%stack_name, "DeleteStack");
            self.client
                .delete_stack()
                .stack_name(stack_name)
                .send()
                .await
                .context("DeleteStack failed")?;
            info!(%stack_name, "DeleteStack accepted; polling until gone");

            let mut n = 0u32;
            loop {
                let desc = self
                    .client
                    .describe_stacks()
                    .stack_name(stack_name)
                    .send()
                    .await;
                match desc {
                    Err(e) => {
                        if describe_stacks_reports_missing_stack(&e) {
                            info!(%stack_name, "DescribeStacks: stack gone after delete");
                            break;
                        }
                        warn!(%stack_name, error = ?e, "DescribeStacks during delete wait");
                        return Err(e.into());
                    }
                    Ok(resp) => {
                        let st = resp.stacks().first();
                        let status = st.and_then(|x| x.stack_status());
                        n = n.saturating_add(1);
                        if n == 1 || n.is_multiple_of(6) {
                            info!(%stack_name, ?status, "delete wait poll");
                        } else {
                            debug!(%stack_name, ?status, "delete wait poll");
                        }
                        match status {
                            Some(StackStatus::DeleteComplete) | None => {
                                info!(%stack_name, "stack delete finished");
                                break;
                            }
                            Some(StackStatus::DeleteFailed) => {
                                bail!("stack `{stack_name}` delete failed");
                            }
                            Some(StackStatus::DeleteInProgress) => {
                                sleep(Duration::from_secs(5)).await;
                            }
                            _ => {
                                sleep(Duration::from_secs(5)).await;
                            }
                        }
                    }
                }
            }
        }

        Ok(())
    }
}

fn describe_stacks_reports_missing_stack<R>(err: &CfSdkError<DescribeStacksError, R>) -> bool {
    if let CfSdkError::ServiceError(ctx) = err {
        let e = ctx.err();
        let code = e.meta().code();
        let msg = e.meta().message();
        let missing =
            code == Some("ValidationError") && msg.is_some_and(|m| m.contains("does not exist"));
        debug!(
            code,
            message = msg,
            request_id = e.request_id(),
            missing_stack = missing,
            "DescribeStacks service error"
        );
        missing
    } else {
        debug!(error = %err, "DescribeStacks non-service error");
        false
    }
}

fn stack_parameters(params: &[(String, String)]) -> Vec<Parameter> {
    params
        .iter()
        .map(|(k, v)| {
            Parameter::builder()
                .parameter_key(k)
                .parameter_value(v)
                .build()
        })
        .collect()
}

fn apply_create_template(
    builder: aws_sdk_cloudformation::operation::create_stack::builders::CreateStackFluentBuilder,
    template: &StackTemplate,
) -> aws_sdk_cloudformation::operation::create_stack::builders::CreateStackFluentBuilder {
    match template {
        StackTemplate::Body(body) => builder.template_body(body),
        StackTemplate::Url(url) => builder.template_url(url),
    }
}

fn apply_update_template(
    builder: aws_sdk_cloudformation::operation::update_stack::builders::UpdateStackFluentBuilder,
    template: &StackTemplate,
) -> aws_sdk_cloudformation::operation::update_stack::builders::UpdateStackFluentBuilder {
    match template {
        StackTemplate::Body(body) => builder.template_body(body),
        StackTemplate::Url(url) => builder.template_url(url),
    }
}

const fn stack_in_progress(status: Option<&StackStatus>) -> bool {
    matches!(
        status,
        Some(
            StackStatus::CreateInProgress
                | StackStatus::RollbackInProgress
                | StackStatus::DeleteInProgress
                | StackStatus::UpdateInProgress
                | StackStatus::UpdateCompleteCleanupInProgress
                | StackStatus::UpdateRollbackInProgress
                | StackStatus::ReviewInProgress
                | StackStatus::ImportInProgress
                | StackStatus::ImportRollbackInProgress
        )
    )
}

const fn stack_failed(status: Option<&StackStatus>) -> bool {
    matches!(
        status,
        Some(
            StackStatus::CreateFailed
                | StackStatus::RollbackFailed
                | StackStatus::RollbackComplete
                | StackStatus::DeleteFailed
                | StackStatus::UpdateFailed
                | StackStatus::UpdateRollbackFailed
                | StackStatus::UpdateRollbackComplete
                | StackStatus::ImportRollbackFailed
        )
    )
}

async fn wait_stack_stable(
    client: &aws_sdk_cloudformation::Client,
    stack_name: &str,
) -> Result<()> {
    let mut tick: u32 = 0;
    loop {
        let resp = client
            .describe_stacks()
            .stack_name(stack_name)
            .send()
            .await
            .context("DescribeStacks while waiting")?;
        let stack = resp.stacks().first().context("stack missing during wait")?;
        let status = stack.stack_status();
        tick = tick.saturating_add(1);
        if tick == 1 || tick.is_multiple_of(6) {
            info!(
                %stack_name,
                ?status,
                reason = stack.stack_status_reason(),
                "CloudFormation stack wait poll"
            );
        } else {
            debug!(%stack_name, ?status, "CloudFormation stack wait poll");
        }
        if matches!(
            status,
            Some(StackStatus::CreateComplete | StackStatus::UpdateComplete)
        ) {
            info!(%stack_name, ?status, "CloudFormation stack reached stable success");
            return Ok(());
        }
        if stack_failed(status) {
            let reason = stack.stack_status_reason().unwrap_or("no reason returned");
            bail!("stack `{stack_name}` failed: {status:?} — {reason}");
        }
        if stack_in_progress(status) || status.is_none() {
            sleep(Duration::from_secs(5)).await;
            continue;
        }
        bail!("stack `{stack_name}` unexpected status: {status:?}");
    }
}
