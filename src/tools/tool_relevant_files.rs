use std::collections::HashMap;
use std::string::ToString;
use std::sync::Arc;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use regex::Regex;
use std::path::PathBuf;

use async_trait::async_trait;
use tokio::sync::Mutex as AMutex;
use tokio::sync::RwLock as ARwLock;
use futures_util::future::join_all;

use crate::subchat::subchat;
use crate::tools::tools_description::Tool;

use crate::call_validation::{ChatMessage, ChatUsage, ContextEnum, SubchatParameters, ContextFile};
use crate::global_context::GlobalContext;

use crate::files_in_workspace::{Document, get_file_text_from_memory_or_disk};
use crate::at_commands::at_commands::{AtCommandsContext, vec_context_file_to_context_tools};
use crate::at_commands::at_file::file_repair_candidates;
use crate::ast::ast_mem_db::RequestSymbolType;


use log::info;


pub struct ToolRelevantFiles;

#[async_trait]
impl Tool for ToolRelevantFiles {
    async fn tool_execute(
        &mut self,
        ccx: Arc<AMutex<AtCommandsContext>>,
        tool_call_id: &String,
        args: &HashMap<String, Value>
    ) -> Result<(bool, Vec<ContextEnum>), String> {
        let problem_statement = match args.get("problem_statement") {
            Some(Value::String(s)) => s.clone(),
            Some(v) => return Err(format!("argument `problem_statement` is not a string: {:?}", v)),
            None => return Err("Missing argument `problem_statement`".to_string())
        };

        let params = crate::tools::tools_execute::unwrap_subchat_params(ccx.clone(), "locate").await?;
        let ccx_subchat = {
            let ccx_lock = ccx.lock().await;
            let mut t = AtCommandsContext::new(
                ccx_lock.global_context.clone(),
                params.subchat_n_ctx,
                30,
                false,
                ccx_lock.messages.clone(),
            ).await;
            t.subchat_tx = ccx_lock.subchat_tx.clone();
            t.subchat_rx = ccx_lock.subchat_rx.clone();
            Arc::new(AMutex::new(t))
        };

        let (res, usage, tool_message) = find_relevant_files(
            ccx_subchat,
            params,
            tool_call_id.clone(),
            problem_statement,
        ).await?;

        // cat output
        let (gcx, top_n) = {
            let ccx_lock = ccx.lock().await;
            (ccx_lock.global_context.clone(), ccx_lock.top_n)
        };

        let mut results = vec![];
        if let Value::Object(files) = res {
            let mut refined_res = serde_json::json!({});
            let mut context_files_in = vec![];
            for (file_path, file_info) in files {
                // refine file_path
                let candidates_file = file_repair_candidates(gcx.clone(), &file_path, top_n, false).await;
                if candidates_file.is_empty() {
                    info!("No candidates found for file {}, skip...", file_path);
                    continue;
                }
                if candidates_file.len() > 1 {
                    info!("Found multiple candidates for file {}: {:?}, skip...", file_path, candidates_file);
                    continue;
                }
                let refined_file_path = candidates_file[0].clone();

                // refine symbols
                let mut symbols = vec![];
                if let Some(Value::String(symbols_str)) = file_info.get("SYMBOLS") {
                    if symbols_str != "*" {
                        symbols = symbols_str.split(",").map(|x|x.trim().to_string()).collect::<Vec<_>>()
                    }
                };
                let text = get_file_text_from_memory_or_disk(gcx.clone(), &PathBuf::from(&refined_file_path)).await?.to_string();
                let mut doc = Document::new(&PathBuf::from(&refined_file_path));
                doc.update_text(&text);

                let ast_arc = gcx.read().await.ast_module.clone().unwrap();
                let ast_lock = ast_arc.read().await;
                let doc_syms = ast_lock.get_file_symbols(RequestSymbolType::All, &doc).await?.symbols;
                let symbols_intersection = doc_syms.into_iter().filter(|s|symbols.contains(&s.name)).collect::<Vec<_>>();

                let mut usefulness = 0f32;
                if let Some(relevancy) = file_info.get("RELEVANCY").and_then(Value::as_f64) {
                    usefulness = (relevancy / 5. * 100.) as f32;
                };

                if symbols_intersection.is_empty() {
                    context_files_in.push(ContextFile {
                        file_name: refined_file_path.clone(),
                        file_content: "".to_string(),
                        line1: 0,
                        line2: text.lines().count(),
                        symbols: vec![],
                        gradient_type: -1,
                        usefulness: usefulness,
                        is_body_important: false,
                    });
                }

                let mut symbols_found = vec![];
                for symbol in symbols_intersection {
                    symbols_found.push(symbol.name.clone());
                    context_files_in.push(ContextFile {
                        file_name: refined_file_path.clone(),
                        file_content: "".to_string(),
                        line1: symbol.full_range.start_point.row + 1,
                        line2: symbol.full_range.end_point.row + 1,
                        symbols: vec![symbol.guid.clone()],
                        gradient_type: -1,
                        usefulness: 100.,
                        is_body_important: false,
                    });
                }

                if let Some(refined_res_obj) = refined_res.as_object_mut() {
                    let mut refined_file_info = file_info.clone();
                    refined_file_info["SYMBOLS"] = Value::String(symbols_found.join(","));
                    refined_res_obj.insert(file_path, refined_file_info);
                }
            }

            results.push(ContextEnum::ChatMessage(ChatMessage {
                role: "tool".to_string(),
                content: format!("{}\n\n💿 {}", serde_json::to_string_pretty(&refined_res).unwrap(), tool_message),
                tool_calls: None,
                tool_call_id: tool_call_id.clone(),
                usage: Some(usage),
                ..Default::default()
            }));
            results.extend(vec_context_file_to_context_tools(context_files_in));

        } else {
            results.push(ContextEnum::ChatMessage(ChatMessage {
                role: "tool".to_string(),
                content: format!("{}\n\n💿 {}", serde_json::to_string_pretty(&res).unwrap(), tool_message),
                tool_calls: None,
                tool_call_id: tool_call_id.clone(),
                usage: Some(usage),
                ..Default::default()
            }));
        }

        Ok((false, results))
    }

    fn tool_depends_on(&self) -> Vec<String> {
        vec!["ast".to_string()]
    }
}


const RF_SYSTEM_PROMPT: &str = r###"You are an expert in finding relevant files within a big project. Your job is to find files, don't propose any changes.

Here's the list of reasons a file or symbol might be relevant wrt task description:

TOCHANGE = changes to that file are necessary to complete the task
DEFINITIONS = file has classes/functions/types involved, but no changes needed
HIGHLEV = file is crucial to understand the logic, such as a database scheme, high level script
USERCODE = file has code that uses the things the task description is about
SIMILAR = has code that might provide an example of how to write things similar to elements of the task


Potential strategies:

TREEGUESS = call tree(), spot up to 20 suspicious files just by looking at file names.

GOTODEF = call definition("xxx", skeleton=true) in parallel for symbols either visible in task description, or symbols you can guess; don't call definition() for symbols
from standard libraries, only symbols within the project are indexed.

VECDBSEARCH = call up to five search() in parallel, some good ideas on what to look for: symbols mentioned in the task, one call for each symbol,
strings mentioned, or write imaginary code that does the thing to fix search("    def f():\n        print(\"the example function!\")")


You'll receive additional instructions that start with 💿. Those are not coming from the user, they are programmed to help you operate
well between chat restarts and they are always in English. Answer in the language the user prefers.
"###;

const RF_EXPERT_PLEASE_WRAP_UP: &str = r###"Save your progress, using the following structure:
{
    "OUTPUT": [
        "dir/dir/file.ext": {             // A relative path with no ambiguity at all.
            "SYMBOLS": "symbol1,symbol2", // Comma-separated list of functions/classes/types/variables/etc defined or used within this file that are relevant to given problem. Write "*" to indicate the whole file is necessary. Write "TBD" to indicate you didn't look inside yet.
        }
    ],
}
"###;

const RF_REDUCE_SYSTEM_PROMPT: &str = r###"You will receive outputs generated by experts using different strategies in the following format:

{
  "OUTPUT": {
      "dir/dir/file.ext": {
          "SYMBOLS": "symbol1,symbol2", // Comma-separated list of symbols defined within this file that are actually relevant. "*" might indicate the whole file is necessary.
      }
  ],
  ...
}

Steps you need to follow:

STEP1_CAT: call exact one cat() using given files and symbols. Pass skeleton=True to the cat() call.

STEP2_EXPAND: expand the visible scope by looking up everything necessary to complete the task.

* definitions: which classes and functions are necessary to understand the task? Don't ask about any well-known library functions
or classes like String and Arc in rust, librarires like re, os, subprocess in python, because they are are already well-known and including them
will not help, and libraries are not included in the AST index anyway.

* references: what relevant symbols require looking at usages from outside to fully understand it? If the task is no repair my_function then it's
a good idea to look up usages of my_function.

* similar code: maybe the task is already solved somewhere in the project, write a piece of code that would be required to solve
the problem, and put it into "query" argument of a search(). You can write the entire function if it's not too big. Search also works well for
examples of tricky calls, just write a couple of lines that will be hard to get right.

Examples:
definition("my_method1")
definition("MyClass2")
references("my_method2")
search("    def f():\n        print(\"the example function!\")")
search("    my_object->tricky_call(with, weird, parameters)")

Limits on the number of calls are pretty liberal, 30 definitions, 5 references and 3 searches is a reasonable answer.

Don't explain much, say STEP1_CAT or STEP2_EXPAND depending on which step you are on, and then call the functions.

IT IS FORBIDDEN TO JUST CALL TOOLS WITHOUT EXPLAINING WHICH STEP YOU ARE ON. EXPLAIN FIRST!
"###;

// The convention for methods uses :: delimiter like this Class::method
// references("my_top_level_function3")


const RF_REDUCE_WRAP_UP: &str = r###"
Experts can make mistakes. Your role is to reduce their noisy output into a single more reliable output. Think step by step. Follow this plan:

1. Write down a couple of interpretations of the original task, something like "Interpretation 1: user wants to do this, and the best place to start this change is at file1.ext, near my_function1, in my_function2".
2. Decide which interpretation is most likely correct.
3. Decide which one or two files will receive the most meaningful updates if the user was to change the code in that interpretation. You'll need to label them TOCHANGE later.
4. Write down which files might support the change, some of them contain high-level logic, some have definitions, some similar code.
5. All the files cannot have relevancy 5; most of them are likely 3, "might provide good insight into the logic behind the program but not directly relevant", but you can
write 1 or 2 if you accidentally wrote a file name and changed your mind about how useful it is, not a problem.
6. After you have completed 1-5, go ahead and formalize your best interpretation in the following JSON format, write "REDUCE_OUTPUT", and continue with triple backquotes.

REDUCE_OUTPUT
```
{
    "dir/dir/file.ext": {
        "SYMBOLS": "symbol1,symbol2",     // Comma-separated list of symbols defined within this file that are actually relevant. Use your own judgement, don't copy from experts.
        "WHY_CODE": "string",             // Write down the reason to include this file in output, pick one of: TOCHANGE, DEFINITIONS, HIGHLEV, USERCODE, SIMILAR.
        "WHY_DESC": "string",             // Describe why this file matters wrt the task, what's going on inside? Describe the file in general in a sentense or two, and then describe what specifically is the relation to the task.
        "RELEVANCY": 0                    // Critically evaluate how is this file really relevant to your interpretation of the task. Rate from 1 to 5. 1 = has TBD, role is unclear, 3 = might provide good insight into the logic behind the program but not directly relevant, 5 = exactly what is needed.
    }
}
```
"###;



// REDUCE2 cat(files, symbols, skeleton=True) definition() usage() search() --EXPAND--> definition() usage() search() calls
// EXPAND cat(files, symbols) -> definition() usage() search() calls -> JSON2 files/symbols/RELEVANCY
// Experts make mistakes; take their RELEVANCY ratings critically, and write your own by looking at the actual code and the best interpretation.
// REDUCE2 cat(fles, symbols) definition() usage() search() -> JSON3
// 1. Confirm relevant symbols: look at the files already present in context, and write down all relevant
// Write a very short pseudo code of the most important piece to fix, mentioning classes and functions necessary.The pseudo code from point 1 might help.
// You have to be mindful of the token count, as some files are large. It's essential to
// select specific symbols within a file that are relevant. Another expert will
// pick up your results, likely they will have to only look at symbols selected by you,
// not whole files, because of the space constraints.

// You'll receive additional instructions that start with 💿. Those are not coming from the user, they are programmed to help you operate
// well between chat restarts and they are always in English. Answer in the language the user prefers.

// "WHY_CODE": "string",         // The reason to include this file in expert's output, one of: TOCHANGE, DEFINITIONS, HIGHLEV, USERCODE.
// "WHY_DESC": "string",         // Description why this file matters wrt the task.
// "RELEVANCY": 0                // Expert's own evaluation of their results, 1 to 5. 1 = this file doesn't even exist, 3 = might provide good insight into the logic behind the program but not directly relevant, 5 = exactly what is needed.

// "WHY_CODE": "string",         // Write down the reason to include this file in output, pick one of: TOCHANGE, DEFINITIONS, HIGHLEV, USERCODE. Put TBD if you didn't look inside.
// "WHY_DESC": "string",         // Describe why this file matters wrt the task, what's going on inside? Put TBD if you didn't look inside.
// "RELEVANCY": 0                // Critically evaluate how is this file really relevant to the task. Rate from 1 to 5. 1 = no evidence this file even exists, 2 = file exists but you didn't look inside, 3 = might provide good insight into the logic behind the program but not directly relevant, 5 = exactly what is needed.

fn parse_reduce_output(content: &str) -> Result<Value, String> {
    let re = Regex::new(r"(?s)REDUCE_OUTPUT\s*```(?:json)?\s*(.+?)\s*```").unwrap();
    let json_str = re.captures(content)
        .and_then(|cap| cap.get(1))
        .map(|m| m.as_str().trim())
        .ok_or_else(|| {
            tracing::warn!("Unable to find REDUCE_OUTPUT section:\n{}", content);
            "Unable to find REDUCE_OUTPUT section".to_string()
        })?;
    let output: Value = serde_json::from_str(json_str).map_err(|e| {
            tracing::warn!("Unable to parse JSON:\n{}({})", json_str, e);
            format!("Unable to parse JSON: {:?}", e)
        })?;
    Ok(output)
}


fn update_usage_from_message(usage: &mut ChatUsage, message: &ChatMessage) {
    if let Some(u) = message.usage.as_ref() {
        usage.total_tokens += u.total_tokens;
        usage.completion_tokens += u.completion_tokens;
        usage.prompt_tokens += u.prompt_tokens;
    }
}


#[derive(Serialize, Deserialize, Debug)]
struct ReduceFileItem {
    #[serde(rename = "FILE_PATH")]
    file_path: String,
    #[serde(rename = "SYMBOLS")]
    symbols: String,
    #[serde(rename = "WHY_CODE")]
    why_code: String,
    #[serde(rename = "WHY_DESC")]
    why_desc: String,
    #[serde(rename = "RELEVANCY")]
    relevancy: u8,
}


async fn find_relevant_files(
    ccx: Arc<AMutex<AtCommandsContext>>,
    subchat_params: SubchatParameters,
    tool_call_id: String,
    user_query: String,
) -> Result<(Value, ChatUsage, String), String> {
    let gcx: Arc<ARwLock<GlobalContext>> = ccx.lock().await.global_context.clone();
    let (vecdb_on, workspace_files) = {
        let gcx = gcx.read().await;
        let vecdb = gcx.vec_db.lock().await;
        (vecdb.is_some(), gcx.documents_state.workspace_files.clone())
    };
    let total_files_in_project = workspace_files.lock().unwrap().len();

    let mut usage = ChatUsage { ..Default::default() };
    let log_prefix = chrono::Local::now().format("%Y%m%d-%H%M%S").to_string();

    // STEP experts
    let mut strategy_messages = vec![];
    strategy_messages.push(ChatMessage::new("system".to_string(), RF_SYSTEM_PROMPT.to_string()));
    strategy_messages.push(ChatMessage::new("user".to_string(), user_query.to_string()));

    let mut futures = vec![];

    let mut strategy_tree = strategy_messages.clone();
    strategy_tree.push(
        crate::tools::tool_locate::pretend_tool_call(
            "tree", "{}",
            "💿 I'll use TREEGUESS strategy, to do that I need to start with a tree() call.".to_string()
        )
    );
    futures.push(subchat(
        ccx.clone(),
        subchat_params.subchat_model.as_str(),
        strategy_tree,
        vec![],  // tree strategy doesn't use any tools for now
        0,
        subchat_params.subchat_max_new_tokens,
        RF_EXPERT_PLEASE_WRAP_UP,
        2,
        Some(0.4),
        Some(format!("{log_prefix}-rf-step1-treeguess")),
        Some(tool_call_id.clone()),
        Some(format!("{log_prefix}-rf-step1-treeguess")),
    ));

    let mut strategy_search_tools = vec!["definition", "references"];
    let mut strategy_search = strategy_messages.clone();
    if vecdb_on {
        strategy_search_tools.push("search");
        strategy_search.push(ChatMessage::new("user".to_string(), "💿 Use VECDBSEARCH strategy.".to_string()));
    } else {
        strategy_search.push(ChatMessage::new("user".to_string(), "💿 Use GOTODEF strategy.".to_string()));
    }

    futures.push(subchat(
        ccx.clone(),
        subchat_params.subchat_model.as_str(),
        strategy_search,
        strategy_search_tools.iter().map(|x|x.to_string()).collect::<Vec<_>>(),
        1,
        subchat_params.subchat_max_new_tokens,
        RF_EXPERT_PLEASE_WRAP_UP,
        2,
        Some(0.4),
        Some(format!("{log_prefix}-rf-step1-gotodef")),
        Some(tool_call_id.clone()),
        Some(format!("{log_prefix}-rf-step1-gotodef")),
    ));

    let results: Vec<Vec<Vec<ChatMessage>>> = join_all(futures).await.into_iter().filter_map(|x| x.ok()).collect();

    let mut only_last_messages = Vec::new();
    let mut experts_cnt = 0;
    for choices in results.iter() {
        for messages in choices.iter() {
            if let Some(assistant_msg) = messages.iter().rfind(|msg| msg.role == "assistant").cloned() {
                only_last_messages.push(assistant_msg);
                experts_cnt += 1;
            }
        }
    }

    // TODO dedup files
    let mut files_inspected = 0;
    for choices in results.iter() {
        for messages in choices.iter() {
            let context_file_msgs: Vec<ChatMessage> = messages.iter().filter(|msg| msg.role == "context_file").cloned().collect();
            for msg in context_file_msgs {
                if let Ok(context_files) = serde_json::from_str::<Vec<ContextFile>>(&msg.content) {
                    files_inspected += context_files.len();
                }
            }
        }
    }

    // collect usages from experts
    for message in &only_last_messages {
        update_usage_from_message(&mut usage, &message);
    }

    // | tree() -TREEGUESS-> files x4
    // | search() -VECDBSEARCH-> files and symbols
    // cat(files, symbols, skeleton=True) --EXPAND--> definition() usage() search() calls -REDUCE-> json files/symbols/RELEVANCY

    // STEP expand/reduce
    let mut expand_reduce_tools = vec!["cat", "definition", "references"];
    if vecdb_on {
        expand_reduce_tools.push("search");
    }

    let mut messages = vec![];
    messages.push(ChatMessage::new("system".to_string(), RF_REDUCE_SYSTEM_PROMPT.to_string()));
    messages.push(ChatMessage::new("user".to_string(), format!("User provided task:\n\n{}", user_query)));
    for (i, expert_message) in only_last_messages.into_iter().enumerate() {
        messages.push(ChatMessage::new("user".to_string(), format!("Expert {} says:\n\n{}", i + 1, expert_message.content)));
    }
    messages.push(ChatMessage::new("user".to_string(), "Start your answer with STEP1_CAT".to_string()));

    {
        let mut ccx_locked = ccx.lock().await;
        ccx_locked.correction_only_up_to_step = messages.len() + 1;
    }

    let result = subchat(
        ccx.clone(),
        subchat_params.subchat_model.as_str(),
        messages,
        expand_reduce_tools.iter().map(|x|x.to_string()).collect::<Vec<_>>(),
        1,  // the most controversial one: the chat generates a lot of ref def search tools at STEP2_EXPAND
        subchat_params.subchat_max_new_tokens,
        RF_REDUCE_WRAP_UP,
        1,
        Some(0.0),
        Some(format!("{log_prefix}-rf-step2-reduce")),
        Some(tool_call_id.clone()),
        Some(format!("{log_prefix}-rf-step2-reduce")),
    ).await?[0].clone();

    let last_message = result.last().unwrap();
    let answer = parse_reduce_output(&last_message.content)?;
    update_usage_from_message(&mut usage, &last_message);

    let tool_message = format!("Used {} experts, inspected {} files, project has {} files",
        experts_cnt,
        files_inspected,
        total_files_in_project
    );

    Ok((answer, usage, tool_message))
}
