use anyhow::Result;
use clap::Args;
use config::NitrumConfig;
use std::collections::HashSet;
use std::env;
use std::path::PathBuf;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

#[derive(Clone, Copy, clap::ValueEnum)]
pub enum LogService {
    /// Host control-plane (`/nitrum/{project}/control-plane`).
    #[value(name = "control-plane")]
    ControlPlane,
    /// In-enclave data-plane (`/nitrum/{project}/data-plane`).
    #[value(name = "data-plane")]
    DataPlane,
}

#[derive(Args)]
pub struct LogsArgs {
    /// Use this project name instead of `project.name` in nitrum.toml (CloudFormation/S3/SSM/Docker tag)
    #[arg(long = "as", value_name = "NAME")]
    pub as_name: Option<String>,
    /// Which Nitrum service log group to tail.
    #[arg(long = "service", value_enum, default_value_t = LogService::ControlPlane)]
    pub service: LogService,
    /// Project directory (default: current directory)
    #[arg(short, long)]
    pub path: Option<PathBuf>,
    /// Follow new events
    #[arg(short, long)]
    pub follow: bool,
    /// Maximum events per poll request
    #[arg(long, default_value_t = 200)]
    pub limit: i32,
    /// Initial lookback window (minutes) before tailing/following
    #[arg(long, default_value_t = 15)]
    pub since_minutes: u64,
    /// Optional `CloudWatch` Logs filter pattern
    #[arg(long)]
    pub filter: Option<String>,
}

pub async fn run(args: LogsArgs) -> Result<()> {
    let root = args
        .path
        .unwrap_or_else(|| env::current_dir().expect("current directory"));
    let config = NitrumConfig::try_from(root.join("nitrum.toml").as_path())?
        .with_name(args.as_name.clone())?;

    let aws_sdk_config = aws_config::load_from_env().await;
    let client = aws_sdk_cloudwatchlogs::Client::new(&aws_sdk_config);
    let suffix = match args.service {
        LogService::ControlPlane => "control-plane",
        LogService::DataPlane => "data-plane",
    };
    let log_group = format!("/nitrum/{}/{}", config.project.name, suffix);

    let now_ms = now_epoch_millis();
    let lookback_ms = (args.since_minutes.saturating_mul(60).saturating_mul(1000)).cast_signed();
    let mut cursor_ms = now_ms.saturating_sub(lookback_ms);
    let mut seen_ids_at_cursor = HashSet::new();

    loop {
        let mut req = client
            .filter_log_events()
            .log_group_name(&log_group)
            .start_time(cursor_ms)
            .limit(args.limit.max(1));

        if let Some(ref pattern) = args.filter {
            req = req.filter_pattern(pattern);
        }

        let out = req.send().await?;
        let mut rows = out.events().to_vec();
        rows.sort_by_key(|e| e.timestamp().unwrap_or_default());

        for event in rows {
            let ts = event.timestamp().unwrap_or_default();
            let event_id = event.event_id().unwrap_or_default().to_string();
            if ts < cursor_ms {
                continue;
            }
            if ts == cursor_ms && !event_id.is_empty() && seen_ids_at_cursor.contains(&event_id) {
                continue;
            }

            let stream = event.log_stream_name().unwrap_or("-");
            let message = event.message().unwrap_or("").trim_end();
            println!("{ts} [{stream}] {message}");

            if ts > cursor_ms {
                cursor_ms = ts;
                seen_ids_at_cursor.clear();
            }
            if !event_id.is_empty() {
                seen_ids_at_cursor.insert(event_id);
            }
        }

        if !args.follow {
            break;
        }
        tokio::time::sleep(Duration::from_secs(2)).await;
    }

    Ok(())
}

fn now_epoch_millis() -> i64 {
    let millis: i128 = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_millis().cast_signed());

    millis.try_into().unwrap_or(i64::MAX)
}
