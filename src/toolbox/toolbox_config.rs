use serde_yaml;
use serde::{Serialize, Deserialize};
use std::collections::HashMap;
use tokio::sync::RwLock as ARwLock;
use crate::call_validation::{ChatMessage, SubchatParameters};
use std::io::Write;
use std::sync::Arc;
use std::path::PathBuf;
use tracing::{error, info};
use crate::global_context::{GlobalContext, try_load_caps_quickly_if_not_present};
use crate::at_tools::tools::{AtParamDict, make_openai_tool_value};
use crate::toolbox::toolbox_compiled_in::{COMPILED_IN_CUSTOMIZATION_YAML, COMPILED_IN_INITIAL_USER_YAML};


#[derive(Deserialize)]
pub struct ToolboxConfigDeserialize {
    #[serde(default)]
    pub system_prompts: HashMap<String, SystemPrompt>,
    #[serde(default)]
    pub subchat_tool_parameters: HashMap<String, SubchatParameters>,
    #[serde(default)]
    pub toolbox_commands: HashMap<String, ToolboxCommand>,
    #[serde(default)]
    pub tools: Vec<AtToolCustDictDeserialize>,
    #[serde(default)]
    pub tools_parameters: Vec<AtParamDict>,
}

#[derive(Debug, Serialize, Deserialize, Default)]
pub struct ToolboxConfig {
    pub system_prompts: HashMap<String, SystemPrompt>,
    pub subchat_tool_parameters: HashMap<String, SubchatParameters>,
    pub toolbox_commands: HashMap<String, ToolboxCommand>,
    pub tools: Vec<ToolCustDict>,
}

#[derive(Clone, Serialize, Deserialize, Debug)]
pub struct ToolCustDict {
    pub name: String,
    pub description: String,
    pub parameters: Vec<AtParamDict>,
    pub parameters_required: Vec<String>,
    pub command: String,
    pub timeout: usize,
    pub output_postprocess: String,
}

impl ToolCustDict {
    pub fn new(cmd: &AtToolCustDictDeserialize, params: &Vec<AtParamDict>) -> Self {
        ToolCustDict {
            name: cmd.name.clone(),
            description: cmd.description.clone(),
            parameters: cmd.parameters.iter()
                .map(
                    |name| params.iter()
                        .find(|param| &param.name == name).unwrap()
                )
                .cloned().collect(),
            parameters_required: cmd.parameters_required.clone(),
            command: cmd.command.clone(),
            timeout: cmd.timeout,
            output_postprocess: cmd.output_postprocess.clone(),
        }
    }

    pub fn into_openai_style(self) -> serde_json::Value {
        make_openai_tool_value(
            self.name,
            false,
            self.description,
            self.parameters_required,
            self.parameters,
        )
    }
}

#[derive(Debug, Deserialize)]
pub struct AtToolCustDictDeserialize {
    pub name: String,
    pub description: String,
    pub parameters: Vec<String>,
    pub parameters_required: Vec<String>,
    pub command: String,
    pub timeout: usize,
    pub output_postprocess: String,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct SystemPrompt {
    #[serde(default)]
    pub description: String,
    pub text: String,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct ToolboxCommand {
    pub description: String,
    pub messages: Vec<ChatMessage>,
    #[serde(default)]
    pub selection_needed: Vec<usize>,
    #[serde(default)]
    pub selection_unwanted: bool,
    #[serde(default)]
    pub insert_at_cursor: bool,
}

fn extract_mapping_values(mapping: &Option<&serde_yaml::Mapping>, variables: &mut HashMap<String, String>)
{
    if let Some(mapping) = mapping {
        for (k, v) in mapping.iter() {
            if let (serde_yaml::Value::String(key), serde_yaml::Value::String(value)) = (k, v) {
                variables.insert(key.clone(), value.clone());
            }
        }
    }
}

fn replace_variables_in_messages(config: &mut ToolboxConfig, variables: &HashMap<String, String>)
{
    for (_, command) in config.toolbox_commands.iter_mut() {
        for (_i, msg) in command.messages.iter_mut().enumerate() {
            let mut tmp = msg.content.clone();
            for (vname, vtext) in variables.iter() {
                tmp = tmp.replace(&format!("%{}%", vname), vtext);
            }
            msg.content = tmp;
        }
    }
}

fn replace_variables_in_system_prompts(config: &mut ToolboxConfig, variables: &HashMap<String, String>)
{
    for (_, prompt) in config.system_prompts.iter_mut() {
        let mut tmp = prompt.text.clone();
        for (vname, vtext) in variables.iter() {
            tmp = tmp.replace(&format!("%{}%", vname), vtext);
        }
        prompt.text = tmp;
    }
}

fn load_and_mix_with_users_config(user_yaml: &str, caps_yaml: &str, caps_default_system_prompt: &str) -> Result<ToolboxConfig, String> {
    let default_unstructured: serde_yaml::Value = serde_yaml::from_str(COMPILED_IN_CUSTOMIZATION_YAML)
        .map_err(|e| format!("Error parsing default YAML: {}\n{}", e, COMPILED_IN_CUSTOMIZATION_YAML))?;
    let user_unstructured: serde_yaml::Value = serde_yaml::from_str(user_yaml)
        .map_err(|e| format!("Error parsing customization.yaml: {}\n{}", e, user_yaml))?;

    let mut variables: HashMap<String, String> = HashMap::new();
    extract_mapping_values(&default_unstructured.as_mapping(), &mut variables);
    extract_mapping_values(&user_unstructured.as_mapping(), &mut variables);

    let work_config_deserialize: ToolboxConfigDeserialize = serde_yaml::from_str(COMPILED_IN_CUSTOMIZATION_YAML)
        .map_err(|e| format!("Error parsing default ToolboxConfig: {}\n{}", e, COMPILED_IN_CUSTOMIZATION_YAML))?;
    let tools = work_config_deserialize.tools.iter()
        .map(|x|ToolCustDict::new(x, &work_config_deserialize.tools_parameters))
        .collect::<Vec<ToolCustDict>>();

    let mut work_config = ToolboxConfig {
        system_prompts: work_config_deserialize.system_prompts,
        toolbox_commands: work_config_deserialize.toolbox_commands,
        subchat_tool_parameters: work_config_deserialize.subchat_tool_parameters,
        tools,
    };

    let user_config_deserialize: ToolboxConfigDeserialize = serde_yaml::from_str(user_yaml)
        .map_err(|e| format!("Error parsing user ToolboxConfig: {}\n{}", e, user_yaml))?;
    let user_tools = user_config_deserialize.tools.iter()
       .map(|x|ToolCustDict::new(x, &user_config_deserialize.tools_parameters))
       .collect::<Vec<ToolCustDict>>();

    let mut user_config = ToolboxConfig {
        system_prompts: user_config_deserialize.system_prompts,
        toolbox_commands: user_config_deserialize.toolbox_commands,
        tools: user_tools,
        ..Default::default()
    };

    replace_variables_in_messages(&mut work_config, &variables);
    replace_variables_in_messages(&mut user_config, &variables);
    replace_variables_in_system_prompts(&mut work_config, &variables);
    replace_variables_in_system_prompts(&mut user_config, &variables);

    let caps_config_deserialize: ToolboxConfigDeserialize = serde_yaml::from_str(caps_yaml)
        .map_err(|e| format!("Error parsing default ToolboxConfig: {}\n{}", e, caps_yaml))?;
    let caps_config = ToolboxConfig {
        system_prompts: caps_config_deserialize.system_prompts,
        toolbox_commands: caps_config_deserialize.toolbox_commands,
        tools: vec![],
        ..Default::default()
    };

    work_config.system_prompts.extend(caps_config.system_prompts.iter().map(|(k, v)| (k.clone(), v.clone())));
    work_config.toolbox_commands.extend(caps_config.toolbox_commands.iter().map(|(k, v)| (k.clone(), v.clone())));

    work_config.system_prompts.extend(user_config.system_prompts.iter().map(|(k, v)| (k.clone(), v.clone())));
    work_config.toolbox_commands.extend(user_config.toolbox_commands.iter().map(|(k, v)| (k.clone(), v.clone())));
    work_config.tools.extend(user_config.tools.iter().map(|x|x.clone()));

    if !caps_default_system_prompt.is_empty() && work_config.system_prompts.get(caps_default_system_prompt).is_some() {
        work_config.system_prompts.insert("default".to_string(), work_config.system_prompts.get(caps_default_system_prompt).map(|x|x.clone()).unwrap());
    }

    Ok(work_config)
}

pub async fn load_customization(gcx: Arc<ARwLock<GlobalContext>>) -> Result<ToolboxConfig, String> {
    let cache_dir = gcx.read().await.cache_dir.clone();
    let caps = try_load_caps_quickly_if_not_present(gcx, 0).await.map_err(|e|format!("error loading caps: {e}"))?;

    let (caps_config_text, caps_default_system_prompt) = {
        let caps_locked = caps.read().unwrap();
        (caps_locked.customization.clone(), caps_locked.code_chat_default_system_prompt.clone())
    };

    let user_config_path = cache_dir.join("customization.yaml");

    if !user_config_path.exists() {
        let mut file = std::fs::File::create(&user_config_path)
            .map_err(|e| format!("Failed to create file: {}", e))?;
        file.write_all(COMPILED_IN_INITIAL_USER_YAML.as_bytes())
            .map_err(|e| format!("Failed to write to file: {}", e))?;

        let the_default = String::from(COMPILED_IN_CUSTOMIZATION_YAML);
        for line in the_default.split('\n') {
            let mut comment = String::from("# ");
            comment.push_str(line);
            comment.push('\n');
            file.write_all(comment.as_bytes())
                .map_err(|e| format!("Failed to write to file: {}", e))?;
        }
    }

    let user_config_text = std::fs::read_to_string(&user_config_path).map_err(|e| format!("Failed to read file: {}", e))?;
    load_and_mix_with_users_config(&user_config_text, &caps_config_text, &caps_default_system_prompt).map_err(|e| e.to_string())
}

pub async fn postprocess_system_prompt(
    global_context: Arc<ARwLock<GlobalContext>>,
    system_prompt: &str
) -> String {
    // if system_prompt.contains("%WORKSPACE_PROJECTS_INFO%") {
    //     let (workspace_dirs, active_file_path) = {
    //         let gcx_locked = gcx.read().await;
    //         let documents_state = &gcx_locked.documents_state;
    //         let dirs_locked = documents_state.workspace_folders.lock().unwrap();
    //         let workspace_dirs = dirs_locked.clone().into_iter().map(|x| x.to_string_lossy().to_string()).collect::<Vec<_>>();
    //         let active_file_path = documents_state.active_file_path.clone();
    //         (workspace_dirs, active_file_path)
    //     };
    //     let mut info = String::new();
    //     if !workspace_dirs.is_empty() {
    //         info.push_str(format!("The current IDE workspace has these project directories:\n{}", workspace_dirs.join("\n")).as_str());
    //     }
    //     let detect_vcs_at_option: Option<PathBuf> = active_file_path.clone().or_else(|| workspace_dirs.get(0).map(PathBuf::from));
    //     if let Some(detect_vcs_at) = detect_vcs_at_option {
    //         let cvs: Option<(PathBuf, &str)> = crate::files_in_workspace::detect_vcs_for_a_file_path(&detect_vcs_at).await;
    //         if let Some(active_file) = active_file_path {
    //             info.push_str(format!("\n\nThe active IDE file is:\n{}", active_file.display()).as_str());
    //         } else {
    //             info.push_str("\n\nThere is no active file currently open in the IDE.");
    //         }
    //         if let Some((vcs_path, vcs_type)) = cvs {
    //             info.push_str(format!("\nThe project is under {} version control, located at:\n{}",
    //                 vcs_type,
    //                 vcs_path.display(),
    //             ).as_str());
    //         } else {
    //             info.push_str("\nThere's no version control detected, complain to user if they want to use anything git/hg/svn/etc.");
    //         }
    //     } else {
    //         info.push_str(format!("\n\nThere is no active file with version control, complain to user if they want to use anything git/hg/svn/etc and ask to open a file in IDE for you to know which project is active.").as_str());
    //     }
    //     system_prompt = system_prompt.replace("%WORKSPACE_PROJECTS_INFO%", &info);
    // }
    // info!("system_prompt\n{}", system_prompt);

    // let mut additional_info = String::new();
    // let datetime = chrono::Local::now().format("%Y-%m-%d %H:%M:%S").to_string();
    // let os = std::env::consts::OS;
    // let username = std::env::var("USER")
    //     .or_else(|_| std::env::var("USERNAME"))
    //     .unwrap_or_else(|_| String::from("unknown"));
    // let additional_info += format!("ENVIRONMENT INFO:\nDATETIME: {}\nOS: {}\nUSER: {}\n", datetime, os, username);

    let mut system_prompt = system_prompt.to_string();
    let workspace_dirs = {
        let workspace_dirs_arc = global_context.read().await.documents_state.workspace_folders.clone();
        let dirs_lock = workspace_dirs_arc.lock().unwrap();
        dirs_lock.clone().into_iter().map(|x| x.to_string_lossy().to_string()).collect::<Vec<_>>()
    };
    if !workspace_dirs.is_empty() && system_prompt.contains("%WORKSPACE_PROJECTS_INFO%") {
        system_prompt = system_prompt.replace("%WORKSPACE_PROJECTS_INFO%", &format!("The current IDE workspace has these project directories:\n{}\n", workspace_dirs.join("\n")).as_str());
    }
    system_prompt
}

pub async fn get_default_system_prompt(
    gcx: Arc<ARwLock<GlobalContext>>,
    have_exploration_tools: bool,
    have_agentic_tools: bool,
) -> String {
    let tconfig = match load_customization(gcx.clone()).await {
        Ok(tconfig) => tconfig,
        Err(e) => {
            error!("cannot load_customization: {e}");
            return "".to_string()
        },
    };
    let mut system_prompt = if have_agentic_tools {
        tconfig.system_prompts.get("agentic_tools")
            .map_or_else(
                || {
                    error!("cannot find system prompt `agentic_tools`");
                    String::new()
                },
                |x| x.text.clone()
            )
    } else if have_exploration_tools {
        tconfig.system_prompts.get("exploration_tools")
            .map_or_else(
                || {
                    error!("cannot find system prompt `exploration_tools`");
                    String::new()
                },
                |x| x.text.clone()
            )
    } else {
        tconfig.system_prompts.get("default")
            .map_or_else(
                || {
                    error!("cannot find system prompt `default`");
                    String::new()
                },
                |x| x.text.clone()
            )
    };

    system_prompt = postprocess_system_prompt(gcx, &system_prompt).await;
    system_prompt
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn is_compiled_in_toolbox_valid_yaml() {
        let _config = load_and_mix_with_users_config(COMPILED_IN_INITIAL_USER_YAML, "", "");
    }
}
