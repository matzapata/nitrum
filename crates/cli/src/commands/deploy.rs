//! Fetch CDK infra from GitHub into `.nitrum/infra` and run `cdk deploy`.

use clap::Args;
use indicatif::ProgressBar;
use std::env;
use std::path::PathBuf;

use crate::constants;
use crate::utils::{cdk, console, github};

#[derive(Args)]
pub struct DeployArgs {
    /// Project directory (default: current directory)
    #[arg(short, long)]
    pub path: Option<PathBuf>,
    /// Deployment environment for infra (`dev` or `prod`)
    #[arg(long, value_name = "ENV", default_value = "dev", value_parser = ["dev", "prod"])]
    pub env: String,
    /// Path to EIF file to deploy (default: `enclave.eif` in project directory)
    #[arg(long, value_name = "PATH")]
    pub eif: Option<PathBuf>,
    /// Extra arguments forwarded to `cdk deploy`
    #[arg(trailing_var_arg = true, num_args = 0..)]
    pub cdk_args: Vec<String>,
}

pub async fn run(args: DeployArgs) {
    let root = args
        .path
        .unwrap_or_else(|| env::current_dir().expect("current directory"));
    let infra_dir = root.join(constants::NITRUM_INFRA_DIR);
    let eif_path = match &args.eif {
        Some(p) if p.is_absolute() => p.clone(),
        Some(p) => root.join(p),
        None => root.join("enclave.eif"),
    };
    let eif_path = eif_path
        .canonicalize()
        .unwrap_or_else(|_| eif_path.clone());
    if !eif_path.is_file() {
        eprintln!("EIF file not found: {}", eif_path.display());
        std::process::exit(1);
    }

    if !infra_dir.exists() {
        let spinner = console::style_spinner(
            ProgressBar::new_spinner(),
            "Fetching infra from GitHub…",
        );
        match github::extract_infra_folder(
            constants::NITRUM_GITHUB_REPO_OWNER,
            constants::NITRUM_GITHUB_REPO_NAME,
            constants::NITRUM_GITHUB_REF,
            &infra_dir,
        )
        .await
        {
            Ok(()) => spinner.finish_with_message(format!(
                "Infra ready at `{}`.",
                constants::NITRUM_INFRA_DIR
            )),
            Err(e) => {
                spinner.finish_and_clear();
                eprintln!("{e:#}");
                std::process::exit(1);
            }
        }
    }

    if !console::confirm(&format!(
        "Deploy AWS resources ({} deployment)?",
        args.env,
    )) {
        return;
    }

    let env_vars = [
        ("DEPLOYMENT", args.env.clone()),
        ("EIF_PATH", eif_path.display().to_string()),
    ];
    let mut deploy_args = vec![
        "NitrumStack".to_string(),
        "-O".to_string(),
        "out.json".to_string(),
        "--require-approval".to_string(),
        "never".to_string(),
    ];
    deploy_args.extend(args.cdk_args.clone());

    if let Err(e) = cdk::run_cdk(&infra_dir, "deploy", &deploy_args, &env_vars).await {
        eprintln!("{e:#}");
        std::process::exit(1);
    }
}
