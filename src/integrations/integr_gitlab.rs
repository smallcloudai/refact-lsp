use std::sync::Arc;
use std::collections::HashMap;
use tokio::sync::Mutex as AMutex;
use tokio::process::Command;
use async_trait::async_trait;
use tracing::{error, info};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::at_commands::at_commands::AtCommandsContext;
use crate::call_validation::{ContextEnum, ChatMessage, ChatContent, ChatUsage};
use crate::tools::tools_description::Tool;
use crate::integrations::integr_abstract::{IntegrationCommon, IntegrationConfirmation, IntegrationTrait};

#[derive(Clone, Serialize, Deserialize, Debug, Default)]
#[allow(non_snake_case)]
pub struct SettingsGitLab {
    pub glab_binary_path: String,
    pub glab_token: String,
}

#[derive(Default)]
pub struct ToolGitlab {
    pub common:  IntegrationCommon,
    pub settings_gitlab: SettingsGitLab,
}

impl IntegrationTrait for ToolGitlab {
    fn as_any(&self) -> &dyn std::any::Any { self }

    fn integr_settings_apply(&mut self, value: &Value) -> Result<(), String> {
        match serde_json::from_value::<SettingsGitLab>(value.clone()) {
            Ok(settings_gitlab) => {
                info!("GitLab settings applied: {:?}", settings_gitlab);
                self.settings_gitlab = settings_gitlab;
            },
            Err(e) => {
                error!("Failed to apply settings: {}\n{:?}", e, value);
                return Err(e.to_string())
            }
        };
        match serde_json::from_value::<IntegrationCommon>(value.clone()) {
            Ok(x) => self.common = x,
            Err(e) => {
                error!("Failed to apply common settings: {}\n{:?}", e, value);
                return Err(e.to_string());
            }
        };
        Ok(())
    }

    fn integr_settings_as_json(&self) -> Value {
        serde_json::to_value(&self.settings_gitlab).unwrap_or_default()
    }

    fn integr_common(&self) -> IntegrationCommon {
        self.common.clone()
    }
    
    fn integr_upgrade_to_tool(&self, _integr_name: &str) -> Box<dyn Tool + Send> {
        Box::new(ToolGitlab {
            common: self.common.clone(),
            settings_gitlab: self.settings_gitlab.clone()
        }) as Box<dyn Tool + Send>
    }

    fn integr_schema(&self) -> &str { GITLAB_INTEGRATION_SCHEMA }
}

#[async_trait]
impl Tool for ToolGitlab {
    fn as_any(&self) -> &dyn std::any::Any { self }

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

        let mut glab_binary_path = self.settings_gitlab.glab_binary_path.clone();
        if glab_binary_path.is_empty() {
            glab_binary_path = "glab".to_string();
        }
        let output = Command::new(glab_binary_path)
            .args(&command_args)
            .current_dir(&project_dir)
            .env("GITLAB_TOKEN", &self.settings_gitlab.glab_token)
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
        command_args.insert(0, "glab".to_string());
        Ok(command_args.join(" "))
    }

    fn tool_depends_on(&self) -> Vec<String> {
        vec![]
    }

    fn usage(&mut self) -> &mut Option<ChatUsage> {
        static mut DEFAULT_USAGE: Option<ChatUsage> = None;
        #[allow(static_mut_refs)]
        unsafe { &mut DEFAULT_USAGE }
    }

    fn confirmation_info(&self) -> Option<IntegrationConfirmation> {
        Some(self.integr_common().confirmation)
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
    if parsed_args[0] == "glab" {
        parsed_args.remove(0);
    }

    Ok(parsed_args)
}

const GITLAB_INTEGRATION_SCHEMA: &str = r#"
fields:
  glab_binary_path:
    f_type: string_long
    f_desc: "Path to the GitLab CLI binary. Leave empty to use the default 'glab' command."
    f_placeholder: "/usr/local/bin/glab"
    f_label: "GLAB Binary Path"
  glab_token:
    f_type: string_long
    f_desc: "GitLab Personal Access Token for authentication."
    f_placeholder: "glpat_xxxxxxxxxxxxxxxx"
description: |
  The GitLab integration allows interaction with GitLab repositories using the GitLab CLI.
  It provides functionality for various GitLab operations such as creating issues, merge requests, and more.
smartlinks:
  - sl_label: "Test"
    sl_chat:
      - role: "user"
        content: |
          🔧 The `gitlab` (`glab`) tool should be visible now. To test the tool, list opened merge requests for your GitLab project, and briefly describe them and express
          happiness, and change nothing. If it doesn't work or the tool isn't available, go through the usual plan in the system prompt.
available:
  on_your_laptop_possible: true
  when_isolated_possible: true
confirmation:
  ask_user_default: ["glab * delete"]
  deny_default: ["glab auth token"]
"#;
