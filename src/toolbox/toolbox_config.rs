use serde_yaml::Value;
use serde_yaml;
use serde::{Serialize, Deserialize};
use crate::call_validation::ChatMessage;
use std::io::Write;
use linked_hash_map::LinkedHashMap;


#[derive(Debug, Serialize, Deserialize)]
pub struct ReadToolboxConfig {
    #[serde(default)]
    pub system_prompts: LinkedHashMap<String, ReadSystemPrompt>,
    #[serde(default)]
    pub toolbox_commands: LinkedHashMap<String, ReadToolboxCommand>,
}

impl ReadToolboxConfig {
    pub fn into_toolbox_config(self) -> ToolboxConfig {
        ToolboxConfig {
            system_prompts: self.system_prompts.into_iter()
                .map(|(id, prompt)| SystemPrompt {
                    id,
                    description: prompt.description,
                    text: prompt.text,
                })
                .collect(),
            default_system_prompt_id: "".to_string(),
            toolbox_commands: self.toolbox_commands.into_iter()
                .map(|(id, command)| ToolboxCommand {
                    id,
                    description: command.description,
                    messages: command.messages,
                    selection_needed: command.selection_needed,
                    selection_unwanted: command.selection_unwanted,
                    insert_at_cursor: command.insert_at_cursor,
                })
                .collect(),
        }
    }
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct ReadSystemPrompt {
    #[serde(default)]
    pub description: String,
    pub text: String,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct ReadToolboxCommand {
    pub description: String,
    pub messages: Vec<ChatMessage>,
    #[serde(default)]
    pub selection_needed: Vec<usize>,
    #[serde(default)]
    pub selection_unwanted: bool,
    #[serde(default)]
    pub insert_at_cursor: bool,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct ToolboxConfig {
    pub system_prompts: Vec<SystemPrompt>,
    pub default_system_prompt_id: String,
    pub toolbox_commands: Vec<ToolboxCommand>,
}

impl ToolboxConfig {
    pub fn update_with_config(&mut self, config: &ToolboxConfig) {
        for system_prompt in config.system_prompts.iter() {
            match self.system_prompts.iter_mut().find(|sp| sp.id == system_prompt.id) {
                Some(sp) => {
                    sp.description = system_prompt.description.clone();
                    sp.text = system_prompt.text.clone();
                }
                None => {
                    self.system_prompts.push(system_prompt.clone());
                }
            }
        }
        for command in config.toolbox_commands.iter() {
            match self.toolbox_commands.iter_mut().find(|c| c.id == command.id) {
                Some(c) => {
                    c.description = command.description.clone();
                    c.messages = command.messages.clone();
                    c.selection_needed = command.selection_needed.clone();
                    c.selection_unwanted = command.selection_unwanted;
                    c.insert_at_cursor = command.insert_at_cursor;
                }
                None => {
                    self.toolbox_commands.push(command.clone());
                }
            }
        }
    }
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct SystemPrompt {
    pub id: String,
    pub description: String,
    pub text: String,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct ToolboxCommand {
    pub id: String,
    pub description: String,
    pub messages: Vec<ChatMessage>,
    pub selection_needed: Vec<usize>,
    pub selection_unwanted: bool,
    pub insert_at_cursor: bool,
}

fn extract_mapping_values(mapping: &Option<&serde_yaml::Mapping>, variables: &mut LinkedHashMap<String, String>)
{
    if let Some(mapping) = mapping {
        for (k, v) in mapping.iter() {
            if let (Value::String(key), Value::String(value)) = (k, v) {
                variables.insert(key.clone(), value.clone());
            }
        }
    }
}

fn _replace_variables_in_messages(config: &mut ReadToolboxConfig, variables: &LinkedHashMap<String, String>)
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

fn replace_variables_in_system_prompts(config: &mut ReadToolboxConfig, variables: &LinkedHashMap<String, String>)
{
    for (_, prompt) in config.system_prompts.iter_mut() {
        let mut tmp = prompt.text.clone();
        for (vname, vtext) in variables.iter() {
            tmp = tmp.replace(&format!("%{}%", vname), vtext);
        }
        prompt.text = tmp;
    }
}

fn parse_and_process_yaml(yaml_str: &str, variables: &mut LinkedHashMap<String, String>) -> Result<ToolboxConfig, String> {
    let unstructured: Value = serde_yaml::from_str(yaml_str).map_err(|e| format!("Error parsing YAML: {}", e))?;
    extract_mapping_values(&unstructured.as_mapping(), variables);
    let mut read_config: ReadToolboxConfig = serde_yaml::from_str(yaml_str).map_err(|e| format!("Error parsing ToolboxConfig: {}", e))?;
    replace_variables_in_system_prompts(&mut read_config, variables);
    Ok(read_config.into_toolbox_config())
}

fn load_and_mix_with_users_config(
    user_yaml: &str,
    customization_web_mb: Option<String>,
) -> Result<ToolboxConfig, String> {
    let mut variables: LinkedHashMap<String, String> = LinkedHashMap::new();
    let default_yaml = crate::toolbox::toolbox_compiled_in::COMPILED_IN_CUSTOMIZATION_YAML;

    let mut config = parse_and_process_yaml(default_yaml, &mut variables)?;
    let user_config = parse_and_process_yaml(user_yaml, &mut variables)?;

    if let Some(customization_web) = customization_web_mb {
        let config_web = parse_and_process_yaml(&customization_web, &mut variables)?;
        config.update_with_config(&config_web);
    }

    config.update_with_config(&user_config);

    Ok(config)
}

pub fn load_customization_high_level(
    cache_dir: std::path::PathBuf,
    customization_web_mb: Option<String>,
) -> Result<ToolboxConfig, String> {
    let user_config_path = cache_dir.join("customization.yaml");

    if !user_config_path.exists() {
        let mut file = std::fs::File::create(&user_config_path)
            .map_err(|e| format!("Failed to create file: {}", e))?;
        file.write_all(crate::toolbox::toolbox_compiled_in::COMPILED_IN_INITIAL_USER_YAML.as_bytes())
            .map_err(|e| format!("Failed to write to file: {}", e))?;

        let the_default = String::from(crate::toolbox::toolbox_compiled_in::COMPILED_IN_CUSTOMIZATION_YAML);
        for line in the_default.split('\n') {
            let mut comment = String::from("# ");
            comment.push_str(line);
            comment.push('\n');
            file.write_all(comment.as_bytes())
                .map_err(|e| format!("Failed to write to file: {}", e))?;
        }
    }

    let user_config_text = std::fs::read_to_string(&user_config_path).map_err(|e| format!("Failed to read file: {}", e))?;
    load_and_mix_with_users_config(&user_config_text, customization_web_mb).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn is_compiled_in_toolbox_valid_toml() {
        let config = load_and_mix_with_users_config(crate::toolbox::toolbox_compiled_in::COMPILED_IN_INITIAL_USER_YAML, None);
        assert!(config.is_ok());
    }
}
