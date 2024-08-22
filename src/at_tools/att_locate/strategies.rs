use std::collections::HashMap;
use std::string::ToString;
use std::sync::Arc;
use hashbrown::HashSet;
use regex::Regex;

use tokio::sync::Mutex as AMutex;
use crate::ast::ast_index::RequestSymbolType;
use crate::at_tools::att_locate::prompts::{STEP1_DET_SYSTEM_PROMPT, STRATEGY_DEF_REF_PROMPT, STRATEGY_TREE_PROMPT, SUPERCAT_EXTRACT_SYMBOLS_PROMPT};
use crate::at_commands::at_commands::AtCommandsContext;
use crate::at_tools::att_locate::locate_utils::{pretend_tool_call, reduce_by_counter, update_usage};
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
    messages.push(ChatMessage::new("system".to_string(), STEP1_DET_SYSTEM_PROMPT.to_string()));
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
) -> Result<(Vec<String>, Vec<String>), String>{
    // results = problem -> (collect definitions + references) * n_choices + map(into_filenames) -> reduce(counters: 5)
    let mut messages = vec![];
    messages.push(ChatMessage::new("system".to_string(), STEP1_DET_SYSTEM_PROMPT.to_string()));
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
    for ch_messages in n_choices.into_iter() {
        if let Some(answer) = ch_messages.last().filter(|x|x.role == "assistant").map(|x|x.content.clone()) {
            let ch_symbols: Vec<String> = match serde_json::from_str(&answer) {
                Ok(x) => x,
                Err(_) => { continue; }
            };
            symbols.extend(ch_symbols);
        }
    }

    let gcx = ccx.lock().await.global_context.clone();
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
    let top_symbols = reduce_by_counter(filtered_symbols.into_iter(), 15);
    
    
    
    // todo: call definitions to the symbols (and references?) to attach list of files to each symbol
    update_usage(usage, &mut usage_collector).await;

    todo!();
    Ok((vec![], vec![]))
}

pub async fn supercat_extract_symbols(
    ccx: Arc<AMutex<AtCommandsContext>>,
    model: &String,
    user_query: &str,
    files: Vec<String>,
    symbols: Vec<String>,
    log_prefix: String,
    tool_call_id: String,
    usage: &mut ChatUsage,
) -> Result<Vec<String>, String> {
    let mut messages = vec![];
    messages.push(ChatMessage::new("system".to_string(), STEP1_DET_SYSTEM_PROMPT.to_string()));
    messages.push(ChatMessage::new("user".to_string(), user_query.to_string()));

    let mut supercat_args = HashMap::new();
    supercat_args.insert("paths".to_string(), files.join(","));
    supercat_args.insert("skeleton".to_string(), "true".to_string());
    if !symbols.is_empty() {
        supercat_args.insert("symbols".to_string(), symbols.join(","));
    }

    messages.push(pretend_tool_call(
        "cat",
        serde_json::to_string(&supercat_args).unwrap().as_str()
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
    ).await?.get(0).ok_or("relevant_files: cat message was empty.".to_string())?.clone();

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

    let mut symbols_result = vec![];
    for msg in n_choices.into_iter().map(|x|x.last().unwrap().clone()).filter(|x|x.role == "assistant") {
        let symbols = {
            let re = Regex::new(r"[^,\s]+").unwrap();
            re.find_iter(&msg.content)
                .map(|mat| mat.as_str().to_string())
                .collect::<Vec<_>>()
        };
        symbols_result.push(symbols);
    }

    let results = reduce_by_counter(symbols_result.into_iter().flatten(), 15);

    Ok(results)
}
