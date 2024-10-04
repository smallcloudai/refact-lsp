use std::sync::Arc;
use std::collections::HashMap;
use tokio::sync::Mutex as AMutex;
use tokio::process::Command;
use async_trait::async_trait;
use tracing::{error, info};
use serde::{Deserialize, Serialize};

use crate::at_commands::at_commands::AtCommandsContext;
use crate::call_validation::{ContextEnum, ChatMessage};

use crate::tools::tools_description::Tool;
use serde_json::Value;

#[derive(Clone, Serialize, Deserialize, Debug)]
#[allow(non_snake_case)]
pub struct IntegrationGit {
    pub git_binary_path: Option<String>,
}

pub struct ToolGit {
    integration_git: IntegrationGit,
}

impl ToolGit {
    pub fn new_if_configured(integrations_value: &serde_yaml::Value) -> Option<Self> {
        let integration_git_value = integrations_value.get("git")?;

        let integration_git = serde_yaml::from_value::<IntegrationGit>(integration_git_value.clone()).or_else(|e| {
            error!("Failed to parse integration git: {:?}", e);
            Err(e)
        }).ok()?;

        Some(Self { integration_git })
    }
}

#[async_trait]
impl Tool for ToolGit {
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

        let git_command = self.integration_git.git_binary_path.as_deref().unwrap_or("git");
        let output = Command::new(git_command)
            .args(&command_args)
            .current_dir(&project_dir)
            .output()
            .await
            .map_err(|e| e.to_string())?;
        let stdout = String::from_utf8_lossy(&output.stdout).to_string();
        let stderr = String::from_utf8_lossy(&output.stderr).to_string();

        if !stderr.is_empty() {
            error!("Error: {:?}", stderr);
            return Err(stderr);
        }

        let mut results = vec![];
        results.push(ContextEnum::ChatMessage(ChatMessage {
            role: "tool".to_string(),
            content: stdout,
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
        command_args.insert(0, "git".to_string());
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
    if parsed_args[0] == "git" {
        parsed_args.remove(0);
    }

    Ok(parsed_args)
}