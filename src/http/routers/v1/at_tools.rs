use std::collections::HashMap;
use std::sync::Arc;
use axum::Extension;
use axum::http::{Response, StatusCode};
use hyper::Body;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::sync::RwLock as ARwLock;

use crate::call_validation::ChatToolCall;
use crate::tools::tools_description::{tool_description_list_from_yaml, tools_merged_and_filtered};
use crate::custom_error::ScratchError;
use crate::global_context::GlobalContext;

#[derive(Serialize, Deserialize, Clone)]
struct ToolsPermissionCheckPost {
    pub tool_calls: Vec<ChatToolCall>,
}


pub async fn handle_v1_tools(
    Extension(gcx): Extension<Arc<ARwLock<GlobalContext>>>,
    _: hyper::body::Bytes,
)  -> axum::response::Result<Response<Body>, ScratchError> {
    let turned_on = crate::tools::tools_description::tools_merged_and_filtered(gcx.clone()).await.keys().cloned().collect::<Vec<_>>();
    let allow_experimental = gcx.read().await.cmdline.experimental;

    let tool_desclist = tool_description_list_from_yaml(&turned_on, allow_experimental).unwrap_or_else(|e|{
        tracing::error!("Error loading compiled_in_tools: {:?}", e);
        vec![]
    });

    let tools_openai_stype = tool_desclist.into_iter().map(|x|x.into_openai_style()).collect::<Vec<_>>();

    let body = serde_json::to_string_pretty(&tools_openai_stype).map_err(|e| ScratchError::new(StatusCode::UNPROCESSABLE_ENTITY, format!("JSON problem: {}", e)))?;
    Ok(Response::builder()
        .status(StatusCode::OK)
        .header("Content-Type", "application/json")
        .body(Body::from(body))
        .unwrap()
    )
}

pub async fn handle_v1_tools_permission_check(
    Extension(gcx): Extension<Arc<ARwLock<GlobalContext>>>,
    body_bytes: hyper::body::Bytes,
) -> Result<Response<Body>, ScratchError> {
    let post = serde_json::from_slice::<ToolsPermissionCheckPost>(&body_bytes)
        .map_err(|e| ScratchError::new(StatusCode::UNPROCESSABLE_ENTITY, format!("JSON problem: {}", e)))?;

    let all_tools = tools_merged_and_filtered(gcx.clone()).await;

    let mut result_messages = vec![];
    for tool_call in &post.tool_calls {
        let tool = match all_tools.get(&tool_call.function.name) {
            Some(x) => x,
            None => {
                return Err(ScratchError::new(StatusCode::UNPROCESSABLE_ENTITY, format!("Unknown tool: {}", tool_call.function.name)))
            }
        };

        let args = match serde_json::from_str::<HashMap<String, Value>>(&tool_call.function.arguments) {
            Ok(args) => args,
            Err(e) => {
                return Err(ScratchError::new(StatusCode::UNPROCESSABLE_ENTITY, format!("JSON problem: {}", e)));
            }
        };

        let tool_locked = tool.lock().await;
        match tool_locked.check_if_denied(&args) {
            Ok((is_denied, message)) => {
                if is_denied {
                    result_messages.push(message);
                }
            }
            Err(e) => {
                return Err(ScratchError::new(StatusCode::UNPROCESSABLE_ENTITY, format!("Error checking if tool {} is allowed: {}", tool_call.function.name, e)));
            }
        }

        match tool_locked.check_for_confirmation_needed(&args) {
            Ok((is_confirmation_needed, message)) => {
                if is_confirmation_needed {
                    result_messages.push(message);
                }
            }
            Err(e) => {
                return Err(ScratchError::new(StatusCode::UNPROCESSABLE_ENTITY, format!("Error checking if tool {} needs confirmation: {}", tool_call.function.name, e)));
            }
        }
    }

    let body = serde_json::json!({
        "pause": !result_messages.is_empty(),
        "pause_reasons": result_messages,
    }).to_string();
    
    Ok(Response::builder()
        .status(StatusCode::OK)
        .header("Content-Type", "application/json")
        .body(Body::from(body))
        .unwrap()
    )
}