use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use hashbrown::HashSet;
use tokio::sync::{Mutex as AMutex, RwLock as ARwLock};
use crate::ast::ast_index::RequestSymbolType;
use crate::at_commands::at_commands::AtCommandsContext;
use crate::at_commands::at_file::{file_repair_candidates, get_project_paths, real_file_path_candidate};
use crate::call_validation::{ChatMessage, ChatToolCall, ChatToolFunction, ChatUsage, SubchatParameters};
use crate::caps::get_model_record;
use crate::files_in_workspace::{Document, get_file_text_from_memory_or_disk};
use crate::global_context::GlobalContext;
use crate::toolbox::toolbox_config::load_customization;


pub async fn unwrap_subchat_params(ccx: Arc<AMutex<AtCommandsContext>>, tool_name: &str) -> Result<SubchatParameters, String> {
    let (gcx, params_mb) = {
        let ccx_locked = ccx.lock().await;
        let gcx = ccx_locked.global_context.clone();
        let params = ccx_locked.subchat_tool_parameters.get(tool_name).cloned();
        (gcx, params)
    };
    let params = match params_mb {
        Some(params) => params,
        None => {
            let tconfig = load_customization(gcx.clone()).await?;
            tconfig.subchat_tool_parameters.get(tool_name).cloned()
                .ok_or_else(|| format!("subchat params for tool {} not found (checked in Post and in Customization)", tool_name))?
        }
    };
    let _ = get_model_record(gcx, &params.model).await?; // check if the model exists
    Ok(params)
}

pub async fn update_usage(usage: Arc<AMutex<ChatUsage>>, usage_collector: &mut ChatUsage) {
    let mut usage_lock = usage.lock().await;
    usage_lock.prompt_tokens += usage_collector.prompt_tokens;
    usage_lock.completion_tokens += usage_collector.completion_tokens;
    usage_lock.total_tokens += usage_collector.total_tokens;
}

pub fn pretend_tool_call(tool_name: &str, tool_arguments: &str) -> ChatMessage {
    let tool_call = ChatToolCall {
        id: format!("{tool_name}_123"),
        function: ChatToolFunction {
            arguments: tool_arguments.to_string(),
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

pub fn reduce_by_counter<I>(values: I, top_n: usize) -> Vec<String>
where
    I: Iterator<Item = String>,
{
    let mut counter = HashMap::new();
    for s in values {
        *counter.entry(s).or_insert(0) += 1;
    }
    let mut counts_vec: Vec<(String, usize)> = counter.into_iter().collect();
    counts_vec.sort_by(|a, b| b.1.cmp(&a.1));
    let top_n: Vec<(String, usize)> = counts_vec.into_iter().take(top_n).collect();
    top_n.into_iter().map(|x|x.0).collect()
}

pub async fn filter_existing_symbols(gcx: Arc<ARwLock<GlobalContext>>, symbols: Vec<String>) -> Result<Vec<String>, String>{
    let ast_symbols_set = {
        let ast = {
            let cx = gcx.read().await;
            cx.ast_module.clone().unwrap()
        };
        let ast_lock = ast.read().await;
        ast_lock.get_symbols_names(RequestSymbolType::All).await?.into_iter().collect::<HashSet<_>>()
    };

    let unique_symbols = symbols.iter().cloned().collect::<HashSet<_>>();
    let sym_intersection = unique_symbols.intersection(&ast_symbols_set).cloned().collect::<HashSet<_>>();
    let filtered_symbols = symbols.iter().cloned().filter(|x|sym_intersection.contains(x)).collect::<Vec<_>>();

    Ok(filtered_symbols)
}

pub async fn assign_symbols_to_paths(gcx: Arc<ARwLock<GlobalContext>>, symbols: Vec<String>, paths: Vec<String>) -> HashMap<String, HashSet<String>> {
    let sym_set = symbols.iter().cloned().collect::<HashSet<_>>();
    let ast = gcx.read().await.ast_module.clone().unwrap();
    let ast_lock = ast.read().await;
    let mut files_to_syms = HashMap::new();
    for p in paths {
        let path = PathBuf::from(&p);
        let mut doc = Document::new(&path);
        let text = match get_file_text_from_memory_or_disk(gcx.clone(), &path).await {
            Ok(t) => t,
            Err(_) => continue
        };
        doc.update_text(&text);
        let doc_syms = match ast_lock.get_file_symbols(RequestSymbolType::All, &doc).await {
            Ok(x) => x.symbols.into_iter().map(|i|i.name).collect::<HashSet<_>>(),
            Err(_) => continue
        };
        let sym_intersection = sym_set.intersection(&doc_syms).cloned().collect::<HashSet<_>>();
        files_to_syms.insert(p, sym_intersection);
    }
    files_to_syms
}

pub async fn complete_and_filter_paths(gcx: Arc<ARwLock<GlobalContext>>, paths: Vec<String>) -> Vec<String> {
    let project_paths = get_project_paths(gcx.clone()).await;
    let mut files_completed = HashMap::new();
    for p in paths.iter().cloned() {
        if files_completed.contains_key(&p) {
            continue;
        }
        let candidates = file_repair_candidates(gcx.clone(), &p, 3, false).await;
        let real_p = match real_file_path_candidate(gcx.clone(), &p, &candidates, &project_paths, false).await {
            Ok(x) => x,
            Err(_) => continue,
        };
        files_completed.insert(p, real_p);
    }
    let paths = paths.into_iter().filter_map(|f|files_completed.get(&f).cloned()).collect::<Vec<_>>();
    paths
}
