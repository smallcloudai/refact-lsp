use std::sync::Arc;
use std::sync::RwLock as StdRwLock;

use axum::Extension;
use axum::response::Result;
use hyper::{Body, Response, StatusCode};
use tracing::info;

use crate::call_validation::ChatPost;
use crate::caps;
use crate::caps::CodeAssistantCaps;
use crate::custom_error::ScratchError;
use crate::global_context::SharedGlobalContext;
use crate::scratchpads;

async fn _lookup_chat_scratchpad(
    caps: Arc<StdRwLock<CodeAssistantCaps>>,
    chat_post: &ChatPost,
) -> Result<(String, String, serde_json::Value, usize, bool), String> {
    let caps_locked = caps.read().unwrap();
    let (model_name, recommended_model_record) =
        caps::which_model_to_use(
            &caps_locked.code_chat_models,
            &chat_post.model,
            &caps_locked.code_chat_default_model,
        )?;
    let (sname, patch) = caps::which_scratchpad_to_use(
        &recommended_model_record.supports_scratchpads,
        &chat_post.scratchpad,
        &recommended_model_record.default_scratchpad,
    )?;
    Ok((model_name, sname.clone(), patch.clone(), recommended_model_record.n_ctx, recommended_model_record.supports_tools))
}

pub async fn handle_v1_chat_completions(
    // standard openai-style handler
    Extension(global_context): Extension<SharedGlobalContext>,
    body_bytes: hyper::body::Bytes,
) -> Result<Response<Body>, ScratchError> {
    chat(global_context, body_bytes, false).await
}

pub async fn handle_v1_chat(
    // less-standard openai-style handler that sends role="context_*" messages first, rewrites the user message
    Extension(global_context): Extension<SharedGlobalContext>,
    body_bytes: hyper::body::Bytes,
) -> Result<Response<Body>, ScratchError> {
    chat(global_context, body_bytes, true).await
}

async fn chat(
    global_context: SharedGlobalContext,
    body_bytes: hyper::body::Bytes,
    allow_at: bool,
) -> Result<Response<Body>, ScratchError> {
    let mut chat_post = serde_json::from_slice::<ChatPost>(&body_bytes).map_err(|e| {
        info!("chat handler cannot parse input:\n{:?}", body_bytes);
        ScratchError::new(StatusCode::BAD_REQUEST, format!("JSON problem: {}", e))
    })?;
    let caps = crate::global_context::try_load_caps_quickly_if_not_present(global_context.clone(), 0).await?;
    let (model_name, scratchpad_name, scratchpad_patch, n_ctx, supports_tools) = _lookup_chat_scratchpad(
        caps.clone(),
        &chat_post,
    ).await.map_err(|e| {
        ScratchError::new(StatusCode::BAD_REQUEST, format!("{}", e))
    })?;
    if chat_post.parameters.max_new_tokens == 0 {
        chat_post.parameters.max_new_tokens = chat_post.max_tokens;
    }
    if chat_post.parameters.max_new_tokens == 0 {
        chat_post.parameters.max_new_tokens = 1024;
    }
    chat_post.parameters.temperature = Some(chat_post.parameters.temperature.unwrap_or(chat_post.temperature.unwrap_or(0.2)));
    chat_post.model = model_name.clone();
    // chat_post.stream = Some(false);  // for debugging 400 errors that are hard to debug with streaming (because "data: " is not present and the error message is ignored by the library)
    let (client1, api_key) = {
        let cx_locked = global_context.write().await;
        (cx_locked.http_client.clone(), cx_locked.cmdline.api_key.clone())
    };
    let mut scratchpad = scratchpads::create_chat_scratchpad(
        global_context.clone(),
        caps,
        model_name.clone(),
        &chat_post,
        &scratchpad_name,
        &scratchpad_patch,
        allow_at,
        supports_tools,
    ).await.map_err(|e|
        ScratchError::new(StatusCode::BAD_REQUEST, e)
    )?;
    let t1 = std::time::Instant::now();
    let prompt = scratchpad.prompt(
        n_ctx,
        &mut chat_post.parameters,
    ).await.map_err(|e|
        ScratchError::new(StatusCode::INTERNAL_SERVER_ERROR, format!("Prompt: {}", e))
    )?;
    info!("chat prompt {:?}", t1.elapsed());
    if !chat_post.chat_id.is_empty() {
        let cache_dir = {
            let gcx_locked = global_context.read().await;
            gcx_locked.cache_dir.clone()
        };
        let notes_dir_path = cache_dir.join("chats");
        let _ = std::fs::create_dir_all(&notes_dir_path);
        let notes_path = notes_dir_path.join(format!("chat{}_{}.json",
            chrono::Local::now().format("%Y%m%d"),
            chat_post.chat_id,
        ));
        let _ = std::fs::write(&notes_path, serde_json::to_string_pretty(&chat_post.messages).unwrap());
    }
    if chat_post.stream.is_some() && !chat_post.stream.unwrap() {
        crate::restream::scratchpad_interaction_not_stream(
            global_context.clone(),
            scratchpad,
            "chat".to_string(),
            &prompt,
            model_name,
            client1,
            api_key,
            &chat_post.parameters,
            chat_post.only_deterministic_messages,
        ).await
    } else {
        crate::restream::scratchpad_interaction_stream(
            global_context.clone(),
            scratchpad,
            "chat-stream".to_string(),
            prompt,
            model_name,
            client1,
            api_key,
            chat_post.parameters.clone(),
            chat_post.only_deterministic_messages,
        ).await
    }
}

