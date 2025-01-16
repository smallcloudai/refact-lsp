use std::sync::Arc;
use axum::Extension;
use axum::http::{Response, StatusCode};
use hyper::Body;
use tokio::sync::RwLock as ARwLock;
use crate::custom_error::ScratchError;
use crate::global_context::GlobalContext;
use crate::privacy::{FilePrivacySettings, set_privacy_rules};

pub async fn handle_v1_privacy_get(
    Extension(gcx): Extension<Arc<ARwLock<GlobalContext>>>,
    _: hyper::body::Bytes,
) -> axum::response::Result<Response<Body>, ScratchError> {
    let privacy_rules = crate::privacy::load_privacy_rules_if_needed(gcx.clone()).await;
    let payload = serde_json::to_string_pretty(&privacy_rules).map_err(|e| {
        ScratchError::new(StatusCode::INTERNAL_SERVER_ERROR, format!("Failed to serialize payload: {}", e))
    })?;
    Ok(Response::builder()
        .status(StatusCode::OK)
        .header("Content-Type", "application/json")
        .body(Body::from(payload))
        .unwrap())
}

pub async fn handle_v1_privacy_set(
    Extension(gcx): Extension<Arc<ARwLock<GlobalContext>>>,
    body_bytes: hyper::body::Bytes,
) -> axum::response::Result<Response<Body>, ScratchError> {
    let post = serde_json::from_slice::<FilePrivacySettings>(&body_bytes)
        .map_err(|e| ScratchError::new(StatusCode::UNPROCESSABLE_ENTITY, format!("JSON problem: {}", e)))?;
    set_privacy_rules(gcx.clone(), post).await;
    Ok(Response::builder()
       .status(StatusCode::OK)
       .header("Content-Type", "application/json")
       .body(Body::from(format!("")))
       .unwrap())
}
