use std::collections::HashMap;
use std::string::ToString;
use std::sync::Arc;
use hashbrown::HashSet;
use regex::Regex;
use tracing::warn;

use tokio::sync::Mutex as AMutex;
use crate::at_tools::att_locate::locate_prompts::{LOCATE_SYSTEM_PROMPT, STRATEGY_DEF_REF_PROMPT, STRATEGY_TREE_PROMPT, SUPERCAT_EXTRACT_SYMBOLS_PROMPT};
use crate::at_commands::at_commands::AtCommandsContext;
use crate::at_tools::att_locate::locate_utils::{assign_symbols_to_paths, filter_existing_symbols, pretend_tool_call, reduce_by_counter, update_usage};
use crate::at_tools::subchat::subchat_single;
use crate::call_validation::{ChatMessage, ChatUsage};


pub async fn strategy_tree(
    ccx: Arc<AMutex<AtCommandsContext>>,
    model: &String,
    user_query: &str,
    log_prefix: String,
    tool_call_id: String,
    usage: Arc<AMutex<ChatUsage>>,
) -> Result<Vec<String>, String> {
    // results = problem + tool_tree + pick 5 files * n_choices_times -> reduce(counters: 5)

    let mut messages = vec![];
    messages.push(ChatMessage::new("system".to_string(), LOCATE_SYSTEM_PROMPT.to_string()));
    messages.push(ChatMessage::new("user".to_string(), user_query.to_string()));

    messages.push(pretend_tool_call("tree", "{}"));
    let mut usage_collector = ChatUsage::default();
    let mut messages = subchat_single(
        ccx.clone(),
        model,
        messages,
        vec!["tree".to_string()],
        None,
        true,
        None,
        None,
        1,
        Some(&mut usage_collector),
        None,
        Some(tool_call_id.clone()),
        Some(format!("{log_prefix}-locate-step1-tree")),
    ).await?.get(0).ok_or("relevant_files: tree deterministic message was empty. Try again later".to_string())?.clone();

    messages.push(ChatMessage::new("user".to_string(), STRATEGY_TREE_PROMPT.to_string()));

    let n_choices = subchat_single(
        ccx.clone(),
        model,
        messages,
        vec![],
        Some("none".to_string()),
        false,
        Some(0.8),
        None,
        5,
        Some(&mut usage_collector),
        Some(format!("{log_prefix}-locate-step1-tree-result")),
        Some(tool_call_id.clone()),
        Some(format!("{log_prefix}-locate-step1-tree-result")),
    ).await?;

    assert_eq!(n_choices.len(), 5);

    let file_names_pattern = r"\b(?:[a-zA-Z]:\\|/)?(?:[\w-]+[/\\])*[\w-]+\.\w+\b";
    let re = Regex::new(file_names_pattern).unwrap();

    let filenames = n_choices.into_iter()
        .filter_map(|mut x| x.pop())
        .filter(|x| x.role == "assistant")
        .map(|x| {
            re.find_iter(&x.content)
                .map(|mat| mat.as_str().to_string())
                .collect::<Vec<_>>()
        })
        .collect::<Vec<Vec<_>>>();

    let results = reduce_by_counter(filenames.into_iter().flatten(), 10);

    update_usage(usage, &mut usage_collector).await;

    Ok(results)
}

pub async fn strategy_symbols_from_problem_text(
    ccx: Arc<AMutex<AtCommandsContext>>,
    model: &String,
    user_query: &str,
    log_prefix: String,
    tool_call_id: String,
    usage: Arc<AMutex<ChatUsage>>,
) -> Result<(HashMap<String, HashSet<String>>), String>{
    // todo: Maybe split whitespace + intersection would be better?
    // results = problem -> (collect definitions + references) * n_choices + map(into_filenames) -> reduce(counters: 5)
    let mut messages = vec![];
    messages.push(ChatMessage::new("system".to_string(), LOCATE_SYSTEM_PROMPT.to_string()));
    messages.push(ChatMessage::new("user".to_string(), user_query.to_string()));
    messages.push(ChatMessage::new("user".to_string(), STRATEGY_DEF_REF_PROMPT.to_string()));

    let mut usage_collector = ChatUsage::default();

    let n_choices = subchat_single(
        ccx.clone(),
        model,
        messages,
        vec![],
        None,
        false,
        Some(0.8),
        None,
        5,
        Some(&mut usage_collector),
        Some(format!("{log_prefix}-locate-step2-defs-refs")),
        Some(tool_call_id.clone()),
        Some(format!("{log_prefix}-locate-step2-defs-refs")),
    ).await?;

    let mut symbols = vec![];
    for msg in n_choices.into_iter().map(|x|x.last().unwrap().clone()).filter(|x|x.role == "assistant") {
        let ch_symbols: Vec<String> = match serde_json::from_str(&msg.content) {
            Ok(x) => x,
            Err(_) => { continue; }
        };
        symbols.extend(ch_symbols);
    }

    let gcx = ccx.lock().await.global_context.clone();
    let symbols = filter_existing_symbols(gcx.clone(), symbols).await?;
    let top_symbols = reduce_by_counter(symbols.into_iter(), 15);
    
    let mut symbols_and_paths = HashMap::new();
    {
        let ast = {
            let cx = gcx.read().await;
            cx.ast_module.clone().unwrap()
        };
        let ast_lock = ast.read().await;
        for s in top_symbols.iter() {
            let declarations = match ast_lock.search_declarations(s.clone()).await {
                Ok(x) => x,
                Err(e) => {
                    warn!(e);
                    continue;
                }
            };
            for r in declarations.exact_matches {
                let path = r.symbol_declaration.file_path.to_string_lossy().to_string();
                symbols_and_paths.entry(s.clone()).or_insert(HashSet::new()).insert(path);
            }
        }
    }

    update_usage(usage, &mut usage_collector).await;

    Ok(symbols_and_paths)
}

pub async fn cat_extract_symbols(
    ccx: Arc<AMutex<AtCommandsContext>>,
    model: &String,
    user_query: &str,
    files: Vec<String>,
    symbols: Vec<String>,
    log_prefix: String,
    tool_call_id: String,
    usage: &mut ChatUsage,
) -> Result<HashMap<String, HashSet<String>>, String> {
    // todo: move to a function
    let mut messages = vec![];
    messages.push(ChatMessage::new("system".to_string(), LOCATE_SYSTEM_PROMPT.to_string()));
    messages.push(ChatMessage::new("user".to_string(), user_query.to_string()));

    let mut args = HashMap::new();
    args.insert("paths".to_string(), files.join(","));
    args.insert("skeleton".to_string(), "true".to_string());
    if !symbols.is_empty() {
        args.insert("symbols".to_string(), symbols.join(","));
    }

    messages.push(pretend_tool_call(
        "cat",
        serde_json::to_string(&args).unwrap().as_str()
    ));

    let mut messages = subchat_single(
        ccx.clone(),
        model,
        messages,
        vec!["cat".to_string()],
        None,
        true,
        None,
        None,
        1,
        Some(usage),
        None,
        Some(tool_call_id.clone()),
        Some(format!("{log_prefix}-locate-step3-cat")),
    ).await?.get(0).ok_or("locate: cat message was empty.".to_string())?.clone();

    messages.push(ChatMessage::new("user".to_string(), SUPERCAT_EXTRACT_SYMBOLS_PROMPT.replace("{USER_QUERY}", &user_query)));

    let n_choices = subchat_single(
        ccx.clone(),
        model,
        messages,
        vec![],
        Some("none".to_string()),
        false,
        Some(0.8),
        None,
        5,
        Some(usage),
        Some(format!("{log_prefix}-locate-step3-cat-result")),
        Some(tool_call_id.clone()),
        Some(format!("{log_prefix}-locate-step3-cat-result")),
    ).await?;

    assert_eq!(n_choices.len(), 5);

    let mut extra_symbols = vec![];
    for msg in n_choices.into_iter().map(|x|x.last().unwrap().clone()).filter(|x|x.role == "assistant") {
        let ch_symbols: Vec<String> = match serde_json::from_str(&msg.content) {
            Ok(x) => x,
            Err(_) => { continue; }
        };
        extra_symbols.extend(ch_symbols);
    }
    let gcx = ccx.lock().await.global_context.clone();
    let extra_symbols = filter_existing_symbols(gcx.clone(), extra_symbols.clone()).await?;
    let top_symbols = reduce_by_counter(extra_symbols.into_iter(), 15);
    
    let new_symbols = symbols.into_iter().chain(top_symbols.into_iter()).collect::<Vec<_>>();
    let files_to_syms = assign_symbols_to_paths(gcx.clone(), new_symbols, files).await;
    
    Ok(files_to_syms)
}
