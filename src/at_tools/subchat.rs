use std::collections::HashSet;
use std::sync::{Arc, RwLock};
use reqwest::Client;
use tokio::sync::RwLock as ARwLock;
use serde_json::Value;
use tracing::{info, warn};

use tokenizers::Tokenizer;

use crate::at_commands::at_commands::AtCommandsContext;
use crate::call_validation::{ChatMessage, ChatPost, ChatUsage, SamplingParameters};
use crate::global_context::{GlobalContext, try_load_caps_quickly_if_not_present};
use crate::http::routers::v1::chat::lookup_chat_scratchpad;
use crate::scratchpad_abstract::ScratchpadAbstract;
use crate::{cached_tokenizers, scratchpads};
use crate::scratchpads::chat_utils_rag::count_tokens;


fn limit_messages(
    t: Arc<RwLock<Tokenizer>>,
    messages: Vec<&ChatMessage>,
    max_new_tokens: usize,
    context_size: usize,
) -> Result<Vec<ChatMessage>, String> {
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
            continue;
        }
        let tokens = 3 + count_tokens(&t_guard, m.content.as_str());
        if tokens_used + tokens < tokens_limit {
            messages_new.push(m.clone());
        }
    }
    
    // msg that called a tool was dropped -> drop tool
    let tool_call_ids = messages_new.iter().filter_map(|x|x.tool_calls.clone()).flatten().map(|x|x.id).collect::<HashSet<_>>();
    messages_new.retain(|x| x.tool_call_id.is_empty() || tool_call_ids.contains(&x.tool_call_id));
    
    Ok(messages_new)
}

async fn create_chat_post_and_scratchpad(
    ccx: &mut AtCommandsContext,
    model_name: &str,
    messages: Vec<&ChatMessage>,
    stream: bool,
    temperature: Option<f32>,
    max_new_tokens: usize,
    tools: Option<Vec<Value>>,
    tool_choice: Option<String>,
) -> Result<(ChatPost, Box<dyn ScratchpadAbstract>), String> {
    let caps = try_load_caps_quickly_if_not_present(
        ccx.global_context.clone(), 0,
    ).await.map_err(|e| { 
        warn!("no caps: {:?}", e);
        "no caps".to_string()
    })?;

    let tokenizer = cached_tokenizers::cached_tokenizer(
        caps.clone(), ccx.global_context.clone(), model_name.to_string(),
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

    let (model_name, scratchpad_name, scratchpad_patch, n_ctx, supports_tools) = lookup_chat_scratchpad(
        caps.clone(),
        &chat_post,
    ).await?;
    
    let messages = limit_messages(
        tokenizer,
        messages,
        max_new_tokens,
        chat_post.max_tokens,
    )?;

    chat_post.messages = messages;
    chat_post.max_tokens = n_ctx;
    chat_post.scratchpad = scratchpad_name.clone();
    
    let scratchpad = scratchpads::create_chat_scratchpad(
        ccx.global_context.clone(),
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

async fn chat_iteraction_stream() {
    todo!();
}

async fn chat_interaction_non_stream(
    global_context: Arc<ARwLock<GlobalContext>>,
    spad: Box<dyn ScratchpadAbstract>,
    prompt: &String,
    chat_post: &ChatPost,
    client: Client,
    api_key: &String,
) -> Result<((String, Option<ChatUsage>)), String> {
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
    
    let choice0_message_content = messages["choices"]
        .as_array()
        .and_then(|array| array.get(0))
        .and_then(|choice0| choice0.get("message"))
        .and_then(|message| message.get("content"))
        .and_then(|content| content.as_str());
    
    match choice0_message_content {
        Some(content) => Ok((content.to_string(), usage_mb)),
        None => Err("error parsing model's output: choice[0].message.content doesn't exist or is not a string".to_string()),
    }
}

async fn chat_interaction(
    ccx: &mut AtCommandsContext,
    mut spad: Box<dyn ScratchpadAbstract>,
    chat_post: &mut ChatPost,
) -> Result<(String, Option<ChatUsage>), String> {
    let (client, api_key) = {
        let cx_locked = ccx.global_context.write().await;
        (cx_locked.http_client.clone(), cx_locked.cmdline.api_key.clone())
    };
    let prompt = spad.prompt(chat_post.max_tokens, &mut chat_post.parameters).await?;
    
    let stream = chat_post.stream.unwrap_or(false);
    return if stream {
        todo!();
    } else {
        Ok(chat_interaction_non_stream(
            ccx.global_context.clone(),
            spad,
            &prompt,
            chat_post,
            client,
            &api_key,
        ).await?)
    }
}

async fn execute_subchat(
    ccx: &mut AtCommandsContext,
    messages: Vec<ChatMessage>,
    depth: usize,
    max_new_tokens: usize,
) -> Result<Vec<ChatMessage>, String> {
    
    let mut messages = messages;
    let mut chat_usage = ChatUsage { ..Default::default() };    
    let mut step_n = 0;
    loop {
        if let Some(last_message) = messages.last() {
            if step_n >= depth {
                break;
            }
            if last_message.role == "assistant" && last_message.tool_calls.is_none() {
                break;
            }
        }
        // TODO: support tools
        // TODO: support stream = true
        // TODO: support N
        let (mut chat_post, spad) = create_chat_post_and_scratchpad(
            ccx, "gpt-4o-mini",
            messages.iter().collect::<Vec<_>>(),
            false, None, max_new_tokens, None, None
        ).await?;
        messages = chat_post.messages.clone();

        let (chat_response, usage_mb) = chat_interaction(ccx, spad, &mut chat_post).await?;
        if let Some(usage) = usage_mb {
            chat_usage.completion_tokens += usage.completion_tokens;
            chat_usage.prompt_tokens += usage.prompt_tokens;
            chat_usage.total_tokens += usage.total_tokens;
        }
        
        messages.push(ChatMessage {
            role: "assistant".to_string(),
            content: chat_response,
            tool_calls: None,
            tool_call_id: "".to_string(),
            ..Default::default()
        });
        step_n += 1;
    }
    
    Ok(messages)
}
