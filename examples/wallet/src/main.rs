//! Wallet enclave sample — generate encrypted keys and sign EIP-1559 txs.

use alloy::consensus::{SignableTransaction, TxEip1559};
use alloy::eips::eip2718::Encodable2718;
use alloy::network::TxSignerSync;
use alloy::primitives::{Address, Bytes, TxKind, U256};
use alloy::signers::local::PrivateKeySigner;
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::routing::{get, post};
use axum::{Json, Router};
use hmac::{Hmac, Mac};
use opentelemetry::KeyValue;
use opentelemetry::metrics::{Counter, Histogram};
use opentelemetry_otlp::WithExportConfig;
use opentelemetry_sdk::Resource;
use opentelemetry_sdk::metrics::SdkMeterProvider;
use sdk::{NitrumClient, SdkError};
use serde::Deserialize;
use serde_json::{Value, json};
use sha2::Sha256;
use std::net::SocketAddr;
use std::str::FromStr;
use std::sync::Arc;
use std::time::Instant;
use tracing::{error, info};

const SERVICE_NAME: &str = "blockchain-wallet";

type HmacSha256 = Hmac<Sha256>;

struct AppState {
    /// Data-plane crypto API client.
    nitrum: NitrumClient,
    /// Wallets created counter.
    wallets_created: Counter<u64>,
    /// Sign latency histogram (ms).
    sign_latency: Histogram<f64>,
    /// Sign error counter.
    sign_errors: Counter<u64>,
}

enum AppError {
    BadRequest(&'static str),
    Internal(&'static str),
}

impl From<SdkError> for AppError {
    fn from(err: SdkError) -> Self {
        error!(error = %err, "sdk error");
        Self::Internal("failed to sign transaction")
    }
}

impl IntoResponse for AppError {
    fn into_response(self) -> axum::response::Response {
        let (status, message) = match self {
            Self::BadRequest(message) => (StatusCode::BAD_REQUEST, message),
            Self::Internal(message) => (StatusCode::INTERNAL_SERVER_ERROR, message),
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
        wallets_created: meter
            .u64_counter("app.wallet.created")
            .with_description("Wallets created by the sample app")
            .build(),
        sign_latency: meter
            .f64_histogram("app.wallet.sign.duration.ms")
            .with_description("Wallet sign request latency in the sample app")
            .with_unit("ms")
            .build(),
        sign_errors: meter
            .u64_counter("app.wallet.sign.errors")
            .with_description("Failed wallet sign requests in the sample app")
            .build(),
    });

    let app = Router::new()
        .route("/health", get(health))
        .route("/wallet", post(create_wallet))
        .route("/wallet/sign", post(sign_wallet))
        .with_state(state);

    let addr = listen_addr();
    info!(%addr, "blockchain-wallet listening");
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

#[derive(Debug, Deserialize)]
struct CreateWalletBody {
    secret: String,
}

async fn create_wallet(
    State(state): State<Arc<AppState>>,
    Json(body): Json<CreateWalletBody>,
) -> Result<Json<Value>, AppError> {
    if body.secret.is_empty() {
        return Err(AppError::BadRequest("secret is required"));
    }

    let ciphertext = create_wallet_inner(&state, &body.secret)
        .await
        .map_err(|e| {
            error!(error = %e, "failed to generate key");
            AppError::Internal("failed to generate key")
        })?;
    state.wallets_created.add(1, &[]);
    Ok(Json(json!({ "ciphertext": ciphertext })))
}

async fn create_wallet_inner(state: &AppState, secret: &str) -> Result<String, SdkError> {
    let private_key = state.nitrum.random(32).await?;
    let payload = json!({
        "privateKey": hex::encode(private_key),
        "secret": secret,
    });
    let ciphertext = state.nitrum.encrypt(&payload.to_string()).await?;
    state
        .nitrum
        .kv_set("wallet:demo_last_ciphertext", &ciphertext)
        .await?;
    Ok(ciphertext)
}

#[derive(Debug, Deserialize)]
struct SignBody {
    ciphertext: String,
    #[serde(rename = "txData")]
    tx_data: TxData,
    proof: String,
}

#[derive(Debug, Deserialize, serde::Serialize)]
struct TxData {
    to: String,
    nonce: serde_json::Value,
    #[serde(rename = "chainId")]
    chain_id: serde_json::Value,
    #[serde(rename = "gasLimit")]
    gas_limit: serde_json::Value,
    #[serde(rename = "maxFeePerGas")]
    max_fee_per_gas: serde_json::Value,
    #[serde(rename = "maxPriorityFeePerGas")]
    max_priority_fee_per_gas: serde_json::Value,
    value: serde_json::Value,
}

async fn sign_wallet(
    State(state): State<Arc<AppState>>,
    Json(body): Json<SignBody>,
) -> Result<Json<Value>, AppError> {
    let start = Instant::now();
    let result = sign_wallet_inner(&state, body).await;
    state.sign_latency.record(
        start.elapsed().as_secs_f64() * 1000.0,
        &[KeyValue::new("route", "/wallet/sign")],
    );
    match result {
        Ok(signature) => Ok(Json(json!({ "signature": signature }))),
        Err(e) => {
            state
                .sign_errors
                .add(1, &[KeyValue::new("reason", "failed")]);
            Err(e)
        }
    }
}

async fn sign_wallet_inner(state: &AppState, body: SignBody) -> Result<String, AppError> {
    let decrypted = state.nitrum.decrypt(&body.ciphertext).await?;
    let parsed: serde_json::Value = serde_json::from_str(&decrypted).map_err(|e| {
        error!(error = %e, "invalid wallet payload");
        AppError::Internal("failed to sign transaction")
    })?;
    let secret = parsed
        .get("secret")
        .and_then(|v| v.as_str())
        .ok_or_else(|| {
            error!("missing secret in wallet payload");
            AppError::Internal("failed to sign transaction")
        })?;
    let private_key_hex = parsed
        .get("privateKey")
        .and_then(|v| v.as_str())
        .ok_or_else(|| {
            error!("missing privateKey in wallet payload");
            AppError::Internal("failed to sign transaction")
        })?;

    let js_like = js_like_tx_json(&body.tx_data)?;
    let expected = hmac_hex(secret.as_bytes(), js_like.as_bytes())?;
    if body.proof != expected {
        error!("HMAC verification failed");
        return Err(AppError::Internal("failed to sign transaction"));
    }

    let signer: PrivateKeySigner = private_key_hex.parse().map_err(|e| {
        error!(error = %e, "invalid private key");
        AppError::Internal("failed to sign transaction")
    })?;

    let to = Address::from_str(&body.tx_data.to).map_err(|e| {
        error!(error = %e, "invalid to");
        AppError::Internal("failed to sign transaction")
    })?;
    let mut tx = TxEip1559 {
        chain_id: parse_u64(&body.tx_data.chain_id)?,
        nonce: parse_u64(&body.tx_data.nonce)?,
        gas_limit: parse_u64(&body.tx_data.gas_limit)?,
        max_fee_per_gas: parse_u128(&body.tx_data.max_fee_per_gas)?,
        max_priority_fee_per_gas: parse_u128(&body.tx_data.max_priority_fee_per_gas)?,
        to: TxKind::Call(to),
        value: parse_u256(&body.tx_data.value)?,
        access_list: Default::default(),
        input: Bytes::new(),
    };

    let signature = signer.sign_transaction_sync(&mut tx).map_err(|e| {
        error!(error = %e, "sign_transaction failed");
        AppError::Internal("failed to sign transaction")
    })?;
    let encoded = tx.into_signed(signature).encoded_2718();
    Ok(format!("0x{}", hex::encode(encoded)))
}

fn hmac_hex(key: &[u8], data: &[u8]) -> Result<String, AppError> {
    let mut mac = HmacSha256::new_from_slice(key).map_err(|e| {
        error!(error = %e, "hmac key");
        AppError::Internal("failed to sign transaction")
    })?;
    mac.update(data);
    Ok(hex::encode(mac.finalize().into_bytes()))
}

fn js_like_tx_json(tx: &TxData) -> Result<String, AppError> {
    Ok(format!(
        "{{\"to\":\"{}\",\"nonce\":{},\"gasLimit\":{},\"maxFeePerGas\":{},\"maxPriorityFeePerGas\":{},\"chainId\":{},\"value\":{}}}",
        tx.to,
        json_number_str(&tx.nonce)?,
        json_number_str(&tx.gas_limit)?,
        json_number_str(&tx.max_fee_per_gas)?,
        json_number_str(&tx.max_priority_fee_per_gas)?,
        json_number_str(&tx.chain_id)?,
        json_number_str(&tx.value)?,
    ))
}

fn json_number_str(v: &serde_json::Value) -> Result<String, AppError> {
    match v {
        serde_json::Value::Number(n) => Ok(n.to_string()),
        serde_json::Value::String(s) => Ok(s.clone()),
        _ => {
            error!("expected number in tx field");
            Err(AppError::Internal("failed to sign transaction"))
        }
    }
}

fn parse_u64(v: &serde_json::Value) -> Result<u64, AppError> {
    json_number_str(v)?.parse().map_err(|e| {
        error!(error = %e, "u64 parse");
        AppError::Internal("failed to sign transaction")
    })
}

fn parse_u128(v: &serde_json::Value) -> Result<u128, AppError> {
    json_number_str(v)?.parse().map_err(|e| {
        error!(error = %e, "u128 parse");
        AppError::Internal("failed to sign transaction")
    })
}

fn parse_u256(v: &serde_json::Value) -> Result<U256, AppError> {
    U256::from_str(&json_number_str(v)?).map_err(|e| {
        error!(error = %e, "U256 parse");
        AppError::Internal("failed to sign transaction")
    })
}
