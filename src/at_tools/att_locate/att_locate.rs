use std::collections::HashMap;
use hashbrown::HashSet;
use std::sync::Arc;
use serde_json::{json, Value};

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use tokio::sync::Mutex as AMutex;
use tracing::info;

use crate::at_commands::at_commands::AtCommandsContext;
use crate::at_tools::att_locate::locate_deciders::{decide_files_to_change, decide_symbols_list};
use crate::at_tools::att_locate::locate_utils::{assign_symbols_to_paths, complete_and_filter_paths, unwrap_subchat_params, update_usage};
use crate::at_tools::att_locate::locate_strategies::{strategy_symbols_from_problem_text, strategy_tree, cat_extract_symbols};
use crate::at_tools::tools::Tool;
use crate::call_validation::{ChatMessage, ChatUsage, ContextEnum};

// may be overridden using hidden arg "max_files_to_change"
const MAX_FILES_TO_CHANGE: usize = 5;


#[derive(Serialize, Deserialize, Debug)]
struct CatResultPathItem {
    file_path: String,
    reason: String,
    description: String,
}

pub struct AttLocate;

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
        // hidden arg
        let max_files_to_change = match args.get("max_files_to_change") {
            Some(Value::Number(n)) => n.as_u64().unwrap() as usize,
            Some(v) => return Err(format!("argument `max_files_to_change` is not a number: {:?}", v)),
            None => MAX_FILES_TO_CHANGE
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
        let res = locate_relevant_files(
            ccx_subchat.clone(), 
            &params.model, 
            problem_statement.as_str(), 
            tool_call_id.clone(), 
            usage.clone(),
            max_files_to_change
        ).await?;
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
    max_files_to_change: usize,
) -> Result<Value, String> {
    // todo: not files to change, but symbols to change (?)
    let gcx = ccx.lock().await.global_context.clone();

    let tree_files_future = strategy_tree(
        ccx.clone(),
        model,
        user_query,
        tool_call_id.clone(),
        usage.clone(),
    );
    let def_ref_future = strategy_symbols_from_problem_text(
        ccx.clone(),
        model,
        user_query,
        tool_call_id.clone(),
        usage.clone(),
    );
    
    let (tree_files, symbols_and_paths) = tokio::try_join!(tree_files_future, def_ref_future)?;

    let mut usage_collector = ChatUsage::default();
    // tree_paths + symbols_paths
    let paths = tree_files.iter().chain(symbols_and_paths.values().flatten()).cloned().collect::<HashSet<_>>().into_iter().collect::<Vec<_>>();
    let symbols = symbols_and_paths.keys().cloned().collect::<Vec<_>>();
    drop(symbols_and_paths);
    
    let paths = complete_and_filter_paths(gcx.clone(), paths).await;

    let cat_symbols = cat_extract_symbols(
        ccx.clone(),
        model,
        user_query,
        paths.clone(),
        symbols.clone(),
        tool_call_id.clone(),
        &mut usage_collector,
    ).await?;

    let all_symbols = cat_symbols.into_iter().chain(symbols.into_iter()).collect::<HashSet<_>>().into_iter().collect::<Vec<_>>();
    let mut files_to_symbols = assign_symbols_to_paths(gcx.clone(), all_symbols, paths).await;
    
    let files_to_change_future = decide_files_to_change(
        ccx.clone(),
        model,
        user_query,
        files_to_symbols.keys().cloned().collect::<Vec<_>>(),
        files_to_symbols.values().flatten().cloned().collect::<Vec<_>>(),
        tool_call_id.clone(),
        usage.clone(),
        max_files_to_change,
    );
    
    let chosen_symbols_future = decide_symbols_list(
        ccx.clone(),
        model,
        user_query,
        files_to_symbols.keys().cloned().collect::<Vec<_>>(),
        files_to_symbols.values().flatten().cloned().collect::<HashSet<_>>().into_iter().collect::<Vec<_>>(),
        tool_call_id.clone(),
        usage.clone(),
    );
    
    let (files_to_change, chosen_symbols) = tokio::try_join!(files_to_change_future, chosen_symbols_future)?;
    let chosen_symbols_set = chosen_symbols.into_iter().collect::<HashSet<_>>();
    // remove files from list if no symbols were chosen
    files_to_symbols.retain(|_, v| {
        *v = v.intersection(&chosen_symbols_set).cloned().collect::<HashSet<_>>();
        !v.is_empty()
    });
    
    let context_files = files_to_symbols.iter().filter(|(k, _)|!files_to_change.contains(k)).map(|(k, _)|k).cloned().collect::<Vec<_>>();
    
    let result_files = context_files.into_iter().map(|x|CatResultPathItem {
        file_path: x,
        reason: "context".to_string(),
        description: "".to_string(),
    }).chain(files_to_change.into_iter().map(|x|CatResultPathItem {
        file_path: x,
        reason: "to_change".to_string(),
        description: "".to_string(),
    })).collect::<Vec<_>>();

    update_usage(usage, &mut usage_collector).await;
    
    let results_dict = json!({
        "files": result_files,
        "symbols": files_to_symbols.values().flatten().cloned().collect::<HashSet<_>>().into_iter().collect::<Vec<_>>()
    });

    Ok(results_dict)
}
