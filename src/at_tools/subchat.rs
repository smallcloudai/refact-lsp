use std::sync::Arc;
use std::collections::HashSet;
use reqwest::Client;
use tokio::sync::RwLock as ARwLock;
use serde_json::Value;
use tracing::{info, warn};

use crate::call_validation::{ChatMessage, ChatPost, ChatToolCall, ChatUsage, SamplingParameters};
use crate::global_context::{GlobalContext, try_load_caps_quickly_if_not_present};
use crate::http::routers::v1::chat::lookup_chat_scratchpad;
use crate::scratchpad_abstract::ScratchpadAbstract;


const TEMPERATURE: f32 = 0.2;
const MAX_NEW_TOKENS: usize = 4096;
const ALLOW_AT: bool = true;


async fn create_chat_post_and_scratchpad(
    global_context: Arc<ARwLock<GlobalContext>>,
    model_name: &str,
    messages: Vec<&ChatMessage>,
    temperature: Option<f32>,
    max_new_tokens: usize,
    tools: Option<Vec<Value>>,
    tool_choice: Option<String>,
    only_deterministic_messages: bool,
) -> Result<(ChatPost, Box<dyn ScratchpadAbstract>), String> {
    let caps = try_load_caps_quickly_if_not_present(
        global_context.clone(), 0,
    ).await.map_err(|e| {
        warn!("no caps: {:?}", e);
        "no caps".to_string()
    })?;

    let mut chat_post = ChatPost {
        messages: messages.iter().cloned().cloned().collect::<Vec<_>>(),
        parameters: SamplingParameters {
            max_new_tokens,
            temperature,
            top_p: None,
            stop: vec![],
        },
        model: model_name.to_string(),
        scratchpad: "".to_string(),
        stream: Some(false),
        temperature,
        max_tokens: 0,
        tools,
        tool_choice,
        only_deterministic_messages: only_deterministic_messages,
        chat_id: "".to_string(),
    };

    let (model_name, scratchpad_name, scratchpad_patch, n_ctx, supports_tools) = lookup_chat_scratchpad(
        caps.clone(),
        &chat_post,
    ).await?;

    if !supports_tools {
        tracing::warn!("supports_tools is false");
    }

    chat_post.max_tokens = n_ctx;
    chat_post.scratchpad = scratchpad_name.clone();

    let scratchpad = crate::scratchpads::create_chat_scratchpad(
        global_context.clone(),
        caps,
        model_name.to_string(),
        &chat_post,
        &scratchpad_name,
        &scratchpad_patch,
        ALLOW_AT,
        supports_tools,
    ).await?;

    Ok((chat_post, scratchpad))
}

#[allow(dead_code)]
async fn chat_interaction_stream() {
    todo!();
}

async fn chat_interaction_non_stream(
    global_context: Arc<ARwLock<GlobalContext>>,
    spad: Box<dyn ScratchpadAbstract>,
    prompt: &String,
    chat_post: &ChatPost,
    client: Client,
    api_key: &String,
) -> Result<Vec<ChatMessage>, String> {
    let t1 = std::time::Instant::now();
    let j = crate::restream::scratchpad_interaction_not_stream_json(
        global_context.clone(),
        spad,
        "chat".to_string(),
        prompt,
        chat_post.model.clone(),
        client,
        api_key.clone(),
        &chat_post.parameters,
        chat_post.only_deterministic_messages,
    ).await.map_err(|e| {
        warn!("network error communicating with the (2): {:?}", e);
        "network error communicating with the model (2)".to_string()
    })?;
    info!("non stream generation took {:?}ms", t1.elapsed().as_millis() as i32);

    info!("j: {:#?}", j);

    let usage_mb = j.get("usage")
        .and_then(|value| match value {
            Value::Object(o) => Some(o),
            v => {
                warn!("usage is not a dict: {:?}; Metering is lost", v);
                None
            }
        })
        .and_then(|o| match serde_json::from_value::<ChatUsage>(Value::Object(o.clone())) {
            Ok(usage) => Some(usage),
            Err(e) => {
                warn!("Failed to parse usage object: {:?}; Metering is lost", e);
                None
            }
        });

    let choice0_msg = j["choices"].as_array()
        .and_then(|array| array.get(0))
        .and_then(|choice0| choice0.get("message"))
        .ok_or(
        "error parsing model's output: choice0.message doesn't exist".to_string()
    )?;

    let det_messages = j.get("deterministic_messages")
        .and_then(|value| value.as_array())
        .and_then(|arr| {
            serde_json::from_value::<Vec<ChatMessage>>(Value::Array(arr.clone())).ok()
        }).unwrap_or_else(Vec::new);

    // convert choice[0] to a ChatMessage (we don't have code like this in any other place in rust, only in python and typescript)
    let (role, content, tool_calls, tool_call_id) = {
        (
            choice0_msg.get("role")
                .and_then(|v| v.as_str())
                .ok_or("error parsing model's output: choice0.message.role doesn't exist or is not a string".to_string())?.to_string(),
            choice0_msg.get("content")
                .and_then(|v| v.as_str())
                .unwrap_or("").to_string(),
            choice0_msg.get("tool_calls")
                .and_then(|v| v.as_array())
                .and_then(|arr| {
                    serde_json::from_value::<Vec<ChatToolCall>>(Value::Array(arr.clone()))
                        .map_err(|_| "error parsing model's output: choice0.message.tool_calls is not a valid ChatToolCall array".to_string())
                        .ok()
                }),
            choice0_msg.get("tool_call_id")
                .and_then(|v| v.as_str())
                .unwrap_or("").to_string()
        )
    };
    let msg = ChatMessage {
        role,
        content,
        tool_calls,
        tool_call_id,
        usage: usage_mb,
    };

    let mut results = vec![];
    results.extend(det_messages);
    results.push(msg);

    Ok(results)
}

async fn chat_interaction(
    global_context: Arc<ARwLock<GlobalContext>>,
    mut spad: Box<dyn ScratchpadAbstract>,
    chat_post: &mut ChatPost,
) -> Result<Vec<ChatMessage>, String> {
    let (client, api_key) = {
        let cx_locked = global_context.write().await;
        (cx_locked.http_client.clone(), cx_locked.cmdline.api_key.clone())
    };
    let prompt = spad.prompt(chat_post.max_tokens, &mut chat_post.parameters).await?;

    let stream = chat_post.stream.unwrap_or(false);
    return if stream {
        todo!();
    } else {
        Ok(chat_interaction_non_stream(
            global_context.clone(),
            spad,
            &prompt,
            chat_post,
            client,
            &api_key,
        ).await?)
    }
}

pub async fn execute_subchat_single_iteration(
    gcx: Arc<ARwLock<GlobalContext>>,
    model_name: &str,
    messages: &Vec<ChatMessage>,
    tools_turn_on: &Vec<String>,
    tool_choice: Option<String>,
    only_deterministic_messages: bool,
) -> Result<Vec<ChatMessage>, String> {
    // this ignores customized tools
    let tools_turned_on_by_cmdline = crate::at_tools::tools::at_tools_merged_and_filtered(gcx.clone()).await.keys().cloned().collect::<Vec<_>>();
    let tools_turn_on_set: HashSet<String> = tools_turn_on.iter().cloned().collect();
    let tools_turned_on_by_cmdline_set: HashSet<String> = tools_turned_on_by_cmdline.into_iter().collect();
    let tools_on_intersection: Vec<String> = tools_turn_on_set.intersection(&tools_turned_on_by_cmdline_set).cloned().collect();
    let tools_compiled_in_only = crate::at_tools::tools::tools_compiled_in(&tools_on_intersection).unwrap_or_else(|e|{
        tracing::error!("Error loading compiled_in_tools: {:?}", e);
        vec![]
    });
    let tools = tools_compiled_in_only.into_iter().map(|x|x.into_openai_style()).collect::<Vec<_>>();
    info!("tools_turned_on_by_cmdline_set {:?}", tools_turned_on_by_cmdline_set);
    info!("tools_turn_on {:?}", tools_turn_on);
    info!("XXXX {:?}", tools);

    let (mut chat_post, spad) = create_chat_post_and_scratchpad(
        gcx.clone(),
        model_name,
        messages.iter().collect::<Vec<_>>(),
        Some(TEMPERATURE),
        MAX_NEW_TOKENS,
        Some(tools),
        tool_choice.clone(),
        only_deterministic_messages,
    ).await?;
    let chat_response_msgs = chat_interaction(gcx.clone(), spad, &mut chat_post).await?;
    let mut result = messages.clone();
    if ALLOW_AT {
        while let Some(message) = result.last() {
            if message.role != "user" {
                break;
            }
            result.pop();
        }
    }
    result.extend(chat_response_msgs);
    Ok(result)
}

pub async fn execute_subchat(
    gcx: Arc<ARwLock<GlobalContext>>,
    model_name: &str,
    messages: &Vec<ChatMessage>,
    tools_turn_on: &Vec<String>,
    wrap_up_depth: usize,
    wrap_up_tokens_cnt: usize,  // when reached wrap_up_tokens_cnt -> insert "user" with text "wrap it up, tokens are over"; tools are disabled
) -> Result<Vec<ChatMessage>, String> {
    let mut messages = messages.clone();
    // let mut chat_usage = ChatUsage { ..Default::default() };
    let mut step_n = 0;
    loop {
        let last_message = messages.last().unwrap();
        if last_message.role == "assistant" && last_message.tool_calls.is_none() {
            // don't have tool calls, exit the loop unconditionally, model thinks it has finished the work
            break;
        }
        if last_message.role == "assistant" && !last_message.tool_calls.is_none() {
            // have tool calls, let's see if we need to wrap up or not
            if step_n >= wrap_up_depth {
                break;
            }
            if let Some(usage) = &last_message.usage {
                if usage.prompt_tokens + usage.completion_tokens > wrap_up_tokens_cnt {
                    break;
                }
            }
        }
        messages = execute_subchat_single_iteration(
            gcx.clone(),
            model_name,
            &messages,
            tools_turn_on,
            Some("auto".to_string()),
            false,
        ).await?;
        step_n += 1;
    }
    Ok(messages)
}
