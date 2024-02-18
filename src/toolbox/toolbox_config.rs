// use serde_json::{Value, Map};
use serde_yaml::Value;
use serde_yaml;
use serde::{Serialize, Deserialize};
use std::collections::HashMap;
use tracing::error;

#[derive(Debug, Serialize, Deserialize)]
pub struct ToolboxConfig {
    // #[serde(alias = "SYSTEM_PROMPT")]   
    // pub system_prompt: String,
    pub commands: HashMap<String, ToolboxCommand>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct ToolboxCommand {
    pub description: String,
    pub messages: Vec<Vec<String>>,
    #[serde(default)]
    pub selection_needed: Vec<usize>,
    #[serde(default)]
    pub selection_unwanted: bool,
    #[serde(default)]
    pub insert_at_cursor: bool,
}

pub fn load_and_mix_with_users_config() -> ToolboxConfig {
    let unstructured: serde_yaml::Value = serde_yaml::from_str(crate::toolbox::toolbox_compiled_in::COMPILED_IN_TOOLBOX_YAML).unwrap();
    let mut tconfig: ToolboxConfig = serde_yaml::from_str(crate::toolbox::toolbox_compiled_in::COMPILED_IN_TOOLBOX_YAML).unwrap();
    let mut variables = HashMap::<String, String>::new();
    if let Some(mapping) = unstructured.as_mapping() {
        for (k, v) in mapping {
            if let (Value::String(key), Value::String(value)) = (k, v) {
                variables.insert(key.clone(), value.clone());
            }
        }
    }
    println!("{:?}", variables);
    for (_, command) in tconfig.commands.iter_mut() {
        let debug_msg = format!("{:?}", command);
        // println!("k={} {:?}", k, command);
        for (i, listof2) in command.messages.iter_mut().enumerate() {
            println!("    i={} listof2={:?}", i, listof2);
            if listof2.len() != 2 {
                error!("message n={} in this command must have two elements: {}", i, debug_msg);
                continue;
            }
            let mut tmp = listof2[1].clone();
            for (vname, vtext) in variables.iter() {
                tmp = tmp.replace(format!("${}", vname).as_str(), vtext.as_str());
            }
            listof2[1] = tmp;
        }
    }
    tconfig
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn is_compiled_in_toolbox_valid_toml() {
        let _yaml: serde_yaml::Value = serde_yaml::from_str(crate::toolbox::toolbox_compiled_in::COMPILED_IN_TOOLBOX_YAML).unwrap();
    }

    #[test]
    fn does_compiled_in_toolbox_fit_structs() {
        let _yaml: ToolboxConfig = serde_yaml::from_str(crate::toolbox::toolbox_compiled_in::COMPILED_IN_TOOLBOX_YAML).unwrap();
    }
}
