use std::sync::Arc;
use axum::Extension;
use axum::http::{Response, StatusCode};
use hyper::Body;
use serde::Deserialize;
use serde_json::Value;
use tokio::sync::RwLock as ARwLock;
use crate::at_tools::subchat::execute_subchat;
use crate::call_validation::ChatMessage;
use crate::custom_error::ScratchError;
use crate::global_context::GlobalContext;

#[derive(Deserialize)]
struct SubChatPost {
    model_name: String,
    messages: Vec<ChatMessage>,
    depth: usize,
    #[serde(default)]
    tools: Option<Vec<Value>>,
    #[serde(default)]
    tool_choice: Option<String>,
    #[serde(default)]
    wrap_up_tokens_cnt: Option<usize>,
}

pub async fn handle_v1_subchat(
    Extension(global_context): Extension<Arc<ARwLock<GlobalContext>>>,
    body_bytes: hyper::body::Bytes,
) -> axum::response::Result<Response<Body>, ScratchError> {
    let post = serde_json::from_slice::<SubChatPost>(&body_bytes)
        .map_err(|e| ScratchError::new(StatusCode::UNPROCESSABLE_ENTITY, format!("JSON problem: {}", e)))?;
    
    let new_messages = execute_subchat(
        global_context.clone(), 
        post.model_name.as_str(),
        post.messages.clone(),
        post.depth,
        post.tools,
        post.tool_choice,
        post.wrap_up_tokens_cnt,
    ).await.map_err(|e| ScratchError::new(StatusCode::INTERNAL_SERVER_ERROR, format!("Error: {}", e)))?;
    
    let resp_serialised = serde_json::to_string_pretty(&new_messages).unwrap();
    Ok(
        Response::builder()
            .status(StatusCode::OK)
            .header("Content-Type", "application/json")
            .body(Body::from(resp_serialised))
            .unwrap()
    )
}
