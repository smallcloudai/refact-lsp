use std::collections::HashMap;
use std::sync::Arc;
use std::sync::RwLock as StdRwLock;
use tokio::sync::{RwLock as ARwLock, Mutex as AMutex};
use tokenizers::Tokenizer;
use crate::ast::ast_module::AstModule;
use crate::at_tools::tools::{at_tools_merged_and_filtered, Tool};

pub mod completion_single_file_fim;
pub mod chat_generic;
pub mod chat_llama2;
pub mod chat_passthrough;
pub mod chat_utils_deltadelta;
pub mod chat_utils_limit_history;
pub mod chat_utils_rag;

use crate::call_validation::CodeCompletionPost;
use crate::call_validation::ChatPost;
use crate::global_context::GlobalContext;
use crate::caps::CodeAssistantCaps;
use crate::scratchpad_abstract::ScratchpadAbstract;
use crate::completion_cache;
use crate::telemetry::telemetry_structs;
use crate::cached_tokenizers;


fn verify_has_send<T: Send>(_x: &T) {}


pub async fn create_code_completion_scratchpad(
    global_context: Arc<ARwLock<GlobalContext>>,
    caps: Arc<StdRwLock<CodeAssistantCaps>>,
    model_name_for_tokenizer: String,
    post: &CodeCompletionPost,
    scratchpad_name: &str,
    scratchpad_patch: &serde_json::Value,
    cache_arc: Arc<StdRwLock<completion_cache::CompletionCache>>,
    tele_storage: Arc<StdRwLock<telemetry_structs::Storage>>,
    ast_module: Option<Arc<ARwLock<AstModule>>>,
) -> Result<Box<dyn ScratchpadAbstract>, String> {
    let mut result: Box<dyn ScratchpadAbstract>;
    let tokenizer_arc: Arc<StdRwLock<Tokenizer>> = cached_tokenizers::cached_tokenizer(caps, global_context.clone(), model_name_for_tokenizer).await?;
    if scratchpad_name == "FIM-PSM" {
        result = Box::new(completion_single_file_fim::SingleFileFIM::new(tokenizer_arc, &post, "PSM".to_string(), cache_arc, tele_storage, ast_module, global_context.clone()));
    } else if scratchpad_name == "FIM-SPM" {
        result = Box::new(completion_single_file_fim::SingleFileFIM::new(tokenizer_arc, &post, "SPM".to_string(), cache_arc, tele_storage, ast_module, global_context.clone()));
    } else {
        return Err(format!("This rust binary doesn't have code completion scratchpad \"{}\" compiled in", scratchpad_name));
    }
    result.apply_model_adaptation_patch(scratchpad_patch, HashMap::new()).await?;
    verify_has_send(&result);
    Ok(result)
}

fn post_tools_to_hashmap(tools: &Option<Vec<serde_json::Value>>) -> Option<HashMap<String, serde_json::Value>> {
    tools.as_ref().map(|tools_vec| {
        tools_vec.iter()
            .filter_map(|tool| {
                tool.get("function")
                    .and_then(|function| function.get("name"))
                    .and_then(|name| name.as_str())
                    .map(|name_str| (name_str.to_string(), tool.clone()))
            })
            .collect()
    })
}

pub async fn create_chat_scratchpad(
    global_context: Arc<ARwLock<GlobalContext>>,
    caps: Arc<StdRwLock<CodeAssistantCaps>>,
    model_name_for_tokenizer: String,
    post: &ChatPost,
    scratchpad_name: &str,
    scratchpad_patch: &serde_json::Value,
    allow_at: bool,
    supports_tools: bool,
) -> Result<Box<dyn ScratchpadAbstract>, String> {
    let mut result: Box<dyn ScratchpadAbstract>;
    if scratchpad_name == "CHAT-GENERIC" {
        let tokenizer_arc: Arc<StdRwLock<Tokenizer>> = cached_tokenizers::cached_tokenizer(caps, global_context.clone(), model_name_for_tokenizer).await?;
        result = Box::new(chat_generic::GenericChatScratchpad::new(tokenizer_arc, post, global_context.clone(), allow_at));
    } else if scratchpad_name == "CHAT-LLAMA2" {
        let tokenizer_arc: Arc<StdRwLock<Tokenizer>> = cached_tokenizers::cached_tokenizer(caps, global_context.clone(), model_name_for_tokenizer).await?;
        result = Box::new(chat_llama2::ChatLlama2::new(tokenizer_arc, post, global_context.clone(), allow_at));
    } else if scratchpad_name == "PASSTHROUGH" {
        let tokenizer_arc: Arc<StdRwLock<Tokenizer>> = cached_tokenizers::cached_tokenizer(caps, global_context.clone(), model_name_for_tokenizer).await?;
        result = Box::new(chat_passthrough::ChatPassthrough::new(tokenizer_arc, post, global_context.clone(), allow_at, supports_tools));
    } else {
        return Err(format!("This rust binary doesn't have chat scratchpad \"{}\" compiled in", scratchpad_name));
    }
    let tools_from_post = post_tools_to_hashmap(&post.tools).unwrap_or_default().keys().cloned().collect::<Vec<_>>();
    let exploration_tools: HashMap<String, Arc<AMutex<Box<dyn Tool + Send>>>> = at_tools_merged_and_filtered(global_context.clone()).await.into_iter().filter(|(key, _)|tools_from_post.contains(&key)).collect();
    result.apply_model_adaptation_patch(scratchpad_patch, exploration_tools).await?;
    verify_has_send(&result);
    Ok(result)
}
