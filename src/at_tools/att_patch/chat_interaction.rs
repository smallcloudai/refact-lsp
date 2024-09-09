use std::collections::HashMap;
use itertools::Itertools;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tokio::sync::{Mutex as AMutex, RwLock as ARwLock};
use std::sync::RwLock as StdRwLock;
use tokenizers::Tokenizer;
use tracing::{info, warn};

use crate::at_commands::at_commands::AtCommandsContext;
use crate::at_commands::at_file::{file_repair_candidates, context_file_from_file_path};
use crate::at_tools::att_patch::tool::{DefaultToolPatch, PatchArguments, N_CHOICES};
use crate::at_tools::att_patch::ast_interaction::get_signatures_by_imports_traversal;
use crate::at_tools::subchat::subchat_single;
use crate::cached_tokenizers::cached_tokenizer;
use crate::call_validation::{ChatMessage, ChatToolCall, ChatToolFunction, ChatUsage};
use crate::global_context::{GlobalContext, try_load_caps_quickly_if_not_present};
use crate::scratchpads::pp_utils::count_tokens;


#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct LocateItem {
    pub file_path: String,
    pub reason: String,
    pub description: String,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct LocateData {
    pub files: Vec<LocateItem>,
    pub symbols: Vec<String>,
}

async fn load_tokenizer(
    gcx: Arc<ARwLock<GlobalContext>>,
    model: &str,
) -> Result<Arc<StdRwLock<Tokenizer>>, String> {
    let caps = try_load_caps_quickly_if_not_present(gcx.clone(), 0).await.map_err(|e| {
        warn!("load_tokenizer: failed to load caps.\nERROR: {}", e);
        format!("load_tokenizer: failed to load caps.\nERROR: {}", e)
    })?;
    cached_tokenizer(caps.clone(), gcx.clone(), model.to_string()).await
}

async fn format_diff_prompt(gcx: Arc<ARwLock<GlobalContext>>) -> String {
    let mut workspace_dirs = {
        let workspace_dirs_arc = gcx.read().await.documents_state.workspace_folders.clone();
        let dirs_lock = workspace_dirs_arc.lock().unwrap();
        dirs_lock.clone().into_iter().map(|x| x.to_string_lossy().to_string()).collect::<Vec<_>>()
    };
    if workspace_dirs.is_empty() {
        workspace_dirs.push(String::from("/home/user/project"));
    }
    let workspace_project_dirs = workspace_dirs.join("\n");
    let first_workspace_dir = workspace_dirs.first().expect("added above");
    DefaultToolPatch::prompt(&workspace_project_dirs, first_workspace_dir)
}

async fn find_last_valid_locate_message(
    ccx: Arc<AMutex<AtCommandsContext>>,
) -> Result<LocateData, String> {
    let messages = ccx.lock().await.messages.clone();
    let locate_tool_ids = messages.iter()
        .flat_map(|message| message.tool_calls.iter().flat_map(|tools| tools.iter()))
        .filter(|tool| tool.function.name == "locate")
        .map(|tool| tool.id.clone())
        .collect::<Vec<_>>();
    
    let locate_messages = locate_tool_ids.iter().filter_map(|id|{
        messages.iter().find_or_first(|x|x.tool_call_id == *id)
    }).collect::<Vec<_>>();

    let locate_data_vec = locate_messages.iter().filter_map(|msg| {
       serde_json::from_str::<Option<LocateData>>(&msg.content).unwrap_or_else(|err|{
           warn!("failed to parse locate data: {:?}", err);
           None
       })
    }).collect::<Vec<_>>();

    locate_data_vec.last().cloned()
        .ok_or("locate data could not be located even though it was requested for use, call `locate` tool or pass the filenames directly".to_string())
}

fn pretend_tool_call(tool_name: &str, arguments: String) -> ChatMessage {
    let tool_call = ChatToolCall {
        id: format!("{tool_name}_123"),
        function: ChatToolFunction {
            arguments,
            name: tool_name.to_string()
        },
        tool_type: "function".to_string(),
    };
    ChatMessage {
        role: "assistant".to_string(),
        content: "".to_string(),
        tool_calls: Some(vec![tool_call]),
        tool_call_id: "".to_string(),
        ..Default::default()
    }
}

async fn cat_paths_context_and_symbols(
    ccx: Arc<AMutex<AtCommandsContext>>,
    paths_to_change: Vec<(String, Option<String>)>,
    paths_context: Vec<String>,
    symbols: Vec<String>,
    messages: &Vec<ChatMessage>,
    tool_call_id: &String,
    usage: &mut ChatUsage,
    model: &str,
    temperature: Option<f32>,
    max_new_tokens: usize,
) -> Result<Vec<ChatMessage>, String> {
    let log_prefix = chrono::Local::now().format("%Y%m%d-%H%M%S").to_string();
    let gcx = ccx.lock().await.global_context.clone();
    let mut messages = messages.clone();
    let mut cat_args = HashMap::new();

    if !paths_context.is_empty() {
        cat_args.insert("paths".to_string(), paths_context.join(","));
    }
    if !symbols.is_empty() {
        cat_args.insert("symbols".to_string(), symbols.join(","));
    }
    cat_args.insert("skeleton".to_string(), "true".to_string());
    
    if cat_args.get("paths").is_none() && cat_args.get("symbols").is_none() {
        if let Some(paths) = get_signatures_by_imports_traversal(
            &paths_to_change.iter().map(|x| x.0.clone()).collect(), gcx.clone()
        ).await {
            cat_args.insert("paths".to_string(), paths.iter().map(|x| x.to_string_lossy()).join(","));
        }
    }
    if cat_args.get("paths").unwrap_or(&"".to_string()).is_empty() {
        warn!("cat_paths_context_and_symbols: no cat will be performed: no paths provided");
        return Ok(messages);
    }
    
    messages.push(pretend_tool_call("cat", serde_json::to_string(&cat_args).unwrap()));

    Ok(subchat_single(
        ccx.clone(),
        &model,
        messages,
        vec!["cat".to_string()],
        None,
        true,
        temperature,
        Some(max_new_tokens),
        N_CHOICES,
        Some(usage),
        Some(format!("{log_prefix}-cat_paths_context_and_symbols")),
        Some(tool_call_id.clone()),
        Some(format!("{log_prefix}-cat_paths_context_and_symbols")),
    ).await?.get(0)
        .ok_or("cat_paths_context_and_symbols: deterministic message was empty".to_string())?.clone())
}

async fn path_to_change_to_message(gcx: Arc<ARwLock<GlobalContext>>, path: String, description: Option<String>) -> ChatMessage {
    let mut text = "".to_string();
    text.push_str(&format!("File to modify: {}\n", path));
    if let Some(d) = description {
        text.push_str(&format!("Description: {}\n", d));
    }

    let candidates = file_repair_candidates(gcx.clone(), &path, 10, false).await;
    match context_file_from_file_path(gcx.clone(), candidates, path.clone()).await {
        Ok(context_file) => text.push_str(&format!("Content:\n```\n{}\n```", context_file.file_content)),
        Err(e) => {
            text = format!("The file `{}` cannot be found on the disk; it needs to be added to the project (use add format).\nERROR: {}", path, e);
        }
    }
    ChatMessage::new("user".to_string(), text)
}

async fn make_chat_history(
    ccx: Arc<AMutex<AtCommandsContext>>,
    model: &str,
    max_tokens: usize,
    temperature: Option<f32>,
    max_new_tokens: usize,
    args: &PatchArguments,
    tool_call_id: &String,
    usage: &mut ChatUsage,
) -> Result<Vec<ChatMessage>, String> {
    let gcx = ccx.lock().await.global_context.clone();
    let tokenizer = { 
        let tokenizer_arc = load_tokenizer(gcx.clone(), model).await?; 
        tokenizer_arc.clone().read().unwrap().clone()
    };

    let mut tokens: usize = 0;
    let max_tokens = max_tokens.saturating_sub(max_new_tokens);
    let system_prompt = format_diff_prompt(gcx.clone()).await;
    let task_message = args.todo.clone();
    
    let mut chat_messages = vec![];
    chat_messages.push(ChatMessage::new("system".to_string(), system_prompt.to_string()));

    tokens += 3 + count_tokens(&tokenizer, &system_prompt);
    tokens += 3 + count_tokens(&tokenizer, &task_message);

    if tokens > max_tokens {
        return Err(format!("too many tokens for the todo message: {tokens} > {max_tokens}, reduce the todo message length"));
    }

    let (paths_to_change, paths_context, symbols) = if args.pick_locate_json_above {
        let locate_data = find_last_valid_locate_message(ccx.clone()).await?;
        let paths_to_change = locate_data.files.iter()
            .filter(|x| x.reason.to_lowercase() == "to_change")
            .map(|x| (x.file_path.clone(), Some(x.description.clone())))
            .collect::<Vec<_>>();
        let paths_context = locate_data.files.iter()
            .filter(|x| x.reason.to_lowercase() != "to_change")
            .map(|x| x.file_path.clone())
            .collect::<Vec<_>>();
        (paths_to_change, paths_context, locate_data.symbols)
    } else {
        (args.paths.iter().map(|x| (x.clone(), None)).collect::<Vec<_>>(), vec![], vec![])
    };
    
    let mut tokens_per_path_to_change = vec![];
    for path in paths_to_change.iter() {
        let msg = path_to_change_to_message(gcx.clone(), path.0.clone(), path.1.clone()).await;
        let n_ctx = 3 + count_tokens(&tokenizer, &msg.content);
        tokens += n_ctx;
        // todo: shortify path for a clearer error
        tokens_per_path_to_change.push((n_ctx, path.clone()));
        chat_messages.push(msg);
    }
    
    if tokens > max_tokens {
        // todo: print as well all files that were not found
        let err_msg = format!(
            "Provided files exceeded tokens limit (tokens: {tokens} > {max_tokens}):\n{}", 
            tokens_per_path_to_change.iter().map(|x|format!("{:?}: {} tokens", x.1, x.0)).collect::<Vec<_>>().join("\n")
        );
        info!(err_msg);
        return Err(err_msg);
    }
    
    // todo: refactor code below
    let mut chat_messages = cat_paths_context_and_symbols(
        ccx.clone(), paths_to_change, paths_context, symbols, &mut chat_messages, tool_call_id,
        usage, model, temperature, max_new_tokens
    ).await?
        .iter()
        .map(|x| if x.role != "tool" { x.clone() } else {
            let mut x = x.clone();
            x.content = "Files for extra context (do not modify them!):".to_string();
            x
        })
        .collect::<Vec<_>>();

    chat_messages.push(ChatMessage::new("user".to_string(), task_message));


    Ok(chat_messages)
}

// todo: refactor this function
pub async fn execute_chat_model(
    ccx: Arc<AMutex<AtCommandsContext>>,
    model: &str,
    max_tokens: usize,
    temperature: Option<f32>,
    max_new_tokens: usize,
    tool_call_id: &String,
    args: &PatchArguments,
    usage: &mut ChatUsage,
) -> Result<Vec<String>, String> {
    let messages = make_chat_history(
        ccx.clone(), model, max_tokens, temperature,
        max_new_tokens, args, tool_call_id, usage,
    ).await?;
    let log_prefix = chrono::Local::now().format("%Y%m%d-%H%M%S").to_string();
    let response = subchat_single(
        ccx.clone(),
        model,
        messages,
        vec![],
        None,
        false,
        temperature,
        Some(max_new_tokens),
        N_CHOICES,
        Some(usage),
        Some(format!("{log_prefix}-patch")),
        Some(tool_call_id.clone()),
        Some(format!("{log_prefix}-patch")),
    ).await;

    match response {
        Ok(res) => {
            Ok(res
                .iter()
                .filter_map(|x| x
                    .iter()
                    .last()
                    .map(|x| {
                        if x.role == "assistant" { Some(x.content.clone()) } else { None }
                    })
                    .flatten())
                .collect::<Vec<_>>())
        }
        Err(err) => Err(err)
    }
}