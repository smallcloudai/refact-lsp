use std::path::PathBuf;
use std::collections::HashMap;
use std::sync::Arc;
use serde::Deserialize;
use serde::Serialize;
use serde_json::Value;
use tokio::sync::Mutex as AMutex;
use async_trait::async_trait;
use std::process::Stdio;
use tokio::process::Command;

use crate::at_commands::at_commands::AtCommandsContext;
use crate::tools::tools_description::{ToolParam, Tool, ToolDesc, MatchConfirmDeny, MatchConfirmDenyResult};
use crate::call_validation::{ChatMessage, ChatContent, ContextEnum};
use crate::postprocessing::pp_command_output::CmdlineOutputFilter;
use crate::integrations::integr_abstract::{IntegrationCommon, IntegrationTrait};
use crate::integrations::setting_up_integrations::YamlError;
use crate::tools::tools_execute::command_should_be_denied;


#[derive(Deserialize, Serialize, Clone, Default)]
pub struct SettingsShell {
    #[serde(default)]
    pub timeout: String,
    #[serde(default)]
    pub output_filter: CmdlineOutputFilter,
}

#[derive(Default)]
pub struct ToolShell {
    pub common: IntegrationCommon,
    pub cfg: SettingsShell,
}

impl IntegrationTrait for ToolShell {
    fn as_any(&self) -> &dyn std::any::Any { self }

    fn integr_schema(&self) -> &str
    {
        SHELL_INTEGRATION_SCHEMA
    }

    fn integr_settings_apply(&mut self, value: &Value) -> Result<(), String> {
        match serde_json::from_value::<SettingsShell>(value.clone()) {
            Ok(x) => self.cfg = x,
            Err(e) => {
                tracing::error!("Failed to apply settings: {}\n{:?}", e, value);
                return Err(e.to_string());
            }
        }
        match serde_json::from_value::<IntegrationCommon>(value.clone()) {
            Ok(x) => self.common = x,
            Err(e) => {
                tracing::error!("Failed to apply common settings: {}\n{:?}", e, value);
                return Err(e.to_string());
            }
        }
        Ok(())
    }

    fn integr_settings_as_json(&self) -> Value {
        serde_json::to_value(&self.cfg).unwrap()
    }

    fn integr_common(&self) -> IntegrationCommon {
        self.common.clone()
    }

    fn integr_upgrade_to_tool(&self, _integr_name: &str) -> Box<dyn Tool + Send> {
        Box::new(ToolShell {
            common: self.common.clone(),
            cfg: self.cfg.clone(),
        }) as Box<dyn Tool + Send>
    }
}

#[async_trait]
impl Tool for ToolShell {
    fn as_any(&self) -> &dyn std::any::Any { self }

    async fn tool_execute(
        &mut self,
        ccx: Arc<AMutex<AtCommandsContext>>,
        tool_call_id: &String,
        args: &HashMap<String, Value>,
    ) -> Result<(bool, Vec<ContextEnum>), String> {
        let (command, workdir) = parse_args(args)?;
        let timeout = self.cfg.timeout.parse::<u64>().unwrap_or(10);

        let gcx = ccx.lock().await.global_context.clone();
        let mut error_log = Vec::<YamlError>::new();
        let env_variables = crate::integrations::setting_up_integrations::get_vars_for_replacements(gcx.clone(), &mut error_log).await;
        let project_dirs = crate::files_correction::get_project_dirs(gcx.clone()).await;

        let tool_output = execute_shell_command(
            &command,
            &workdir,
            timeout,
            &self.cfg.output_filter,
            &env_variables,
            project_dirs
        ).await?;

        let result = vec![ContextEnum::ChatMessage(ChatMessage {
            role: "tool".to_string(),
            content: ChatContent::SimpleText(tool_output),
            tool_calls: None,
            tool_call_id: tool_call_id.clone(),
            ..Default::default()
        })];

        Ok((false, result))
    }

    fn tool_depends_on(&self) -> Vec<String> {
        vec![]
    }

    fn tool_description(&self) -> ToolDesc {
        ToolDesc {
            name: "shell".to_string(),
            agentic: true,
            experimental: false,
            description: "Execute a single command, using the \"sh\" on unix-like systems and \"cmd.exe\" on windows. Use it for one-time tasks like dependencies installation. Don't call this unless you have to. Not suitable for regular work because it requires a confirmation at each step.".to_string(),
            parameters: vec![
                ToolParam {
                    name: "command".to_string(),
                    param_type: "string".to_string(),
                    description: "shell command to execute".to_string(),
                },
                ToolParam {
                    name: "workdir".to_string(),
                    param_type: "string".to_string(),
                    description: "workdir for the command".to_string(),
                },
            ],
            parameters_required: vec![
                "command".to_string(),
                "workdir".to_string(),
            ],
        }
    }

    fn match_against_confirm_deny(
        &self,
        args: &HashMap<String, Value>
    ) -> Result<MatchConfirmDeny, String> {
        let command_to_match = self.command_to_match_against_confirm_deny(&args).map_err(|e| {
            format!("Error getting tool command to match: {}", e)
        })?;
        if command_to_match.is_empty() {
            return Err("Empty command to match".to_string());
        }
        if let Some(rules) = &self.confirmation_info() {
            let (is_denied, deny_rule) = command_should_be_denied(&command_to_match, &rules.deny);
            if is_denied {
                return Ok(MatchConfirmDeny {
                    result: MatchConfirmDenyResult::DENY,
                    command: command_to_match.clone(),
                    rule: deny_rule.clone(),
                });
            }
        }
        // NOTE: do not match command if not denied, always wait for confirmation from user
        Ok(MatchConfirmDeny {
            result: MatchConfirmDenyResult::CONFIRMATION,
            command: command_to_match.clone(),
            rule: "*".to_string(),
        })
    }

    fn command_to_match_against_confirm_deny(
        &self,
        args: &HashMap<String, Value>,
    ) -> Result<String, String> {
        let (command, _) = parse_args(args)?;
        Ok(command)
    }
}

pub async fn execute_shell_command(
    command: &str,
    workdir: &str,
    timeout: u64,
    output_filter: &CmdlineOutputFilter,
    env_variables: &HashMap<String, String>,
    project_dirs: Vec<PathBuf>,
) -> Result<String, String> {
    let shell = if cfg!(target_os = "windows") { "cmd.exe" } else { "sh" };
    let shell_arg = if cfg!(target_os = "windows") { "/C" } else { "-c" };
    let mut cmd = Command::new(shell);

    if workdir.is_empty() {
        if let Some(first_project_dir) = project_dirs.first() {
            cmd.current_dir(first_project_dir);
        } else {
            tracing::warn!("no working directory, using whatever directory this binary is run :/");
        }
    } else {
        cmd.current_dir(workdir);
    }

    for (key, value) in env_variables {
        cmd.env(key, value);
    }

    cmd.arg(shell_arg).arg(command);
    cmd.stdout(Stdio::piped()).stderr(Stdio::piped());

    let output = tokio::time::timeout(tokio::time::Duration::from_secs(timeout), cmd.output())
        .await
        .map_err(|_| format!("Command timed out after {} seconds", timeout))?
        .map_err(|e| format!("Failed to execute command: {}", e))?;

    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).to_string();

    let filtered_stdout = crate::postprocessing::pp_command_output::output_mini_postprocessing(output_filter, &stdout);
    let filtered_stderr = crate::postprocessing::pp_command_output::output_mini_postprocessing(output_filter, &stderr);

    if !filtered_stderr.is_empty() {
        return Err(filtered_stderr);
    }

    Ok(filtered_stdout)
}

fn parse_args(args: &HashMap<String, Value>) -> Result<(String, String), String> {
    let command = match args.get("command") {
        Some(Value::String(s)) => {
            if s.is_empty() {
                return Err("Command is empty".to_string());
            } else {
                s.clone()
            }
        },
        Some(v) => return Err(format!("argument `command` is not a string: {:?}", v)),
        None => return Err("Missing argument `command`".to_string())
    };

    let workdir = match args.get("workdir") {
        Some(Value::String(s)) => {
            if s.is_empty() {
                "".to_string()
            } else {
                let workdir = PathBuf::from(s.clone());
                if !workdir.exists() {
                    return Err("Workdir doesn't exist".to_string());
                } else {
                    s.clone()
                }
            }
        },
        Some(v) => return Err(format!("argument `workdir` is not a string: {:?}", v)),
        None => "".to_string()
    };

    Ok((command, workdir))
}

pub const SHELL_INTEGRATION_SCHEMA: &str = r#"
fields:
  timeout:
    f_type: string_short
    f_desc: "The command must immediately return the results, it can't be interactive. If the command runs for too long, it will be terminated and stderr/stdout collected will be presented to the model."
    f_default: "10"
  output_filter:
    f_type: "output_filter"
    f_desc: "The output from the command can be long or even quasi-infinite. This section allows to set limits, prioritize top or bottom, or use regexp to show the model the relevant part."
    f_extra: true
description: |
  Allows to execute any command line tool with confirmation from the chat itself.
available:
  on_your_laptop_possible: true
  when_isolated_possible: true
confirmation:
  ask_user_default: ["*"]
  deny_default: ["sudo*"]
"#;