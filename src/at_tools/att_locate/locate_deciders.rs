use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::Mutex as AMutex;

use crate::at_commands::at_commands::AtCommandsContext;
use crate::at_tools::att_locate::locate_prompts::{CAT_FILE_TO_CHANGE_PROMPT, CAT_REDUCE_SYMBOLS_PROMPT, CAT_REDUCE_TO_CHANGE_PROMPT, LOCATE_SYSTEM_PROMPT};
use crate::at_tools::att_locate::locate_utils::{complete_and_filter_paths, pretend_tool_call, reduce_by_counter, update_usage};
use crate::at_tools::subchat::{subchat_single, write_dumps};
use crate::call_validation::{ChatMessage, ChatUsage};


pub async fn decide_symbols_list(
    ccx: Arc<AMutex<AtCommandsContext>>,
    model: &String,
    user_query: &str,
    files: Vec<String>,
    symbols: Vec<String>,
    tool_call_id: String,
    usage: Arc<AMutex<ChatUsage>>,
) -> Result<Vec<String>, String> {
    let mut usage_collector = ChatUsage::default();
    let gcx = ccx.lock().await.global_context.clone();
    
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
    drop(args);

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
        Some(&mut usage_collector),
        Some("locate-decide_symbols_list-1".to_string()),
        Some(tool_call_id.clone()),
        Some("locate-decide_symbols_list-1".to_string()),
    ).await?.get(0).ok_or("locate: decide_symbols_list-1 was empty.".to_string())?.clone();

    messages.push(ChatMessage::new(
        "user".to_string(), 
        CAT_REDUCE_SYMBOLS_PROMPT
            .replace("{USER_QUERY}", &user_query)
            .replace("{PROPOSED_SYMBOLS}", &symbols.join("\n"))
    ));

    let messages = subchat_single(
        ccx.clone(),
        model,
        messages,
        vec![],
        None,
        false,
        None,
        None,
        1,
        Some(&mut usage_collector),
        Some("locate-decide_symbols_list-2".to_string()),
        Some(tool_call_id.clone()),
        Some("locate-decide_symbols_list-2".to_string()),
    ).await?.get(0).ok_or("locate: decide_symbols_list-2 was empty.".to_string())?.clone();
    
    let answer = messages.last().filter(|x|x.role == "assistant").map(|x|x.content.clone())
        .ok_or("locate: decide_symbols_list-2 last message was empty".to_string())?;
    let new_symbols: Vec<String> = serde_json::from_str(&answer)
       .map_err(|_| "locate: decide_symbols_list-2 could not parse json".to_string())?;
    
    let new_symbols = new_symbols.into_iter().filter(|x|symbols.contains(x)).collect::<Vec<_>>();

    write_dumps(gcx.clone(), "decide_symbols_list-result.log".to_string(), &serde_json::to_string_pretty(&new_symbols).unwrap()).await;
    update_usage(usage, &mut usage_collector).await;

    Ok(new_symbols)
}

pub async fn decide_files_to_change(
    ccx: Arc<AMutex<AtCommandsContext>>,
    model: &String,
    user_query: &str,
    files: Vec<String>,
    symbols: Vec<String>,
    tool_call_id: String,
    usage: Arc<AMutex<ChatUsage>>,
    max_files_to_change: usize,
) -> Result<Vec<String>, String> {
    // todo: idea for decider: decide for each file separately: N calls vote (T/N): useful / not useful
    let mut usage_collector = ChatUsage::default();
    let gcx = ccx.lock().await.global_context.clone();

    let files_to_change = top_files_to_change(
        ccx.clone(), model, user_query, files.clone(), symbols.clone(), tool_call_id.clone(), &mut usage_collector
    ).await?;
    if files_to_change.is_empty() {
        return Err("No files to change found: top_files_to_change produced an empty vec".to_string());
    }
    if files_to_change.len() == 1 {
        return Ok(files_to_change);
    }

    let mut messages = vec![];
    messages.push(ChatMessage::new("system".to_string(), LOCATE_SYSTEM_PROMPT.to_string()));
    messages.push(ChatMessage::new("user".to_string(), user_query.to_string()));

    let mut args = HashMap::new();
    args.insert("paths".to_string(), files_to_change.join(","));
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
        Some(&mut usage_collector),
        Some("locate-decide_files_to_change-1".to_string()),
        Some(tool_call_id.clone()),
        Some("locate-decide_files_to_change-1".to_string()),
    ).await?.get(0).ok_or("locate: decide_files_to_change-1 was empty.".to_string())?.clone();

    messages.push(ChatMessage::new(
        "user".to_string(), 
        CAT_REDUCE_TO_CHANGE_PROMPT
            .replace("{USER_QUERY}", &user_query)
            .replace("{MAX_FILES}", &format!("{}", max_files_to_change))
    ));

    let messages = subchat_single(
        ccx.clone(),
        model,
        messages,
        vec![],
        None,
        false,
        None,
        None,
        1,
        Some(&mut usage_collector),
        Some("locate-decide_files_to_change-2".to_string()),
        Some(tool_call_id.clone()),
        Some("locate-decide_files_to_change-2".to_string()),
    ).await?.get(0).ok_or("locate: decide_files_to_change-2 was empty.".to_string())?.clone();

    let answer = messages.last().filter(|x|x.role == "assistant").map(|x|x.content.clone())
        .ok_or("locate: decide_files_to_change-2 last message was empty".to_string())?;
    let paths: Vec<String> = serde_json::from_str(&answer)
        .map_err(|_| "locate: decide_files_to_change-2 could not parse json".to_string())?;
    let paths = paths.into_iter().take(max_files_to_change).collect::<Vec<_>>();

    write_dumps(gcx.clone(), "decide_files_to_change-result.log".to_string(), &serde_json::to_string_pretty(&paths).unwrap()).await;

    update_usage(usage, &mut usage_collector).await;
    
    Ok(paths)
}

async fn top_files_to_change(
    ccx: Arc<AMutex<AtCommandsContext>>,
    model: &String,
    user_query: &str,
    files: Vec<String>,
    symbols: Vec<String>,
    tool_call_id: String,
    usage: &mut ChatUsage,
) -> Result<Vec<String>, String> {
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
    drop(args);

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
        Some("locate-top_files_to_change-1".to_string()),
        Some(tool_call_id.clone()),
        Some("locate-top_files_to_change-1".to_string()),
    ).await?.get(0).ok_or("locate: top_files_to_change-1 was empty.".to_string())?.clone();

    messages.push(ChatMessage::new("user".to_string(), CAT_FILE_TO_CHANGE_PROMPT.replace("{USER_QUERY}", &user_query)));

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
        Some("locate-top_files_to_change-2".to_string()),
        Some(tool_call_id.clone()),
        Some("locate-top_files_to_change-2".to_string()),
    ).await?;

    assert_eq!(n_choices.len(), 5);

    let mut files_to_change = vec![];
    for ch_res in n_choices.into_iter()
        .filter_map(|x| x.last().cloned())
        .filter(|x| x.role == "assistant")
        .map(|x| x.content.clone()) {
        let ch_files_change: Vec<String> = match serde_json::from_str(&ch_res) {
            Ok(x) => x,
            Err(_) => continue,
        };
        files_to_change.extend(ch_files_change);
    }
    
    let gcx = ccx.lock().await.global_context.clone();
    let files_to_change = complete_and_filter_paths(gcx.clone(), files_to_change).await;
    let top_files_to_change = reduce_by_counter(files_to_change.into_iter(), 7);

    write_dumps(gcx.clone(), "top_files_to_change-result.log".to_string(), &serde_json::to_string_pretty(&top_files_to_change).unwrap()).await;

    Ok(top_files_to_change)
}
