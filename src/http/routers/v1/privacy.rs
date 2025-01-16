use std::sync::Arc;
use axum::Extension;
use axum::http::{Response, StatusCode};
use hyper::Body;
use serde::Deserialize;
use tokio::sync::RwLock as ARwLock;
use crate::custom_error::ScratchError;
use crate::global_context::GlobalContext;
use crate::privacy::{FilePrivacySettings, set_privacy_rules};

#[derive(Deserialize)]
struct PrivacyGet {
    pub global: bool,
}

pub async fn handle_v1_privacy_get(
    Extension(gcx): Extension<Arc<ARwLock<GlobalContext>>>,
    body_bytes: hyper::body::Bytes,
) -> axum::response::Result<Response<Body>, ScratchError> {
    let get = serde_json::from_slice::<PrivacyGet>(&body_bytes)
        .map_err(|e| ScratchError::new(StatusCode::UNPROCESSABLE_ENTITY, format!("JSON problem: {}", e)))?;
    let privacy_rules;
    if get.global {
        privacy_rules = crate::privacy::load_privacy_rules_if_needed(gcx.clone()).await;
    } else {
        // TODO: local privacy.yaml
        privacy_rules = Arc::new(FilePrivacySettings::default());
    }
    let payload = serde_json::to_string_pretty(&privacy_rules).map_err(|e| {
        ScratchError::new(StatusCode::INTERNAL_SERVER_ERROR, format!("Failed to serialize payload: {}", e))
    })?;
    Ok(Response::builder()
        .status(StatusCode::OK)
        .header("Content-Type", "application/json")
        .body(Body::from(payload))
        .unwrap())
}

#[derive(Deserialize)]
struct PrivacySet {
    pub privacy_rules: FilePrivacySettings,
    pub global: bool,
}

pub async fn handle_v1_privacy_set(
    Extension(gcx): Extension<Arc<ARwLock<GlobalContext>>>,
    body_bytes: hyper::body::Bytes,
) -> axum::response::Result<Response<Body>, ScratchError> {
    let post = serde_json::from_slice::<PrivacySet>(&body_bytes)
        .map_err(|e| ScratchError::new(StatusCode::UNPROCESSABLE_ENTITY, format!("JSON problem: {}", e)))?;
    if post.global {
        set_privacy_rules(gcx.clone(), post.privacy_rules).await;
    } else {
        // TODO: local privacy.yaml
    }
    Ok(Response::builder()
       .status(StatusCode::OK)
       .header("Content-Type", "application/json")
       .body(Body::from(format!("")))
       .unwrap())
}
