use std::collections::HashMap;
use std::sync::Arc;
use async_trait::async_trait;
use tokio::sync::RwLock as ARwLock;
use serde::{Serialize, Deserialize};
use serde_json::Value;
use crate::at_commands::at_commands::AtCommandsContext;
use crate::at_tools::subchat::{execute_subchat, execute_subchat_single_iteration};
use crate::at_tools::tools::Tool;
use crate::call_validation::{ChatMessage, ContextEnum};
use crate::global_context::GlobalContext;


pub struct AttRelevantFiles;

#[async_trait]
impl Tool for AttRelevantFiles {
    async fn tool_execute(&mut self, ccx: &mut AtCommandsContext, tool_call_id: &String, _args: &HashMap<String, Value>) -> Result<Vec<ContextEnum>, String> {
        let problem = ccx.messages.iter().filter(|m| m.role == "user").last().map(|x|x.content.clone()).ok_or(
            "execute_subchat: unable to find user problem description".to_string()
        )?;

        let res = find_relevant_files(ccx.global_context.clone(), problem.as_str()).await?;
        let relevant_files = res.output.keys().cloned().collect::<Vec<String>>();

        let mut results = vec![];
        results.push(ContextEnum::ChatMessage(ChatMessage {
            role: "tool".to_string(),
            content: format!("Found {} relevant files:\n{}", relevant_files.len(), relevant_files.join("\n")),
            tool_calls: None,
            tool_call_id: tool_call_id.clone(),
            ..Default::default()
        }));

        Ok(results)
    }
    fn tool_depends_on(&self) -> Vec<String> {
        vec!["ast".to_string(), "vecdb".to_string()]
    }
}


const RF_OUTPUT_FILES: usize = 6;
const RF_ATTEMPTS: usize = 1;
const RF_WRAP_UP_DEPTH: usize = 5;
const RF_WRAP_UP_TOKENS_CNT: usize = 8000;


async fn find_relevant_files(
    gcx: Arc<ARwLock<GlobalContext>>,
    user_query: &str,
) -> Result<PleaseWriteMem, String> {
    let sys = RF_SYSTEM_PROMPT
        .replace("{RF_ATTEMPTS}", &format!("{}", RF_ATTEMPTS))
        .replace("{RF_OUTPUT_FILES}", &format!("{}", RF_OUTPUT_FILES));

    let mut messages = vec![];
    messages.push(ChatMessage::new("system".to_string(), sys.to_string()));
    messages.push(ChatMessage::new("user".to_string(), user_query.to_string()));

    let tools_turn_on = vec!["definition", "references", "tree", "knowledge", "file", "search"].iter().map(|x|x.to_string()).collect();

    // for strategy:
        let mut sub_conversation = execute_subchat(
            gcx.clone(),
            "gpt-4o-mini",
            &messages,
            &tools_turn_on,
            RF_WRAP_UP_DEPTH,
            RF_WRAP_UP_TOKENS_CNT,
            // result prompt
        ).await?;  // don't wait

    // wait all strategies

    // move into execute_subchat
    sub_conversation.push(ChatMessage::new("user".to_string(), PLEASE_WRITE_MEM.to_string()));
    let results = execute_subchat_single_iteration(
        gcx.clone(),
        "gpt-4o-mini",
        &sub_conversation,
        &vec![],
        Some("none".to_string()),
        false,
    ).await?;

    // reduce execute_subchat_single_iteration
    // write prompt

    // parse reduce result
    let result_content = results.last().map(|m| m.content.to_string()).ok_or_else(|| "find_relevant_files: no results".to_string())?;
    let mem: PleaseWriteMem = match serde_json::from_str(&result_content) {
        Ok(mem) => mem,
        Err(e) => {
            sub_conversation.push(ChatMessage::new("user".to_string(), format!("find_relevant_files: cannot parse result: {}. Try again.", e)));

            let retry_results = execute_subchat(
                gcx.clone(),
                "gpt-4o-mini",
                &sub_conversation,
                &vec![],
                1,
                8192,
            ).await?;

            let retry_result_content = retry_results.last().map(|m| m.content.to_string()).ok_or_else(|| "find_relevant_files: no results after retry".to_string())?;
            serde_json::from_str(&retry_result_content).map_err(|e| format!("find_relevant_files: cannot parse result after retry: {}. Try again.", e))?
        }
    };

    Ok(mem)
}

const RF_SYSTEM_PROMPT: &str = r###"You are an expert in finding relevant files within a big project. Your job is to find files, don't propose any changes.

Look at task description. Here's the list of reasons a file might be relevant wrt task description:
TOCHANGE = changes to that file are necessary to complete the task
DEFINITIONS = file has classes/functions/types involved, but no changes needed
HIGHLEV = file is crucial to understand the logic, such as a database scheme, high level script
USERCODE = file has code that uses the things the task description is about

You have to be mindful of the token count, as some files are large. It's essential to
select specific symbols within a file that are relevant. Another expert will
pick up your results, likely they will have to only look at symbols selected by you,
not whole files, because of the space constraints.

Here's your plan:
1. Call knowledge(), pass a short version of the task as im_going_to_do parameter. This call is
a memory mechanism, it will tell you about your previous attempts at doing the same task. Don't
plan anything until you see your achievements in previous attempts.
2. Don't rewrite data from memory. You need to decide if you want to continue with the unfinished strategy, or try a strategy you didn't use yet, so write only about that. Prefer new strategies, over already tried ones.
3. If the strategy is finished or goes in circles, try a new strategy. Don't repeat the same actions over again. A new strategy is better than a tried one, because it's likely to bring you results you didn't see yet.
4. Make sure the output form is sufficiently filled, actively fill the gaps.
5. There's a hard limit of {RF_ATTEMPTS} attempts. Your memory will tell you which attempt you are on. Make sure on attempt number {RF_ATTEMPTS} to put together your best final answer.

Potential strategies:
CATFILES = call tree(), spot up to {RF_OUTPUT_FILES} suspicious files just by looking at file names, look into them by calling file() in parallel, write down relevant function/class names, summarize what they do. Stop this after checking {RF_OUTPUT_FILES} files, switch to a different strategy.
GOTODEF = call definition() for symbols involved, get more files this way. Don't call for symbols from standard libraries, only symbols within the project are indexed.
GOTOREF = call references() to find usages of the code to be changed to identify what exactly calls or uses the thing in question.
VECDBSEARCH = search() can find semantically similar code, text in comments, and sometimes documentation.
CUSTOM = a different strategy that makes sense for the task at hand.

You'll receive additional instructions that start with 💿. Those are not coming from the user, they are programmed to help you operate
well between chat restarts and they are always in English. Answer in the language the user prefers.

EXPLAIN YOUR ACTIONS BEFORE CALLING ANY FUNCTIONS. IT'S FORBIDDEN TO CALL TOOLS UNTIL YOU EXPLAINED WHAT YOU ARE GOING TO DO.
"###;

const PLEASE_WRITE_MEM: &str = r###"You are out of turns or tokens for this chat. Now you need to save your progress, such that a new chat can pick up from there. Use this structure:
{
  "PROGRESS": {
    "UNFINISHED_STRATEGY": "string",             // Maybe you've got interrupted at a worst possible moment, you were in the middle of executing a good plan! Write down your strategy, which is it? "I was calling this and this tool and looking for this". This is a text field, feel free to write a paragraph. Leave an empty string to try something else on the next attempt.
    "UNFINISHED_STRATEGY_POINTS_TODO": "string"  // In that unfinished strategy, what specific file names or symbols are left to explore? Don't worry about any previous strategies, just the unfinished one. Use comma-separated list, leave an empty string if there's no unfinished strategy. For file paths, omit most of the path, maybe leave one or two parent dirs, just enough for the path not to be ambiguous.
    "STRATEGIES_IN_MEMORY": "string",            // Write comma-separated list of which strategies you can detect in memory (CATFILES, GOTODEF, GOTOREF, VECDBSEARCH, CUSTOM) by looking at action sequences.
    "STRATEGIES_DIDNT_USE": "string"             // Which strategies you can't find in memory or in this chat? (CATFILES, GOTODEF, GOTOREF, VECDBSEARCH, CUSTOM)
  },
  "ACTION_SEQUENCE": {
    "ACTIONS": [           // Write the list of your actions in this chat (not the actions from memory), don't be shy of mistakes, don't omit anything.
      ["string", "string"] // Each element is a tuple with a tool call or a change in your interpretation (aha moments, discoveries, errors), for example ["call", "definition", "MyClass"] or ["discovery", "there are no MyClass in this project, but there is MyClass2"]
    ],
    "GOAL": "string",      // What the goal of the actions above appears to be? It could be the original goal, but try to be more specific.
    "SUCCESSFUL": 0,       // Did your actions actually get you closer to the goal? Rate from 1 to 5. 1 = no visible progress at all or moving in circles, 3 = at least it's going somewhere, 5 = clearly moving towards the goal.
    "REFLECTION": "string" // If the actions were inefficient, what would you have done differently? Write an empty string if the actions were good as it is, or it's not clear how to behave better.
  },
  "OUTPUT": {                       // The output is dict<filename, info_dict>. You don't have to repeat all the previous files visible in memory, but you need to add new relevant files (not all files, only the relevant files) from the current attempt, as well as update info for files already visible in memory, if there are updates in the current chat.
    "dir/dir/file.ext": {           // Here you need a strict absolute path with no ambiguity at all.
      "SYMBOLS": "symbol1,symbol2", // Comma-separated list of functions/classes/types/variables/etc defined within this file that are actually relevant, for example "MyClass::my_function". List all symbols that are relevant, not just some of them. Write "*" to indicate the whole file is necessary. Write "TBD" to indicate you didn't look inside yet.
      "WHY_CODE": "string",         // Write down the reason to include this file in output, pick one of: TOCHANGE, DEFINITIONS, HIGHLEV, USERCODE. Put TBD if you didn't look inside.
      "WHY_DESC": "string",         // Describe why this file matters wrt the task, what's going on inside? Put TBD if you didn't look inside.
      "RELEVANCY": 0                // Critically evaluate how is this file really relevant to the task. Rate from 1 to 5. 1 = this file doesn't even exist, 3 = might provide good insight into the logic behind the program but not directly relevant, 5 = exactly what is needed.
    }
  ],
  "READY": 0,                       // Is the output good enough to give to the user, when you look at output visible in memory and add the output from this attempt? Rate from 1 to 5. 1 = not much was found, 3 = some good files were found, but there are gaps left to fill, such as symbols that actually present in the file and relevant. 5 = the output is very very good, all the classes files (TOCHANGE, DEFINITIONS, HIGHLEV, USERCODE) were checked.
  "STRATEGIES_THIS_CHAT": "string", // Write comma-separated list of which strategies you see you used in this chat (CATFILES, GOTODEF, GOTOREF, VECDBSEARCH, CUSTOM)
  "ALL_STRATEGIES": 0               // Rate from 1 to 5. 1 = one of less strategies tried, 3 = a couple were attempted, 5 = three or more strategies that make sense for the task attempted, and successfully so.
}
"###;

#[derive(Serialize, Deserialize, Debug)]
struct Progress {
    #[serde(rename="UNFINISHED_STRATEGY")]
    unfinished_strategy: String,
    #[serde(rename="UNFINISHED_STRATEGY_POINTS_TODO")]
    unfinished_strategy_points_todo: String,
    #[serde(rename="STRATEGIES_IN_MEMORY")]
    strategies_in_memory: String,
    #[serde(rename="STRATEGIES_DIDNT_USE")]
    strategies_didnt_use: String,
}

#[derive(Serialize, Deserialize, Debug)]
struct Action {
    #[serde(rename="ACTIONS")]
    actions: Vec<(String, String)>,
    #[serde(rename="GOAL")]
    goal: String,
    #[serde(rename="SUCCESSFUL")]
    successful: u8,
    #[serde(rename="REFLECTION")]
    reflection: String,
}

#[derive(Serialize, Deserialize, Debug)]
struct OutputFile {
    #[serde(rename="SYMBOLS")]
    symbols: String,
    #[serde(rename="WHY_CODE")]
    why_code: String,
    #[serde(rename="WHY_DESC")]
    why_desc: String,
    #[serde(rename="RELEVANCY")]
    relevancy: u8,
}

#[derive(Serialize, Deserialize, Debug)]
struct ActionSequence {
    #[serde(rename="ACTIONS")]
    actions: Vec<(String, String)>,
    #[serde(rename="GOAL")]
    goal: String,
    #[serde(rename="SUCCESSFUL")]
    successful: u8,
    #[serde(rename="REFLECTION")]
    reflection: String,
}
#[derive(Serialize, Deserialize, Debug)]
struct PleaseWriteMem {
    #[serde(rename="PROGRESS")]
    progress: Progress,
    #[serde(rename="ACTION_SEQUENCE")]
    action_sequence: ActionSequence,
    #[serde(rename = "OUTPUT")]
    output: HashMap<String, OutputFile>,
    #[serde(rename="READY")]
    ready: u8,
    #[serde(rename="STRATEGIES_THIS_CHAT")]
    strategies_this_chat: String,
    #[serde(rename="ALL_STRATEGIES")]
    all_strategies: u8,
}
