use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::Arc;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use async_trait::async_trait;
use tokio::sync::Mutex as AMutex;
use tracing::info;

use crate::at_tools::att_locate::prompts::{STEP1_DET_SYSTEM_PROMPT, SUPERCAT_DECIDER_PROMPT, SUPERCAT_REDUCE_TO_CHANGE};
use crate::at_commands::at_commands::AtCommandsContext;
use crate::at_tools::att_locate::locate_utils::{pretend_tool_call, reduce_by_counter, update_usage};
use crate::at_tools::att_locate::strategies::{strategy_symbols_from_problem_text, strategy_tree, supercat_extract_symbols};
use crate::at_tools::subchat::subchat_single;
use crate::at_tools::tools::Tool;
use crate::call_validation::{ChatMessage, ChatUsage, ContextEnum, SubchatParameters};
use crate::caps::get_model_record;
use crate::toolbox::toolbox_config::load_customization;


pub struct AttLocate;


#[derive(Serialize, Deserialize, Debug)]
struct SuperCatResultItem {
    file_path: String,
    reason: String,
    description: String,
}


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

#[async_trait]
impl Tool for AttLocate{
    async fn tool_execute(
        &mut self,
        ccx: Arc<AMutex<AtCommandsContext>>,
        tool_call_id: &String,
        args: &HashMap<String, Value>
    ) -> Result<Vec<ContextEnum>, String> {
        
        let problem_statement_summary = match args.get("problem_statement") {
            Some(Value::String(s)) => s.clone(),
            Some(v) => return Err(format!("argument `problem_statement` is not a string: {:?}", v)),
            None => return Err("Missing argument `problem_statement`".to_string())
        };

        let params = unwrap_subchat_params(ccx.clone(), "locate").await?;
        let ccx_subchat = {
            let ccx_lock = ccx.lock().await;
            Arc::new(AMutex::new(AtCommandsContext::new(
                ccx_lock.global_context.clone(),
                params.n_ctx,
                30,
                false,
                ccx_lock.messages.clone(),
            ).await))
        };

        let problem_message_mb = {
            let ccx_locked = ccx_subchat.lock().await;
            ccx_locked.messages.iter().filter(|m| m.role == "user").last().map(|x|x.content.clone())
        };

        let mut problem_statement = format!("Problem statement:\n{}", problem_statement_summary);
        if let Some(problem_message) = problem_message_mb {
            problem_statement = format!("{}\n\nProblem described by user:\n{}", problem_statement, problem_message);
        }

        let usage = Arc::new(AMutex::new(ChatUsage::default()));
        let res = locate_relevant_files(ccx_subchat.clone(), &params.model, problem_statement.as_str(), tool_call_id.clone(), usage.clone()).await?;
        let usage_values = usage.lock().await.clone();
        info!("att_locate produced usage: {:?}", usage_values);

        let mut results = vec![];
        results.push(ContextEnum::ChatMessage(ChatMessage {
            role: "tool".to_string(),
            content: format!("{}", serde_json::to_string_pretty(&res).unwrap()),
            tool_calls: None,
            tool_call_id: tool_call_id.clone(),
            usage: Some(usage_values),
        }));

        Ok(results)
    }
    fn tool_depends_on(&self) -> Vec<String> {
        vec!["ast".to_string()]
    }
}

async fn locate_relevant_files(
    ccx: Arc<AMutex<AtCommandsContext>>,
    model: &String,
    user_query: &str,
    tool_call_id: String,
    usage: Arc<AMutex<ChatUsage>>,
) -> Result<Value, String> {
    let log_prefix = chrono::Local::now().format("%Y%m%d-%H%M%S").to_string();
    let mut paths_chosen = vec![];

    let tree_files_future = strategy_tree(
        ccx.clone(),
        model,
        user_query,
        log_prefix.clone(),
        tool_call_id.clone(),
        usage.clone(),
    );
    let def_ref_future = strategy_symbols_from_problem_text(
        ccx.clone(),
        model,
        user_query,
        log_prefix.clone(),
        tool_call_id.clone(),
        usage.clone(),
    );
    let (tree_files, (def_ref_files, mut symbols)) = tokio::try_join!(tree_files_future, def_ref_future)?;

    let mut usage_collector = ChatUsage::default();
    
    paths_chosen.extend(tree_files);
    paths_chosen.extend(def_ref_files);

    let extra_symbols = supercat_extract_symbols(
        ccx.clone(),
        model,
        user_query,
        paths_chosen.clone(),
        symbols.clone(),
        log_prefix.clone(),
        tool_call_id.clone(),
        &mut usage_collector,
    ).await?;

    symbols.extend(extra_symbols);
    let symbols = symbols.into_iter().collect::<HashSet<_>>().into_iter().collect::<Vec<_>>();

    let file_results = supercat_decider(
        ccx.clone(),
        model,
        user_query,
        paths_chosen,
        symbols.clone(),
        log_prefix.clone(),
        tool_call_id.clone(),
        &mut usage_collector,
    ).await?;

    let results_dict = json!({
        "files": file_results,
        "symbols": symbols,
    });

    update_usage(usage, &mut usage_collector).await;

    Ok(results_dict)
}

async fn supercat_decider(
    ccx: Arc<AMutex<AtCommandsContext>>,
    model: &String,
    user_query: &str,
    files: Vec<String>,
    symbols: Vec<String>,
    log_prefix: String,
    tool_call_id: String,
    usage: &mut ChatUsage,
) -> Result<Value, String> {
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
        Some(format!("{log_prefix}-locate-step4-det")),
    ).await?.get(0).ok_or("locate: cat message was empty.".to_string())?.clone();

    messages.push(ChatMessage::new("user".to_string(), SUPERCAT_DECIDER_PROMPT.replace("{USER_QUERY}", &user_query)));

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
        Some(format!("{log_prefix}-locate-step4-det-result")),
        Some(tool_call_id.clone()),
        Some(format!("{log_prefix}-locate-step4-det-result")),
    ).await?;

    assert_eq!(n_choices.len(), 5);

    let mut results_to_change = vec![];
    let mut results_context = vec![];
    let mut file_descriptions: HashMap<String, HashSet<String>> = HashMap::new();

    for ch_messages in n_choices {
        let answer_mb = ch_messages.last().filter(|x|x.role == "assistant").map(|x|x.content.clone());
        if answer_mb.is_none() {
            continue;
        }
        let answer = answer_mb.unwrap();
        let results: Vec<SuperCatResultItem> = match serde_json::from_str(&answer) {
            Ok(x) => x,
            Err(_) => continue
        };

        for r in results.iter() {
            file_descriptions.entry(r.file_path.clone()).or_insert(HashSet::new()).insert(r.description.clone());
        }

        let to_change = results.iter().filter(|x|x.reason == "to_change").map(|x|x.file_path.clone()).collect::<Vec<_>>();
        if to_change.is_empty() {
            continue;
        }
        results_to_change.push(to_change);

        let context = results.iter().filter(|x|x.reason == "context").map(|x|x.file_path.clone()).collect::<Vec<_>>();
        results_context.push(context);
    }

    let files_to_change = reduce_by_counter(results_to_change.into_iter().flatten().filter(|x|PathBuf::from(x).is_file()), 5);
    let files_context = reduce_by_counter(results_context.into_iter().flatten().filter(|x|PathBuf::from(x).is_file()), 5);

    let mut res_to_change = vec![];
    res_to_change.extend(
        files_to_change.into_iter().map(|x| SuperCatResultItem{
            file_path: x.clone(),
            reason: "to_change".to_string(),
            description: file_descriptions.get(&x).unwrap_or(&HashSet::new()).into_iter().cloned().collect::<Vec<_>>().join(", "),
        })
    );

    let mut res_context = vec![];
    res_context.extend(
        files_context.into_iter().map(|x| SuperCatResultItem{
            file_path: x.clone(),
            reason: "context".to_string(),
            description: file_descriptions.get(&x).unwrap_or(&HashSet::new()).into_iter().cloned().collect::<Vec<_>>().join(", "),
        })
    );


    let mut supercat_args = HashMap::new();
    supercat_args.insert("paths".to_string(), res_to_change.into_iter().map(|x|x.file_path).collect::<Vec<_>>().join(","));
    supercat_args.insert("skeleton".to_string(), "true".to_string());
    if !symbols.is_empty() {
        supercat_args.insert("symbols".to_string(), symbols.join(","));
    }

    let mut messages = vec![];
    messages.push(ChatMessage::new("system".to_string(), STEP1_DET_SYSTEM_PROMPT.to_string()));
    messages.push(ChatMessage::new("user".to_string(), user_query.to_string()));

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
        Some(format!("{log_prefix}-locate-step4-det2")),
    ).await?.get(0).ok_or("locate: cat message was empty.".to_string())?.clone();

    messages.push(ChatMessage::new("user".to_string(), SUPERCAT_REDUCE_TO_CHANGE.replace("{USER_QUERY}", &user_query)));

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
        Some(usage),
        None,
        Some(tool_call_id.clone()),
        Some(format!("{log_prefix}-locate-step4-reduce-to-change")),
    ).await?.get(0).ok_or("locate: locate-step4-reduce-to-change was empty".to_string())?.clone();

    let answer = messages.last().filter(|x|x.role == "assistant").map(|x|x.content.clone()).ok_or("locate: locate-step4-reduce-to-change last message was empty".to_string())?;

    let results: Vec<SuperCatResultItem> = serde_json::from_str(&answer).map_err(|x|x.to_string())?;

    let res = results.into_iter().chain(res_context.into_iter()).collect::<Vec<_>>();

    Ok(json!(res))
}
