//! Nitrum hello sample — exercises crypto API and egress via `sdk`.

use axum::extract::{Query, State};
use axum::http::StatusCode;
use axum::routing::{get, post};
use axum::{Json, Router};
use opentelemetry::KeyValue;
use opentelemetry::metrics::Counter;
use opentelemetry_otlp::WithExportConfig;
use opentelemetry_sdk::Resource;
use opentelemetry_sdk::metrics::SdkMeterProvider;
use sdk::NitrumClient;
use serde::Deserialize;
use serde_json::{Value, json};
use std::net::SocketAddr;
use std::sync::Arc;
use tracing::{error, info};

struct AppState {
    /// Data-plane crypto API client.
    nitrum: NitrumClient,
    /// Outbound HTTP client for egress demos.
    http: reqwest::Client,
    /// Encrypt/decrypt round-trip counter.
    crypto_ops: Counter<u64>,
}

#[tokio::main]
async fn main() {
    // Initialize tracing.
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();

    // Initialize telemetry.
    if let Ok(endpoint) = std::env::var("OTEL_EXPORTER_OTLP_ENDPOINT")
        && let Ok(exporter) = opentelemetry_otlp::MetricExporter::builder()
            .with_tonic()
            .with_endpoint(endpoint)
            .build()
    {
        opentelemetry::global::set_meter_provider(
            SdkMeterProvider::builder()
                .with_periodic_exporter(exporter)
                .with_resource(
                    Resource::builder()
                        .with_service_name("nitrum-hello-world")
                        .build(),
                )
                .build(),
        );
    }
    // Create a meter for the application.
    let meter = opentelemetry::global::meter("nitrum-hello-world");
    
    // Create a state for the application.
    let state = Arc::new(AppState {
        nitrum: NitrumClient::default(),
        http: reqwest::Client::new(),
        crypto_ops: meter
            .u64_counter("app.crypto.ops")
            .with_description("Encrypt/decrypt round-trips handled by the sample app")
            .build(),
    });

    // Create a router for the application.
    let app = Router::new()
        .route("/health", get(health_handler))
        .route("/egress", get(egress_handler))
        .route("/attestation", get(attestation_handler))
        .route("/crypto", post(crypto_handler))
        .route("/random", post(random_handler))
        .route("/env", get(env_handler))
        .with_state(state);

    // Get the port from the environment or use 8080 as default.
    let port: u16 = std::env::var("PORT")
        .ok()
        .and_then(|p| p.parse().ok())
        .unwrap_or(8080);
    let addr = SocketAddr::from(([0, 0, 0, 0], port));

    // Start the server.
    info!("listening on {}", addr);
    let listener = tokio::net::TcpListener::bind(addr).await.unwrap();
    axum::serve(listener, app).await.unwrap();
}

// ############################################################################
// Health handler.
// ############################################################################

async fn health_handler() -> &'static str {
    "OK"
}

// ############################################################################ 
// Egress handler.
// ############################################################################

#[derive(Debug, Deserialize)]
struct EgressQuery {
    url: String,
}

/// `GET /egress?url=…` — fetch a URL; returns `{ ok, status?, error? }`.
async fn egress_handler(State(state): State<Arc<AppState>>, Query(q): Query<EgressQuery>) -> Json<Value> {
    match state
        .http
        .get(&q.url)
        .timeout(std::time::Duration::from_secs(5))
        .send()
        .await
    {
        Ok(res) => Json(json!({ "ok": true, "status": res.status().as_u16() })),
        Err(e) => Json(json!({ "ok": false, "error": e.to_string() })),
    }
}

// ############################################################################
// Attestation query handler.
// ############################################################################

#[derive(Debug, Deserialize)]
struct AttestationQuery {
    nonce: Option<String>,
    #[serde(rename = "publicKey")]
    public_key: Option<String>,
    #[serde(rename = "userData")]
    user_data: Option<String>,
}

async fn attestation_handler(
    State(state): State<Arc<AppState>>,
    Query(q): Query<AttestationQuery>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let data = state
        .nitrum
        .attestation(
            q.nonce.as_deref(),
            q.public_key.as_deref(),
            q.user_data.as_deref(),
        )
        .await
        .map_err(|e| {
            error!(error = %e, "sdk error");
            (StatusCode::BAD_GATEWAY, Json(json!({ "error": "upstream request failed" })))
        })?;

    Ok(Json(json!({ "data": data, "error": null })))
}

// ############################################################################
// Crypto handler.
// ############################################################################

#[derive(Debug, Deserialize)]
struct CryptoBody {
    plaintext: Option<String>,
}

async fn crypto_handler(
    State(state): State<Arc<AppState>>,
    Json(body): Json<CryptoBody>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let plaintext = body.plaintext.unwrap_or_default();
    
    let encrypted = match state.nitrum.encrypt(&plaintext).await {
        Ok(v) => v,
        Err(e) => {
            error!(error = %e, "sdk error");
            state.crypto_ops.add(1, &[KeyValue::new("result", "error")]);
            return Err((
                StatusCode::BAD_GATEWAY,
                Json(json!({ "error": "upstream request failed" })),
            ));
        }
    };
    let decrypted = match state.nitrum.decrypt(&encrypted).await {
        Ok(v) => v,
        Err(e) => {
            error!(error = %e, "sdk error");
            state.crypto_ops.add(1, &[KeyValue::new("result", "error")]);
            return Err((
                StatusCode::BAD_GATEWAY,
                Json(json!({ "error": "upstream request failed" })),
            ));
        }
    };
    state.crypto_ops.add(1, &[KeyValue::new("result", "ok")]);
    
    Ok(Json(json!({
        "encrypted": { "data": encrypted, "error": null },
        "decrypted": { "data": decrypted, "error": null },
    })))
}

// ############################################################################
// Random handler.
// ############################################################################

#[derive(Debug, Deserialize)]
struct RandomBody {
    length: Option<usize>,
}

async fn random_handler(
    State(state): State<Arc<AppState>>,
    Json(body): Json<RandomBody>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let length = body.length.unwrap_or(32);

    let bytes = state.nitrum.random(length).await.map_err(|e| {
        error!(error = %e, "sdk error");
        (StatusCode::BAD_GATEWAY, Json(json!({ "error": "upstream request failed" })))
    })?;

    Ok(Json(
        json!({ "data": base64_encode(&bytes), "error": null }),
    ))
}

// ############################################################################
// Environment handler.
// ############################################################################

async fn env_handler() -> Json<Value> {
    Json(json!({ "env": std::env::var("DEMO").ok() }))
}

// ############################################################################
// Utils
// ############################################################################

fn base64_encode(bytes: &[u8]) -> String {
    use base64::Engine;
    base64::engine::general_purpose::STANDARD.encode(bytes)
}
