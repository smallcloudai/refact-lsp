use std::sync::Arc;
use tokio::sync::Mutex as AMutex;
use axum::Extension;
use axum::http::{Response, StatusCode};
use hyper::Body;
use serde::Deserialize;
use tokio::sync::RwLock as ARwLock;
use crate::subchat::subchat_single;
use crate::at_commands::at_commands::AtCommandsContext;
use crate::call_validation::{ChatContent, ChatMessage};
use crate::custom_error::ScratchError;
use crate::global_context::{try_load_caps_quickly_if_not_present, GlobalContext};


#[derive(Deserialize)]
struct CommitMessageFromDiffPost {
    diff: String
}

pub async fn handle_v1_commit_message_from_diff(
    Extension(global_context): Extension<Arc<ARwLock<GlobalContext>>>,
    body_bytes: hyper::body::Bytes,
) -> axum::response::Result<Response<Body>, ScratchError> {
    let post = serde_json::from_slice::<CommitMessageFromDiffPost>(&body_bytes)
        .map_err(|e| ScratchError::new(StatusCode::UNPROCESSABLE_ENTITY, format!("JSON problem: {}", e)))?;
    let messages = vec![ChatMessage {
        role: "system".to_string(),
        content: ChatContent::SimpleText(format!("Generate a short and descriptive commit message for the diff:\n{}", post.diff)),
        ..Default::default()
    }];
    let model_name = match try_load_caps_quickly_if_not_present(global_context.clone(), 0).await {
        Ok(caps) => {
            caps.read()
                .map(|x| Ok(x.code_chat_default_model.clone()))
                .map_err(|_|
                    ScratchError::new(StatusCode::INTERNAL_SERVER_ERROR, "Caps are not available".to_string())
                )?
        },
        Err(_) => Err(ScratchError::new(StatusCode::INTERNAL_SERVER_ERROR, "No caps available".to_string()))
    }?;

    let ccx: Arc<AMutex<AtCommandsContext>> = Arc::new(AMutex::new(
        AtCommandsContext::new(
            global_context.clone(), 
            32000, 
            1, 
            false, 
            messages.clone(), 
            "".to_string(), 
            false
        ).await)
    );

    let new_messages = subchat_single(
        ccx.clone(),
        model_name.as_str(),
        messages,
        vec![],
        None,
        false,
        Some(0.5),
        None,
        1,
        None,
        None,
        None,
    ).await.map_err(|e| ScratchError::new(StatusCode::INTERNAL_SERVER_ERROR, format!("Error: {}", e)))?;

    let commit_message = new_messages
        .into_iter()
        .next()
        .map(|x| x.into_iter().last().map(|last_m| {
            match last_m.content {
                ChatContent::SimpleText(text) => Some(text),
                ChatContent::Multimodal(_) => { None }
            }
        }))
        .flatten()
        .flatten()
        .ok_or(ScratchError::new(StatusCode::INTERNAL_SERVER_ERROR, "No commit message found".to_string()))?;
    Ok(
        Response::builder()
            .status(StatusCode::OK)
            .header("Content-Type", "application/json")
            .body(Body::from(commit_message))
            .unwrap()
    )
}
