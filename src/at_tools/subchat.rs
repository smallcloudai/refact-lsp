use std::collections::HashSet;
use std::sync::{Arc, RwLock};
use reqwest::Client;
use tokio::sync::RwLock as ARwLock;
use serde_json::Value;
use tracing::{info, warn};

use tokenizers::Tokenizer;

use crate::call_validation::{ChatMessage, ChatPost, ChatToolCall, ChatUsage, SamplingParameters};
use crate::global_context::{GlobalContext, try_load_caps_quickly_if_not_present};
use crate::http::routers::v1::chat::lookup_chat_scratchpad;
use crate::scratchpad_abstract::ScratchpadAbstract;
use crate::{cached_tokenizers, scratchpads};
use crate::scratchpads::chat_utils_rag::count_tokens;


const TEMP: f32 = 0.2;
const MAX_NEW_TOKENS: usize = 4096;


fn limit_messages(
    t: Arc<RwLock<Tokenizer>>,
    messages: Vec<&ChatMessage>,
    max_new_tokens: usize,
    context_size: usize,
) -> Result<(Vec<ChatMessage>, usize), String> {
    let tokens_limit= context_size.saturating_sub(max_new_tokens);
    let mut tokens_used = 0;
    let t_guard = t.read().unwrap();
    
    let mut messages_take = vec![];
    for (idx, m) in messages.iter().enumerate() {
        if m.role == "system" || m.role == "user" {
            tokens_used += 3 + count_tokens(&t_guard, m.content.as_str());
            messages_take.push(idx)
        }
    }
    let mut messages_new = vec![];
    for (idx, m) in messages.iter().cloned().enumerate().rev() {
        if messages_take.contains(&idx) {
            messages_new.push(m.clone());
            println!("keeping message_idx (S||U): {}", idx);
            continue;
        }
        let tokens = 3 + count_tokens(&t_guard, m.content.as_str());
        if tokens_used + tokens < tokens_limit {
            messages_new.push(m.clone());
            tokens_used += tokens;
            println!("keeping message_idx: {}: +{} tokens", idx, tokens);
        } else {
            println!("dropping message_idx: {} (OOT): {} tokens", idx, tokens);
        }
    }
    messages_new.reverse();

    // msg that called a tool was dropped -> drop tool
    let tool_call_ids = messages_new.iter().filter_map(|x|x.tool_calls.clone()).flatten().map(|x|x.id).collect::<HashSet<_>>();
    messages_new.retain(|x| x.tool_call_id.is_empty() || tool_call_ids.contains(&x.tool_call_id));
    
    Ok((messages_new, tokens_used))
}

async fn create_chat_post_and_scratchpad(
    global_context: Arc<ARwLock<GlobalContext>>,
    model_name: &str,
    messages: Vec<&ChatMessage>,
    stream: bool,
    temperature: Option<f32>,
    max_new_tokens: usize,
    tools: Option<Vec<Value>>,
    tool_choice: Option<String>,
    wrap_up_tokens_cnt_mb: Option<usize>,
) -> Result<(ChatPost, Box<dyn ScratchpadAbstract>), String> {
    let caps = try_load_caps_quickly_if_not_present(
        global_context.clone(), 0,
    ).await.map_err(|e| { 
        warn!("no caps: {:?}", e);
        "no caps".to_string()
    })?;

    let tokenizer = cached_tokenizers::cached_tokenizer(
        caps.clone(), global_context.clone(), model_name.to_string(),
    ).await?;

    let mut chat_post = ChatPost {
        messages: vec![],
        parameters: SamplingParameters {
            max_new_tokens,
            temperature,
            top_p: None,
            stop: vec![],
        },
        model: model_name.to_string(),
        scratchpad: "".to_string(),
        stream: Some(stream),
        temperature,
        max_tokens: 0,
        tools,
        tool_choice,
        only_deterministic_messages: false,
        chat_id: "".to_string(),
    };

    let (model_name, scratchpad_name, scratchpad_patch, n_ctx, mut supports_tools) = lookup_chat_scratchpad(
        caps.clone(),
        &chat_post,
    ).await?;
    
    let (mut messages, tok_used) = limit_messages(
        tokenizer,
        messages,
        max_new_tokens,
        n_ctx,
    )?;
    
    if let Some(wrap_up_tokens_cnt) = wrap_up_tokens_cnt_mb {
        if tok_used > wrap_up_tokens_cnt {
            if let Some(last_message) = messages.last_mut() {
                last_message.tool_calls = None;
            }
            messages.push(ChatMessage::new(
                "user".to_string(), "You are out of tokens for additional context. You must formulate your answer right now.".to_string(),
            ));
            chat_post.tools = None;
            chat_post.tool_choice = None;
            supports_tools = false;
        }
    }

    // chat_post.messages = messages.iter().cloned().cloned().collect::<Vec<_>>();
    chat_post.messages = messages;
    chat_post.max_tokens = n_ctx;
    chat_post.scratchpad = scratchpad_name.clone();
    
    let scratchpad = scratchpads::create_chat_scratchpad(
        global_context.clone(),
        caps,
        model_name.to_string(),
        &chat_post,
        &scratchpad_name,
        &scratchpad_patch,
        false,
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
) -> Result<(Vec<ChatMessage>, Option<ChatUsage>), String> {
    let t1 = std::time::Instant::now();
    let messages = crate::restream::scratchpad_interaction_not_stream_json(
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
    
    // println!("messages: {:#?}", messages);
    
    let usage_mb = messages.get("usage")
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

    let choice0_msg = messages["choices"].as_array()
        .and_then(|array| array.get(0))
        .and_then(|choice0| choice0.get("message"))
        .ok_or(
        "error parsing model's output: choice0.message doesn't exist".to_string()
    )?;

    let det_messages = messages.get("deterministic_messages")
        .and_then(|value| value.as_array())
        .and_then(|arr| {
            serde_json::from_value::<Vec<ChatMessage>>(Value::Array(arr.clone())).ok()
        }).unwrap_or_else(Vec::new);
    
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
        ..Default::default()
    };

    let mut results = vec![];
    results.extend(det_messages);
    results.push(msg);
    
    Ok((results, usage_mb))
}

async fn chat_interaction(
    global_context: Arc<ARwLock<GlobalContext>>,
    mut spad: Box<dyn ScratchpadAbstract>,
    chat_post: &mut ChatPost,
) -> Result<(Vec<ChatMessage>, Option<ChatUsage>), String> {
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

pub async fn execute_subchat(
    global_context: Arc<ARwLock<GlobalContext>>,
    model_name: &str,
    messages: Vec<ChatMessage>,
    max_depth: usize,
    tools: Option<Vec<Value>>,
    tool_choice: Option<String>,
    wrap_up_tokens_cnt: Option<usize>, // when reached max_tokens_allowed -> insert "user" with text "wrap it up, tokens are over"; tools are disabled
) -> Result<Vec<ChatMessage>, String> {
    
    let mut messages = messages;
    let mut chat_usage = ChatUsage { ..Default::default() };    
    let mut step_n = 0;
    loop {
        if let Some(last_message) = messages.last() {
            if step_n >= max_depth {
                break;
            }
            if last_message.role == "assistant" && last_message.tool_calls.is_none() {
                break;
            }
        }
        // TODO: support stream = true
        let (mut chat_post, spad) = create_chat_post_and_scratchpad(
            global_context.clone(), 
            model_name,
            messages.iter().collect::<Vec<_>>(),
            false, Some(TEMP), MAX_NEW_TOKENS, 
            tools.clone(), tool_choice.clone(),
            wrap_up_tokens_cnt,
        ).await?;
        messages = chat_post.messages.clone();

        let (chat_response_msgs, usage_mb) = chat_interaction(global_context.clone(), spad, &mut chat_post).await?;
        if let Some(usage) = usage_mb {
            chat_usage.completion_tokens += usage.completion_tokens;
            chat_usage.prompt_tokens += usage.prompt_tokens;
            chat_usage.total_tokens += usage.total_tokens;
        }
        
        messages.extend(chat_response_msgs);

        step_n += 1;
    }
    if let Some(last_message) = messages.last_mut() {
        if chat_usage.total_tokens != 0 {
            last_message.usage = Some(chat_usage);
        }
    }
    Ok(messages)
}
