use std::collections::HashMap;
use std::sync::Arc;
use axum::Extension;
use axum::http::{Response, StatusCode};
use hyper::Body;
use indexmap::IndexMap;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::sync::{Mutex as AMutex, RwLock as ARwLock};

use crate::at_commands::at_commands::AtCommandsContext;
use crate::cached_tokenizers;
use crate::call_validation::{ChatMessage, ChatToolCall, PostprocessSettings, SubchatParameters};
use crate::http::routers::v1::chat::CHAT_TOP_N;
use crate::tools::tools_description::{commands_require_confirmation_rules_from_integrations_yaml, tool_description_list_from_yaml, tools_merged_and_filtered};
use crate::custom_error::ScratchError;
use crate::global_context::{try_load_caps_quickly_if_not_present, GlobalContext};
use crate::tools::tools_execute::{command_should_be_confirmed_by_user, command_should_be_denied, run_tools};


#[derive(Serialize, Deserialize, Clone)]
struct ToolsPermissionCheckPost {
    pub tool_calls: Vec<ChatToolCall>,
}

#[derive(Serialize)]
#[serde(rename_all = "lowercase")]
enum PauseReasonType { 
    Confirmation,
    Denial,
}

#[derive(Serialize)]
struct PauseReason {
    #[serde(rename = "type")]
    reason_type: PauseReasonType,
    command: String,
    rule: String,
    tool_call_id: String,
}

#[derive(Serialize, Deserialize, Clone)]
pub struct ToolsExecutePost {
    pub context_messages: Vec<ChatMessage>,
    pub messages: Vec<ChatMessage>,
    pub n_ctx: usize,
    pub maxgen: usize,
    pub subchat_tool_parameters: IndexMap<String, SubchatParameters>, // tool_name: {model, allowed_context, temperature}
    pub postprocess_parameters: PostprocessSettings,
    pub model_name: String,
    pub chat_id: String,
    pub style: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ToolExecuteResponse {
    pub messages: Vec<ChatMessage>,
    pub tools_runned: bool,
}

pub async fn handle_v1_tools(
    Extension(gcx): Extension<Arc<ARwLock<GlobalContext>>>,
    _: hyper::body::Bytes,
) -> axum::response::Result<Response<Body>, ScratchError> {
    let all_tools = match tools_merged_and_filtered(gcx.clone()).await {
        Ok(tools) => tools,
        Err(e) => {
            let error_body = serde_json::json!({ "detail": e }).to_string();
            return Ok(Response::builder()
                .status(StatusCode::INTERNAL_SERVER_ERROR)
                .header("Content-Type", "application/json")
                .body(Body::from(error_body))
                .unwrap());
        }
    };

    let turned_on = all_tools.keys().cloned().collect::<Vec<_>>();
    let allow_experimental = gcx.read().await.cmdline.experimental;

    let tool_desclist = tool_description_list_from_yaml(all_tools, &turned_on, allow_experimental).await.unwrap_or_else(|e| {
        tracing::error!("Error loading compiled_in_tools: {:?}", e);
        vec![]
    });

    let tools_openai_stype = tool_desclist.into_iter().map(|x| x.into_openai_style()).collect::<Vec<_>>();

    let body = serde_json::to_string_pretty(&tools_openai_stype).map_err(|e| ScratchError::new(StatusCode::UNPROCESSABLE_ENTITY, format!("JSON problem: {}", e)))?;
    Ok(Response::builder()
        .status(StatusCode::OK)
        .header("Content-Type", "application/json")
        .body(Body::from(body))
        .unwrap())
}

pub async fn handle_v1_tools_check_if_confirmation_needed(
    Extension(gcx): Extension<Arc<ARwLock<GlobalContext>>>,
    body_bytes: hyper::body::Bytes,
) -> Result<Response<Body>, ScratchError> {
    let post = serde_json::from_slice::<ToolsPermissionCheckPost>(&body_bytes)
        .map_err(|e| ScratchError::new(StatusCode::UNPROCESSABLE_ENTITY, format!("JSON problem: {}", e)))?;

    let all_tools = match tools_merged_and_filtered(gcx.clone()).await {
        Ok(tools) => tools,
        Err(e) => {
            let error_body = serde_json::json!({ "detail": e }).to_string();
            return Ok(Response::builder()
                .status(StatusCode::INTERNAL_SERVER_ERROR)
                .header("Content-Type", "application/json")
                .body(Body::from(error_body))
                .unwrap());
        }
    };

    let mut result_messages = vec![];
    let mut confirmation_rules = None;
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

        let command_to_match = {
            let tool_locked = tool.lock().await;
            tool_locked.command_to_match_against_confirm_deny(&args)
        }.map_err(|e| {
            ScratchError::new(StatusCode::UNPROCESSABLE_ENTITY, format!("Error getting tool command to match: {}", e))
        })?;

        if !command_to_match.is_empty() {
            if confirmation_rules.is_none() {
                confirmation_rules = Some(commands_require_confirmation_rules_from_integrations_yaml(gcx.clone()).await.map_err(|e| {
                    ScratchError::new(StatusCode::INTERNAL_SERVER_ERROR, format!("Error loading generic tool config: {}", e))
                })?);
            }

            if let Some(rules) = &confirmation_rules {
                let (is_denied, deny_rule) = command_should_be_denied(&command_to_match, &rules.commands_deny);
                if is_denied {
                    result_messages.push(PauseReason {
                        reason_type: PauseReasonType::Denial,
                        command: command_to_match.clone(),
                        rule: deny_rule.clone(),
                        tool_call_id: tool_call.id.clone(),
                    });
                    continue;
                }
                let (needs_confirmation, confirmation_rule) = command_should_be_confirmed_by_user(&command_to_match, &rules.commands_need_confirmation);
                if needs_confirmation {
                    result_messages.push(PauseReason {
                        reason_type: PauseReasonType::Confirmation,
                        command: command_to_match.clone(),
                        rule: confirmation_rule.clone(),
                        tool_call_id: tool_call.id.clone(),
                    });
                    continue;
                }
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
        .unwrap())
}

pub async fn handle_v1_tools_execute(
    Extension(gcx): Extension<Arc<ARwLock<GlobalContext>>>,
    body_bytes: hyper::body::Bytes,
) -> Result<Response<Body>, ScratchError> {
    let tools_execute_post = serde_json::from_slice::<ToolsExecutePost>(&body_bytes)
      .map_err(|e| ScratchError::new(StatusCode::UNPROCESSABLE_ENTITY, format!("JSON problem: {}", e)))?;

    let caps = try_load_caps_quickly_if_not_present(gcx.clone(), 0).await?;
    let tokenizer = cached_tokenizers::cached_tokenizer(caps, gcx.clone(), tools_execute_post.model_name.clone()).await
        .map_err(|e| ScratchError::new(StatusCode::INTERNAL_SERVER_ERROR, format!("Error loading tokenizer: {}", e)))?;

    let mut ccx = AtCommandsContext::new(
        gcx.clone(),
        tools_execute_post.n_ctx,
        CHAT_TOP_N,
        false,
        tools_execute_post.messages.clone(),
        tools_execute_post.chat_id.clone(),
    ).await;
    ccx.subchat_tool_parameters = tools_execute_post.subchat_tool_parameters.clone();
    ccx.postprocess_parameters = tools_execute_post.postprocess_parameters.clone();
    let ccx_arc = Arc::new(AMutex::new(ccx));

    let (messages, tools_runned) = run_tools(
        ccx_arc.clone(), tokenizer.clone(), tools_execute_post.maxgen, &tools_execute_post.messages, &mut HasRagResults::new()
    ).await;

    let body = serde_json::json!({
        "messages": messages,
        "tools_runned": tools_runned,
    }).to_string();

    Ok(Response::builder()
        .status(StatusCode::OK)
        .header("Content-Type", "application/json")
        .body(Body::from(body))
        .unwrap()
    )
}

pub async fn handle_v1_tools_execute(
    Extension(gcx): Extension<Arc<ARwLock<GlobalContext>>>,
    body_bytes: hyper::body::Bytes,
) -> Result<Response<Body>, ScratchError> {
    let tools_execute_post = serde_json::from_slice::<ToolsExecutePost>(&body_bytes)
      .map_err(|e| ScratchError::new(StatusCode::UNPROCESSABLE_ENTITY, format!("JSON problem: {}", e)))?;

    let caps = try_load_caps_quickly_if_not_present(gcx.clone(), 0).await?;
    let tokenizer = cached_tokenizers::cached_tokenizer(caps, gcx.clone(), tools_execute_post.model_name.clone()).await
        .map_err(|e| ScratchError::new(StatusCode::INTERNAL_SERVER_ERROR, format!("Error loading tokenizer: {}", e)))?;

    let mut ccx = AtCommandsContext::new(
        gcx.clone(),
        tools_execute_post.n_ctx,
        CHAT_TOP_N,
        false,
        tools_execute_post.context_messages.clone(),
        tools_execute_post.chat_id.clone(),
    ).await;
    ccx.subchat_tool_parameters = tools_execute_post.subchat_tool_parameters.clone();
    ccx.postprocess_parameters = tools_execute_post.postprocess_parameters.clone();
    let ccx_arc = Arc::new(AMutex::new(ccx));

    let (messages, tools_runned) = run_tools(
        ccx_arc.clone(), tokenizer.clone(), tools_execute_post.maxgen, &tools_execute_post.messages, &tools_execute_post.style
    ).await;

    let response = ToolExecuteResponse {
        messages,
        tools_runned,
    };

    let response_json = serde_json::to_string(&response)
        .map_err(|e| ScratchError::new(StatusCode::UNPROCESSABLE_ENTITY, format!("JSON problem: {}", e)))?;

    Ok(Response::builder()
        .status(StatusCode::OK)
        .header("Content-Type", "application/json")
        .body(Body::from(response_json))
        .unwrap()
    )
}