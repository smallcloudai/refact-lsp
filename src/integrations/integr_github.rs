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
pub struct IntegrationGitHub {
    pub gh_binary_path: Option<String>,
    pub GH_TOKEN: String,
}

pub struct ToolGithub {
    integration_github: IntegrationGitHub,
}

impl ToolGithub {
    pub fn new_if_configured(integrations_value: &serde_yaml::Value) -> Option<Self> {
        let integration_github_value = integrations_value.get("github").or_else(|| {
            info!("No integration github found in integrations.yaml");
            None
        })?;

        let integration_github = serde_yaml::from_value::<IntegrationGitHub>(integration_github_value.clone()).or_else(|e| {
            error!("Failed to parse integration github: {:?}", e);
            Err(e)
        }).ok()?;
        Some(Self { integration_github })
    }
}

#[async_trait]
impl Tool for ToolGithub {
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
        let command = match args.get("command") {
            Some(Value::String(s)) => s,
            Some(v) => return Err(format!("argument `command` is not a string: {:?}", v)),
            None => return Err("Missing argument `command`".to_string())
        };

        let mut parsed_args = shell_words::split(command).map_err(|e| e.to_string())?;
        if parsed_args.is_empty() {
            return Err("Parsed command is empty".to_string());
        }
        for (i, arg) in parsed_args.iter().enumerate() {
            info!("argument[{}]: {}", i, arg);
        }
        if parsed_args[0] == "gh" {
            parsed_args.remove(0);
        }

        let gh_command = self.integration_github.gh_binary_path.as_deref().unwrap_or("gh");
        let output = Command::new(gh_command)
            .args(&parsed_args)
            .current_dir(project_dir)
            .env("GH_TOKEN", &self.integration_github.GH_TOKEN)
            .output()
            .await
            .map_err(|e| e.to_string())?;
        let stdout = String::from_utf8_lossy(&output.stdout).to_string();
        let stderr = String::from_utf8_lossy(&output.stderr).to_string();

        if !stderr.is_empty() {
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
            content: content,
            tool_calls: None,
            tool_call_id: tool_call_id.clone(),
            ..Default::default()
        }));

        Ok((false, results))
    }
}
