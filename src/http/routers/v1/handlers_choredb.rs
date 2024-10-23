use std::sync::Arc;
use tokio::sync::RwLock as ARwLock;
use serde_json::json;
use indexmap::IndexMap;
use axum::Extension;
use axum::response::Result;
use axum::extract::Query;
use hyper::{Body, Response, StatusCode};
use serde::Deserialize;

use crate::custom_error::ScratchError;
use crate::global_context::GlobalContext;
use crate::chore_db::{chat_message_get, chat_message_set};
use crate::call_validation::ChatMessage;


#[derive(Deserialize)]
struct ChatMessageGetQuery {
    cthread_id: String,
    i: usize,
}

#[derive(Deserialize)]
struct ChatMessageSetRequest {
    cthread_id: String,
    i: usize,
    message: ChatMessage,
}

pub async fn handle_chat_message_get(
    Extension(gcx): Extension<Arc<ARwLock<GlobalContext>>>,
    query_bytes: hyper::body::Bytes,
) -> Result<Response<Body>, ScratchError> {
    let params: ChatMessageGetQuery = serde_urlencoded::from_bytes(&query_bytes)
        .map_err(|e| ScratchError::new(StatusCode::BAD_REQUEST, format!("Invalid query parameters: {}", e)))?;

    let cdb = gcx.read().await.chore_db.clone();

    let message = chat_message_get(cdb, params.cthread_id, params.i).await
        .ok_or_else(|| ScratchError::new(StatusCode::NOT_FOUND, "Chat message not found".to_string()))?;

    let response = Response::builder()
        .header("Content-Type", "application/json")
        .body(Body::from(serde_json::to_string(&message).unwrap()))
        .unwrap();

    Ok(response)
}

pub async fn handle_chat_message_set(
    Extension(gcx): Extension<Arc<ARwLock<GlobalContext>>>,
    body_bytes: hyper::body::Bytes,
) -> Result<Response<Body>, ScratchError> {
    let post: ChatMessageSetRequest = serde_json::from_slice(&body_bytes).map_err(|e| {
        tracing::info!("cannot parse input:\n{:?}", body_bytes);
        ScratchError::new(StatusCode::BAD_REQUEST, format!("JSON problem: {}", e))
    })?;

    let cdb = gcx.read().await.chore_db.clone();

    chat_message_set(cdb, post.cthread_id, post.i, post.message).await;

    let response = Response::builder()
        .status(StatusCode::OK)
        .header("Content-Type", "application/json")
        .body(Body::from(json!({"status": "success"}).to_string()))
        .unwrap();

    Ok(response)
}
