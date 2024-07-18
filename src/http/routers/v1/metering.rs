use std::sync::Arc;
use axum::Extension;
use axum::http::{Response, StatusCode};
use hyper::Body;
use tokio::sync::RwLock as ARwLock;
use crate::custom_error::ScratchError;
use crate::global_context::GlobalContext;


pub async fn handle_v1_metering(
    Extension(global_context): Extension<Arc<ARwLock<GlobalContext>>>,
    _body_bytes: hyper::body::Bytes,
) -> axum::response::Result<Response<Body>, ScratchError> {
    let metering = global_context.read().await.metering.lock().await.iter().cloned().collect::<Vec<_>>();

    Ok(Response::builder()
        .status(StatusCode::OK)
        .header("Content-Type", "application/json")
        .body(Body::from(serde_json::to_string_pretty(&metering).unwrap()))
        .unwrap())
}
