//! HTTP-01 challenge response for ingress.

use crate::state::DataPlaneState;
use crate::storage::keys;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::response::Response;
use bytes::Bytes;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use tracing::info;

#[must_use]
pub fn challenge_handler(
    State(state): State<Arc<DataPlaneState>>,
    Path(token): Path<String>,
) -> Pin<Box<dyn Future<Output = Response> + Send>> {
    Box::pin(async move {
        match state
            .storage
            .get_object(&keys::acme_challenge_key(&token))
            .await
        {
            Ok(Some(body)) => {
                info!(token = %token, "ingress: ACME HTTP-01 challenge");
                (
                    StatusCode::OK,
                    [("content-type", "application/octet-stream")],
                    Bytes::from(body),
                )
                    .into_response()
            }
            _ => (StatusCode::NOT_FOUND, ()).into_response(),
        }
    })
}
