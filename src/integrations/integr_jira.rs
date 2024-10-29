use std::sync::Arc;
use std::collections::HashMap;
use tokio::sync::Mutex as AMutex;
use tokio::process::Command;
use async_trait::async_trait;
use tracing::{error, info};
use serde::{Deserialize, Serialize};

use crate::at_commands::at_commands::AtCommandsContext;
use crate::call_validation::{ContextEnum, ChatMessage, ChatContent};

use crate::tools::tools_description::Tool;
use serde_json::Value;


#[derive(Clone, Serialize, Deserialize, Debug)]
#[allow(non_snake_case)]
pub struct IntegrationJira {
    pub jira_binary_path: Option<String>,
    pub JIRA_TOKEN: String,
}

pub struct ToolJira {
    integration_jira: IntegrationJira,
}

impl ToolJira {
    pub fn new_from_yaml(jira_config: &serde_yaml::Value) -> Result<Self, String> {
        let integration_jira = serde_yaml::from_value::<IntegrationJira>(jira_config.clone())
            .map_err(|e| {
                let location = e.location().map(|loc| format!(" at line {}, column {}", loc.line(), loc.column())).unwrap_or_default();
                format!("{}{}", e.to_string(), location)
            })?;
        Ok(Self { integration_jira })
    }
}

#[async_trait]
impl Tool for ToolJira {
    async fn tool_execute(
        &mut self,
        _ccx: Arc<AMutex<AtCommandsContext>>,
        tool_call_id: &String,
        args: &HashMap<String, Value>,
    ) -> Result<(bool, Vec<ContextEnum>), String> {
        let project_dir = match args.get("project_dir") {
            Some(Value::String(s)) => s,
            Some(v) => return Err(format!("argument `project_dir` is not a string: {:?}", v)),
            None => return Err("Missing argument `project_dir`".to_string())
        };
        let command_args = parse_command_args(args)?;

        let jira_command = self.integration_jira.jira_binary_path.as_deref().unwrap_or("jira");
        let output = Command::new(jira_command)
            .args(&command_args)
            .current_dir(&project_dir)
            .env("JIRA_TOKEN", &self.integration_jira.JIRA_TOKEN)
            .output()
            .await
            .map_err(|e| e.to_string())?;
        let stdout = String::from_utf8_lossy(&output.stdout).to_string();
        let stderr = String::from_utf8_lossy(&output.stderr).to_string();

        if !stderr.is_empty() {
            error!("Error: {:?}", stderr);
            return Err(stderr);
        }

        let content = if stdout.starts_with("[") {
            match serde_json::from_str::<Value>(&stdout) {
                Ok(Value::Array(arr)) => {
                    let row_count = arr.len();
                    format!("{}\n\n💿 The UI has the capability to view tool result json efficiently. The result contains {} rows. Write no more than 3 rows as text and possibly \"and N more\" wording, keep it short.",
                        stdout, row_count
                    )
                },
                Ok(_) => stdout,
                Err(_) => stdout,
            }
        } else {
            stdout
        };
        let mut results = vec![];
        results.push(ContextEnum::ChatMessage(ChatMessage {
            role: "tool".to_string(),
            content: ChatContent::SimpleText(content),
            tool_calls: None,
            tool_call_id: tool_call_id.clone(),
            ..Default::default()
        }));

        Ok((false, results))
    }

    fn command_to_match_against_confirm_deny(
        &self,
        args: &HashMap<String, Value>,
    ) -> Result<String, String> {
        let mut command_args = parse_command_args(args)?;
        command_args.insert(0, "jira".to_string());
        Ok(command_args.join(" "))
    }
}

fn parse_command_args(args: &HashMap<String, Value>) -> Result<Vec<String>, String> {
    let command = match args.get("command") {
        Some(Value::String(s)) => s,
        Some(v) => return Err(format!("argument `command` is not a string: {:?}", v)),
        None => return Err("Missing argument `command`".to_string())
    };

    let mut parsed_args = shell_words::split(&command).map_err(|e| e.to_string())?;
    if parsed_args.is_empty() {
        return Err("Parsed command is empty".to_string());
    }
    for (i, arg) in parsed_args.iter().enumerate() {
        info!("argument[{}]: {}", i, arg);
    }
    if parsed_args[0] == "jira" {
        parsed_args.remove(0);
    }

    Ok(parsed_args)
}
