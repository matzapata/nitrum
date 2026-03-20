use anyhow::{Context, Result, bail};
use std::path::Path;
use std::process::Stdio;
use tokio::process::Command;

/// Runs `npx cdk <subcommand> …extra` in `infra_dir` with inherited stdio.
pub async fn run_cdk(
    infra_dir: &Path,
    subcommand: &str,
    extra_args: &[String],
    env_vars: &[(&str, String)],
) -> Result<()> {
    let mut npm_install = Command::new("npm");
    npm_install
        .arg("install")
        .arg("--silent")
        .current_dir(infra_dir)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    for (key, value) in env_vars {
        npm_install.env(key, value);
    }

    let install_output = npm_install
        .output()
        .await
        .context("failed to spawn `npm install` (is Node.js/npm installed?)")?;

    if !install_output.status.success() {
        let stderr = String::from_utf8_lossy(&install_output.stderr);
        let stdout = String::from_utf8_lossy(&install_output.stdout);
        let detail = [&*stderr, &*stdout]
            .into_iter()
            .map(str::trim)
            .find(|s| !s.is_empty())
            .unwrap_or("");

        bail!(
            "npm install exited with status {}{}{}",
            install_output.status,
            if detail.is_empty() { "" } else { "\n\n" },
            detail
        );
    }

    let mut cmd = Command::new("npx");
    cmd.arg("cdk").arg(subcommand);
    for a in extra_args {
        cmd.arg(a);
    }
    cmd.current_dir(infra_dir)
        .stdin(Stdio::inherit())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit());
    for (key, value) in env_vars {
        cmd.env(key, value);
    }

    let status = cmd
        .status()
        .await
        .context("failed to spawn `cdk` (is it installed globally?)")?;

    if !status.success() {
        bail!("cdk {subcommand} exited with status {status}");
    }
    Ok(())
}
