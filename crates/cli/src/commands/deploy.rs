//! Deploy with AWS CloudFormation (bundled `samples/hello` template) and S3 EIF upload.

use clap::Args;
use std::env;
use std::path::PathBuf;
use tracing::info;

use crate::utils::{aws, cloudformation, console, s3_bucket};

#[derive(Args)]
pub struct DeployArgs {
    /// Project directory (default: current directory)
    #[arg(short, long)]
    pub path: Option<PathBuf>,
    /// Retain KMS, DynamoDB, logs, and SSM on stack delete (`Retain=true` in CloudFormation)
    #[arg(long, action = clap::ArgAction::SetTrue)]
    pub retain: bool,
    /// Path to EIF file to deploy (default: `enclave.eif` in project directory)
    #[arg(long, value_name = "PATH")]
    pub eif: Option<PathBuf>,
    /// Docker tag for `matzapata/nitrum-control-plane` on instances
    #[arg(long, default_value = "latest")]
    pub control_plane_image_tag: String,
    /// AWS region (overrides `AWS_REGION` / `AWS_DEFAULT_REGION`)
    #[arg(long, value_name = "REGION")]
    pub region: Option<String>,
}

pub async fn run(args: DeployArgs) {
    info!("nitrum deploy: starting");
    let root = args
        .path
        .unwrap_or_else(|| env::current_dir().expect("current directory"));
    info!(project_root = %root.display(), "project directory");
    let eif_path = match &args.eif {
        Some(p) if p.is_absolute() => p.clone(),
        Some(p) => root.join(p),
        None => root.join("enclave.eif"),
    };
    let eif_path = eif_path.canonicalize().unwrap_or_else(|_| eif_path.clone());
    if !eif_path.is_file() {
        eprintln!("EIF file not found: {}", eif_path.display());
        std::process::exit(1);
    }

    let cfg_path = root.join("nitrum.toml");
    let config = match shared::config::try_load(&cfg_path) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("{e}");
            std::process::exit(1);
        }
    };

    let sdk = aws::sdk_config(args.region.clone()).await;

    let slug = config.name.clone();
    info!(%slug, "stack / EnvironmentName from nitrum.toml");

    let bucket = match s3_bucket::derived_eif_bucket_name(&config.name) {
        Ok(n) => n,
        Err(e) => {
            eprintln!("{e:#}");
            std::process::exit(1);
        }
    };

    let retain_str = if args.retain { "true" } else { "false" };

    info!(
        eif_path = %eif_path.display(),
        retain = args.retain,
        control_plane_tag = %args.control_plane_image_tag,
        cli_region = ?args.region,
        "deploy parameters"
    );

    let eif_label = match cloudformation::eif_version_label(&eif_path) {
        Ok(l) => l,
        Err(e) => {
            eprintln!("{e:#}");
            std::process::exit(1);
        }
    };

    let template_path = match cloudformation::write_bundled_template(&root) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("{e:#}");
            std::process::exit(1);
        }
    };
    info!(template_path = %template_path.display(), "bundled CloudFormation template written");

    let region_display = sdk
        .region()
        .map(|r| r.as_ref().to_string())
        .unwrap_or_else(aws::resolve_aws_region);

    if !console::confirm(&format!(
        "Deploy CloudFormation stack `{slug}` (EnvironmentName={slug}, Retain={retain_str}, region {region_display}, S3 `s3://{bucket}/{eif_label}`)?"
    )) {
        return;
    }

    let template_body = match std::fs::read_to_string(&template_path) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("read {}: {e}", template_path.display());
            std::process::exit(1);
        }
    };

    if let Err(e) = aws::ensure_bucket_exists(&sdk, &bucket).await {
        eprintln!("{e:#}");
        std::process::exit(1);
    }

    if let Err(e) = aws::s3_put_file_if_needed(&sdk, &bucket, &eif_label, &eif_path).await {
        eprintln!("{e:#}");
        std::process::exit(1);
    }

    let params = vec![
        ("EnvironmentName".to_string(), slug.clone()),
        ("Retain".to_string(), retain_str.to_string()),
        ("EifS3Bucket".to_string(), bucket.clone()),
        ("EifS3Key".to_string(), eif_label.clone()),
        ("EifVersionLabel".to_string(), eif_label),
        ("AsgMinSize".to_string(), "1".to_string()),
        ("AsgMaxSize".to_string(), "1".to_string()),
        ("AsgDesiredCapacity".to_string(), "1".to_string()),
        (
            "ControlPlaneImageTag".to_string(),
            args.control_plane_image_tag.clone(),
        ),
    ];

    if let Err(e) = aws::cloudformation_deploy(&sdk, &slug, &template_body, &params).await {
        eprintln!("{e:#}");
        std::process::exit(1);
    }

    let outputs = match aws::cloudformation_stack_outputs(&sdk, &slug).await {
        Ok(o) => o,
        Err(e) => {
            eprintln!("failed to fetch CloudFormation stack outputs: {e:#}");
            std::process::exit(1);
        }
    };

    let out_path = root.join("out.json");
    let mut envelope = serde_json::Map::new();
    envelope.insert(
        slug.clone(),
        serde_json::to_value(&outputs).expect("output map serializes to JSON"),
    );
    let out_file = match std::fs::File::create(&out_path) {
        Ok(f) => f,
        Err(e) => {
            eprintln!("create {}: {e}", out_path.display());
            std::process::exit(1);
        }
    };
    if let Err(e) = serde_json::to_writer_pretty(out_file, &serde_json::Value::Object(envelope)) {
        eprintln!("write {}: {e}", out_path.display());
        std::process::exit(1);
    }
    info!(path = %out_path.display(), "wrote CloudFormation outputs to out.json");

    info!(%slug, "nitrum deploy: finished successfully");
}
