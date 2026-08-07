//! Nitrum hello sample — exercises crypto API, egress, and KV via `sdk`.

use axum::extract::{Query, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::routing::{get, post};
use axum::{Json, Router};
use opentelemetry::KeyValue;
use opentelemetry::metrics::{Counter, Histogram};
use opentelemetry_otlp::WithExportConfig;
use opentelemetry_sdk::Resource;
use opentelemetry_sdk::metrics::SdkMeterProvider;
use sdk::{NitrumClient, SdkError};
use serde::Deserialize;
use serde_json::{Value, json};
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Instant;
use tracing::{error, info};

const SERVICE_NAME: &str = "nitrum-hello";

struct AppState {
    /// Data-plane crypto API client.
    nitrum: NitrumClient,
    /// Outbound HTTP client for egress demos.
    http: reqwest::Client,
    /// Encrypt/decrypt round-trip counter.
    crypto_ops: Counter<u64>,
    /// KV latency histogram (ms).
    kv_latency: Histogram<f64>,
}

enum AppError {
    BadGateway(&'static str),
}

impl From<SdkError> for AppError {
    fn from(err: SdkError) -> Self {
        error!(error = %err, "sdk error");
        Self::BadGateway("upstream request failed")
    }
}

impl From<reqwest::Error> for AppError {
    fn from(err: reqwest::Error) -> Self {
        error!(error = %err, "egress error");
        Self::BadGateway("egress request failed")
    }
}

impl IntoResponse for AppError {
    fn into_response(self) -> axum::response::Response {
        let (status, message) = match self {
            Self::BadGateway(message) => (StatusCode::BAD_GATEWAY, message),
        };
        (status, Json(json!({ "error": message }))).into_response()
    }
}

#[tokio::main]
async fn main() {
    init_telemetry(SERVICE_NAME);

    let meter = opentelemetry::global::meter(SERVICE_NAME);
    let state = Arc::new(AppState {
        nitrum: NitrumClient::default(),
        http: reqwest::Client::new(),
        crypto_ops: meter
            .u64_counter("app.crypto.ops")
            .with_description("Encrypt/decrypt round-trips handled by the sample app")
            .build(),
        kv_latency: meter
            .f64_histogram("app.kv.duration.ms")
            .with_description("KV set+get round-trip latency in the sample app")
            .with_unit("ms")
            .build(),
    });

    let app = Router::new()
        .route("/health", get(health))
        .route("/egress", get(egress))
        .route("/egress-blocked", get(egress_blocked))
        .route("/attestation", get(attestation))
        .route("/crypto", post(crypto))
        .route("/random", post(random))
        .route("/kv", post(kv))
        .route("/env", get(env))
        .with_state(state);

    let addr = listen_addr();
    info!(%addr, "hello sample listening");
    let listener = tokio::net::TcpListener::bind(addr).await.expect("bind");
    axum::serve(listener, app).await.expect("serve");
}

fn init_telemetry(default_service_name: &str) {
    let _ = tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .try_init();

    let endpoint = std::env::var("OTEL_EXPORTER_OTLP_ENDPOINT").ok();
    let Some(endpoint) = endpoint.filter(|e| !e.is_empty()) else {
        return;
    };

    let resource = Resource::builder()
        .with_service_name(
            std::env::var("OTEL_SERVICE_NAME").unwrap_or_else(|_| default_service_name.into()),
        )
        .build();

    if let Ok(exporter) = opentelemetry_otlp::MetricExporter::builder()
        .with_tonic()
        .with_endpoint(endpoint)
        .build()
    {
        let provider = SdkMeterProvider::builder()
            .with_periodic_exporter(exporter)
            .with_resource(resource)
            .build();
        opentelemetry::global::set_meter_provider(provider);
    }
}

fn listen_addr() -> SocketAddr {
    let port: u16 = std::env::var("PORT")
        .ok()
        .and_then(|p| p.parse().ok())
        .unwrap_or(8080);
    SocketAddr::from(([0, 0, 0, 0], port))
}

async fn health() -> &'static str {
    "OK"
}

async fn egress(State(state): State<Arc<AppState>>) -> Result<Json<Value>, AppError> {
    let res = state
        .http
        .get("https://api.ipify.org?format=json")
        .timeout(std::time::Duration::from_secs(15))
        .send()
        .await?;
    let body = res.json::<Value>().await.map_err(|e| {
        error!(error = %e, "egress decode");
        AppError::BadGateway("egress decode failed")
    })?;
    Ok(Json(body))
}

/// Expected denial path: always 200 so clients can assert allowlist behavior.
async fn egress_blocked(State(state): State<Arc<AppState>>) -> Json<Value> {
    match state
        .http
        .get("https://example.com")
        .timeout(std::time::Duration::from_secs(5))
        .send()
        .await
    {
        Ok(_) => Json(json!({ "ok": true })),
        Err(e) => Json(json!({ "ok": false, "error": e.to_string() })),
    }
}

#[derive(Debug, Deserialize)]
struct AttestationQuery {
    nonce: Option<String>,
    #[serde(rename = "publicKey")]
    public_key: Option<String>,
    #[serde(rename = "userData")]
    user_data: Option<String>,
}

async fn attestation(
    State(state): State<Arc<AppState>>,
    Query(q): Query<AttestationQuery>,
) -> Result<Json<Value>, AppError> {
    let bytes = state
        .nitrum
        .attestation(
            q.nonce.as_deref(),
            q.public_key.as_deref(),
            q.user_data.as_deref(),
        )
        .await?;
    Ok(Json(
        json!({ "data": base64_encode(&bytes), "error": null }),
    ))
}

#[derive(Debug, Deserialize)]
struct CryptoBody {
    plaintext: Option<String>,
}

async fn crypto(
    State(state): State<Arc<AppState>>,
    Json(body): Json<CryptoBody>,
) -> Result<Json<Value>, AppError> {
    let plaintext = body.plaintext.unwrap_or_default();
    let encrypted = match state.nitrum.encrypt(&plaintext).await {
        Ok(v) => v,
        Err(e) => {
            state.crypto_ops.add(1, &[KeyValue::new("result", "error")]);
            return Err(e.into());
        }
    };
    let decrypted = match state.nitrum.decrypt(&encrypted).await {
        Ok(v) => v,
        Err(e) => {
            state.crypto_ops.add(1, &[KeyValue::new("result", "error")]);
            return Err(e.into());
        }
    };
    state.crypto_ops.add(1, &[KeyValue::new("result", "ok")]);
    Ok(Json(json!({
        "encrypted": { "data": encrypted, "error": null },
        "decrypted": { "data": decrypted, "error": null },
    })))
}

#[derive(Debug, Deserialize)]
struct RandomBody {
    length: Option<usize>,
}

async fn random(
    State(state): State<Arc<AppState>>,
    Json(body): Json<RandomBody>,
) -> Result<Json<Value>, AppError> {
    let length = body.length.unwrap_or(32);
    let bytes = state.nitrum.random(length).await?;
    Ok(Json(
        json!({ "data": base64_encode(&bytes), "error": null }),
    ))
}

#[derive(Debug, Deserialize)]
struct KvBody {
    key: Option<String>,
    value: Option<String>,
}

async fn kv(
    State(state): State<Arc<AppState>>,
    Json(body): Json<KvBody>,
) -> Result<Json<Value>, AppError> {
    let start = Instant::now();
    let key = body.key.unwrap_or_else(|| "hello/default".into());
    let value = body.value.unwrap_or_else(|| format!("kv-{}", now_ms()));

    let result = async {
        state.nitrum.kv_set(&key, &value).await?;
        let got = state.nitrum.kv_get(&key).await?;
        Ok::<_, SdkError>(got.unwrap_or_default())
    }
    .await;

    state.kv_latency.record(
        start.elapsed().as_secs_f64() * 1000.0,
        &[KeyValue::new("route", "/kv")],
    );

    let got = result?;
    Ok(Json(json!({ "key": key, "value": got })))
}

async fn env() -> Json<Value> {
    Json(json!({ "env": std::env::var("DEMO").ok() }))
}

fn base64_encode(bytes: &[u8]) -> String {
    use base64::Engine;
    base64::engine::general_purpose::STANDARD.encode(bytes)
}

fn now_ms() -> u128 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0)
}
