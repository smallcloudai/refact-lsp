use indexmap::IndexMap;
use std::collections::HashMap;
use std::sync::Arc;
use serde_json::{Value, json};
use serde::{Deserialize, Serialize};
use async_trait::async_trait;
use tokio::sync::RwLock as ARwLock;
use tokio::sync::Mutex as AMutex;

use crate::at_commands::at_commands::AtCommandsContext;
use crate::call_validation::{ChatUsage, ContextEnum};
use crate::global_context::GlobalContext;
use crate::toolbox::toolbox_config::ToolCustDict;


#[async_trait]
pub trait Tool: Send + Sync {
    async fn tool_execute(
        &mut self,
        ccx: Arc<AMutex<AtCommandsContext>>,
        tool_call_id: &String,
        args: &HashMap<String, Value>
    ) -> Result<(bool, Vec<ContextEnum>), String>;

    fn tool_depends_on(&self) -> Vec<String> { vec![] }   // "ast", "vecdb"

    fn usage(&mut self) -> &mut Option<ChatUsage> {
        static mut DEFAULT_USAGE: Option<ChatUsage> = None;
        #[allow(static_mut_refs)]
        unsafe { &mut DEFAULT_USAGE }
    }
}

pub async fn at_tools_merged_and_filtered(gcx: Arc<ARwLock<GlobalContext>>) -> IndexMap<String, Arc<AMutex<Box<dyn Tool + Send>>>>
{
    let (ast_on, vecdb_on, experimental) = {
        let gcx_locked = gcx.read().await;
        let vecdb = gcx_locked.vec_db.lock().await;
        (gcx_locked.ast_module.is_some(), vecdb.is_some(), gcx_locked.cmdline.experimental)
    };

    let mut tools_all = IndexMap::from([
        ("search".to_string(), Arc::new(AMutex::new(Box::new(crate::at_tools::att_search::AttSearch{}) as Box<dyn Tool + Send>))),
        ("definition".to_string(), Arc::new(AMutex::new(Box::new(crate::at_tools::att_ast_definition::AttAstDefinition{}) as Box<dyn Tool + Send>))),
        ("references".to_string(), Arc::new(AMutex::new(Box::new(crate::at_tools::att_ast_reference::AttAstReference{}) as Box<dyn Tool + Send>))),
        ("tree".to_string(), Arc::new(AMutex::new(Box::new(crate::at_tools::att_tree::AttTree{}) as Box<dyn Tool + Send>))),
        ("patch".to_string(), Arc::new(AMutex::new(Box::new(crate::at_tools::att_patch::tool::ToolPatch::new()) as Box<dyn Tool + Send>))),
        ("knowledge".to_string(), Arc::new(AMutex::new(Box::new(crate::at_tools::att_knowledge::AttGetKnowledge{}) as Box<dyn Tool + Send>))),
        ("web".to_string(), Arc::new(AMutex::new(Box::new(crate::at_tools::att_web::AttWeb{}) as Box<dyn Tool + Send>))),
        ("cat".to_string(), Arc::new(AMutex::new(Box::new(crate::at_tools::att_cat::AttCat{}) as Box<dyn Tool + Send>))),
        // ("locate".to_string(), Arc::new(AMutex::new(Box::new(crate::at_tools::att_locate::AttLocate{}) as Box<dyn Tool + Send>))),
        ("locate".to_string(), Arc::new(AMutex::new(Box::new(crate::at_tools::att_relevant_files::AttRelevantFiles{}) as Box<dyn Tool + Send>))),
    ]);

    if experimental {
        // ("save_knowledge".to_string(), Arc::new(AMutex::new(Box::new(crate::at_tools::att_knowledge::AttSaveKnowledge{}) as Box<dyn Tool + Send>))),
        // ("memorize_if_user_asks".to_string(), Arc::new(AMutex::new(Box::new(crate::at_tools::att_note_to_self::AtNoteToSelf{}) as Box<dyn AtTool + Send>))),
        tools_all.insert("github".to_string(), Arc::new(AMutex::new(Box::new(crate::at_tools::tool_github::ToolGithub{}) as Box<dyn Tool + Send>)));
    }

    let mut filtered_tools = IndexMap::new();
    for (tool_name, tool_arc) in tools_all {
        let tool_locked = tool_arc.lock().await;
        let dependencies = tool_locked.tool_depends_on();
        if dependencies.contains(&"ast".to_string()) && !ast_on {
            continue;
        }
        if dependencies.contains(&"vecdb".to_string()) && !vecdb_on {
            continue;
        }
        filtered_tools.insert(tool_name, tool_arc.clone());
    }

    // let tconfig_maybe = crate::toolbox::toolbox_config::load_customization(gcx.clone()).await;
    // if tconfig_maybe.is_err() {
    //     tracing::error!("Error loading toolbox config: {:?}", tconfig_maybe.err().unwrap());
    // } else {
    //     for cust in tconfig_maybe.unwrap().tools {
    //         result.insert(
    //             cust.name.clone(),
    //             Arc::new(AMutex::new(Box::new(crate::at_tools::att_execute_cmd::AttExecuteCommand {
    //                 command: cust.command,
    //                 timeout: cust.timeout,
    //                 output_postprocess: cust.output_postprocess,
    //             }) as Box<dyn Tool + Send>)));
    //     }
    // }

    filtered_tools
}

const TOOLS: &str = r####"
tools:
  - name: "search"
    description: "Find similar pieces of code or text using vector database"
    parameters:
      - name: "query"
        type: "string"
        description: "Single line, paragraph or code sample to search for similar content."
      - name: "scope"
        type: "string"
        description: "'workspace' to search all files in workspace, 'dir/subdir/' to search in files within a directory, 'dir/file.ext' to search in a single file."
    parameters_required:
      - "query"
      - "scope"

  - name: "definition"
    description: "Read definition of a symbol in the project using AST"
    parameters:
      - name: "symbol"
        type: "string"
        description: "The exact name of a function, method, class, type alias. No spaces allowed."
      - name: "skeleton"
        type: "boolean"
        description: "Skeletonize ouput. Set true to explore, set false when as much context as possible is needed."
    parameters_required:
      - "symbol"

  - name: "references"
    description: "Find usages of a symbol within a project using AST"
    parameters:
      - name: "symbol"
        type: "string"
        description: "The exact name of a function, method, class, type alias. No spaces allowed."
      - name: "skeleton"
        type: "boolean"
        description: "Skeletonize ouput. Set true to explore, set false when as much context as possible is needed."
    parameters_required:
      - "symbol"

  - name: "tree"
    description: "Get a files tree with symbols for the project. Use it to get familiar with the project, file names and symbols"
    parameters:
      - name: "path"
        type: "string"
        description: "An optional absolute path to get files tree for a particular folder or file. Do not pass it if you need full project tree."
      - name: "use_ast"
        type: "boolean"
        description: "if true, for each file an array of AST symbols will appear as well as its filename"
    parameters_required: []

  - name: "web"
    description: "Fetch a web page and convert to readable plain text."
    parameters:
      - name: "url"
        type: "string"
        description: "URL of the web page to fetch."
    parameters_required:
      - "url"

  - name: "knowledge"
    description: "What kind of knowledge you will need to accomplish this task? Call each time you have a new task or topic."
    parameters:
      - name: "im_going_to_do"
        type: "string"
        description: "Put your intent there: 'debug file1.cpp', 'install project1', 'gather info about MyClass'"
    parameters_required:
      - "im_going_to_do"

  - name: "cat"
    description: "Like cat in console, but better: it can read multiple files and skeletonize them. Give it AST symbols important for the goal (classes, functions, variables, etc) to see them in full."
    parameters:
      - name: "paths"
        type: "string"
        description: "Comma separated file names or directories: dir1/file1.ext, dir2/file2.ext, dir3/dir4"
      - name: "symbols"
        type: "string"
        description: "Comma separated AST symbols: MyClass, MyClass::method, my_function"
      - name: "skeleton"
        type: "boolean"
        description: "if true, files will be skeletonized - mostly only AST symbols will be visible"
    parameters_required:
      - "paths"

  # -- agentic tools below --

  - name: "locate"
    agentic: true
    description: "Get a list of files that are relevant to solve a particular task."
    parameters:
      - name: "problem_statement"
        type: "string"
        description: "Copy word-for-word the problem statement as provided by the user, if available. Otherwise, tell what you need to do in your own words."
    parameters_required:
      - "problem_statement"

  - name: "patch"
    agentic: true
    description: "Make modifications to multiple source files. Can edit, rename, create, delete files. Calling this once for multiple files is better than multiple calls, because the changes will be consistent between the files."
    parameters:
      - name: "paths"
        type: "string"
        description: "If there is a good locate() call above, use 'pick_locate_json_above' magic string. If there isn't, use comma separated files list: dir/file1.ext, dir/file2.ext"
      - name: "todo"
        type: "string"
        description: "Copy word-for-word the problem statement as provided by the user, if available. Otherwise, tell what you need to do in your own words."
    parameters_required:
      - "paths"
      - "todo"

  - name: "github"
    agentic: true
    description: "Access to gh command line command, to fetch issues, review PRs."
    parameters:
      - name: "project_dir"
        type: "string"
        description: "Look at system prompt for location of version control (.git folder) of the active file."
      - name: "command"
        type: "string"
        description: 'Examples:\ngh issue create --body "hello world" --title "Testing gh integration"\ngh issue list --author @me --json id,title,labels,updatedAt,body\n'
    parameters_required:
      - "project_dir"
      - "command"
"####;

#[allow(dead_code)]
const NOT_READY_TOOLS: &str = r####"
  - name: "diff"
    description: "Perform a diff operation. Can be used to get git diff for a project (no arguments) or git diff for a specific file (file_path)"
    parameters:
      - name: "file_path"
        type: "string"
        description: "Path to the specific file to diff (optional)."
    parameters_required:
"####;


// - name: "save_knowledge"
// description: "Use it when you see something you'd want to remember about user, project or your experience for your future self."
// parameters:
//   - name: "memory_topic"
//     type: "string"
//     description: "one or two words that describe the memory"
//   - name: "memory_text"
//     type: "string"
//     description: "The text of memory you want to save"
//   - name: "memory_type"
//     type: "string"
//     description: "one of: `consequence` -- the set of actions that caused success / fail; `reflection` -- what can you do better next time; `familirity` -- what new did you get about the project; `relationship` -- what new did you get about the user."
// parameters_required:
//   - "memory_topic"
//   - "memory_text"
//   - "memory_type"

// - "op"
// - name: "op"
// type: "string"
// description: "Operation on a file: 'new', 'edit', 'remove'"
// - "lookup_definitions"
// - name: "lookup_definitions"
// type: "string"
// description: "Comma separated types that might be useful in making this change"
// - name: "remember_how_to_use_tools"
// description: Save a note to memory.
// parameters:
//   - name: "text"
//     type: "string"
//     description: "Write the exact format message here, starting with CORRECTION_POINTS"
// parameters_required:
//   - "text"

// - name: "memorize_if_user_asks"
// description: |
//     DO NOT CALL UNLESS USER EXPLICITLY ASKS. Use this format exactly:
//     when ... [describe situation when it's applicable] use ... tool call or method or plan
// parameters:
//   - name: "text"
//     type: "string"
//     description: "Follow the format in function description."
// parameters_required:
//   - "text"
//   - "shortdesc"


#[derive(Deserialize)]
pub struct ToolDictDeserialize {
    pub tools: Vec<ToolDict>,
}

#[derive(Clone, Serialize, Deserialize, Debug)]
pub struct ToolDict {
    pub name: String,
    #[serde(default)]
    pub agentic: bool,
    pub description: String,
    pub parameters: Vec<AtParamDict>,
    pub parameters_required: Vec<String>,
}

#[derive(Clone, Serialize, Deserialize, Debug)]
pub struct AtParamDict {
    pub name: String,
    #[serde(rename = "type")]
    pub param_type: String,
    pub description: String,
}

pub fn make_openai_tool_value(
    name: String,
    agentic: bool,
    description: String,
    parameters_required: Vec<String>,
    parameters: Vec<AtParamDict>,
) -> Value {
    let params_properties = parameters.iter().map(|param| {
        (
            param.name.clone(),
            json!({
                "type": param.param_type,
                "description": param.description
            })
        )
    }).collect::<serde_json::Map<_, _>>();

    let function_json = json!({
            "type": "function",
            "function": {
                "name": name,
                "agentic": agentic,
                "description": description,
                "parameters": {
                    "type": "object",
                    "properties": params_properties,
                    "required": parameters_required
                }
            }
        });
    function_json
}

impl ToolDict {
    pub fn into_openai_style(self) -> Value {
        make_openai_tool_value(
            self.name,
            self.agentic,
            self.description,
            self.parameters_required,
            self.parameters,
        )
    }
}

pub fn tool_description_list_from_yaml(turned_on: &Vec<String>) -> Result<Vec<ToolDict>, String> {
    let at_dict: ToolDictDeserialize = serde_yaml::from_str(TOOLS)
        .map_err(|e|format!("Failed to parse TOOLS: {}", e))?;
    Ok(at_dict.tools.iter().filter(|x|turned_on.contains(&x.name)).cloned().collect::<Vec<_>>())
}

pub async fn tools_from_customization(gcx: Arc<ARwLock<GlobalContext>>, turned_on: &Vec<String>) -> Vec<ToolCustDict> {
    return match crate::toolbox::toolbox_config::load_customization(gcx.clone()).await {
        Ok(tconfig) => tconfig.tools.iter().filter(|x|turned_on.contains(&x.name)).cloned().collect::<Vec<_>>(),
        Err(e) => {
            tracing::error!("Error loading toolbox config: {:?}", e);
            vec![]
        }
    }
}
