use std::any::Any;
use std::collections::HashMap;
use std::sync::Arc;
use std::process::Stdio;
use indexmap::IndexMap;
use tokio::sync::{Mutex as AMutex, RwLock as ARwLock};
use tokio::time::{Duration, sleep, Instant};
use tokio::io::BufReader;
use serde::Deserialize;
use async_trait::async_trait;
use tokio::process::Command;
use tracing::{info, warn, error};

use crate::at_commands::at_commands::AtCommandsContext;
use crate::tools::tools_description::{ToolParam, Tool, ToolDesc};
use crate::call_validation::{ChatMessage, ChatContent, ContextEnum};
use crate::global_context::GlobalContext;
use crate::integrations::process_io_utils::{kill_process_and_children, read_until_token_or_timeout, wait_until_port_gets_occupied};
use crate::integrations::sessions::IntegrationSession;
use crate::postprocessing::pp_command_output::{CmdlineOutputFilter, output_mini_postprocessing};


#[derive(Deserialize, Clone)]
struct CmdlineToolBackground {
    #[serde(default)]
    startup_wait_port: Option<u16>,
    #[serde(default)]
    startup_wait_keyword: Option<String>,
    #[serde(default)]
    startup_timeout: u64,
}

#[derive(Deserialize, Clone)]
struct CmdToolBlocking {
    timeout: u64,
}

#[derive(Deserialize)]
struct CmdlineToolConfig {
    description: String,
    parameters: Vec<ToolParam>,
    parameters_required: Option<Vec<String>>,
    command: String,
    command_workdir: String,
    #[serde(default)]
    blocking: Option<CmdToolBlocking>,
    #[serde(default)]
    background: Option<CmdlineToolBackground>,
    #[serde(default)]
    output_filter: CmdlineOutputFilter,
}

pub struct ToolCmdline {
    name: String,
    cfg: CmdlineToolConfig,
}

pub fn cmdline_tool_from_yaml_value(cfg_cmdline_value: &serde_yaml::Value) -> Result<IndexMap<String, Arc<AMutex<Box<dyn Tool + Send>>>>, String> {
    let mut result = IndexMap::new();
    let cfgmap = match serde_yaml::from_value::<IndexMap<String, CmdlineToolConfig>>(cfg_cmdline_value.clone()) {
        Ok(cfgmap) => cfgmap,
        Err(e) => {
            let location = e.location().map(|loc| format!(" at line {}, column {}", loc.line(), loc.column())).unwrap_or_default();
            return Err(format!("failed to parse cmdline section: {:?}{}", e, location));
        }
    };
    for (c_name, mut c_cmd_tool) in cfgmap.into_iter() {
        if c_cmd_tool.background.is_some() {
            c_cmd_tool.parameters.push(ToolParam {
                name: "action".to_string(),
                param_type: "string".to_string(),
                description: "(start=default |stop | restart | status | communicate)".to_string(),
            });
        }
        let tool = Arc::new(AMutex::new(Box::new(
            ToolCmdline {
                name: c_name.clone(),
                cfg: c_cmd_tool,
            }
        ) as Box<dyn Tool + Send>));
        result.insert(c_name, tool);
    }
    Ok(result)
}

pub struct CmdlineSession {
    cmdline_string: String,
    cmdline_process: tokio::process::Child,
    #[allow(dead_code)]
    cmdline_stdout: BufReader<tokio::process::ChildStdout>,
    #[allow(dead_code)]
    cmdline_stderr: BufReader<tokio::process::ChildStderr>,
}

impl IntegrationSession for CmdlineSession {
    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
    fn is_expired(&self) -> bool { false }
}

fn _replace_args(x: &str, args_str: &HashMap<String, String>) -> String {
    let mut result = x.to_string();
    for (key, value) in args_str {
        result = result.replace(&format!("%{}%", key), value);
    }
    result
}

fn format_output(stdout_out: &str, stderr_out: &str) -> String {
    let mut out = String::new();
    if !stdout_out.is_empty() {
        out.push_str(&format!("STDOUT:\n{}\n", stdout_out));
    }
    if !stderr_out.is_empty() {
        out.push_str(&format!("STDERR:\n{}\n", stderr_out));
    }
    out
}

async fn create_command_from_string(
    cmd_string: &str,
    command_workdir: &String,
) -> Result<Command, String> {
    let command_args = shell_words::split(cmd_string)
        .map_err(|e| format!("Failed to parse command: {}", e))?;
    if command_args.is_empty() {
        return Err("Command is empty after parsing".to_string());
    }
    let mut cmd = Command::new(&command_args[0]);
    if command_args.len() > 1 {
        cmd.args(&command_args[1..]);
    }
    cmd.current_dir(command_workdir);
    Ok(cmd)
}

async fn execute_blocking_command(
    command: &str,
    cfg: &CmdToolBlocking,
    command_workdir: &String,
    output_filter: &CmdlineOutputFilter,
) -> Result<String, String> {
    info!("EXEC: {command}, workdir: '{command_workdir}'");
    let command_future = async {
        let mut cmd = create_command_from_string(command, command_workdir).await?;
        let start_time = Instant::now();
        let result = cmd
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .output()
            .await;
        let duration = start_time.elapsed();
        info!("EXEC: {} finished in {:?}", command, duration);

        let output = match result {
            Ok(output) => output,
            Err(e) => {
                let msg = format!("cannot run command: '{}'. workdir: '{}'. Error: {}", &command, command_workdir, e);
                error!("{msg}");
                return Err(msg);
            }
        };

        let stdout = output_mini_postprocessing(output_filter, &String::from_utf8_lossy(&output.stdout).to_string());
        let stderr = output_mini_postprocessing(output_filter, &String::from_utf8_lossy(&output.stderr).to_string());

        let mut out = format_output(&stdout, &stderr);
        let exit_code = output.status.code().unwrap_or_default();
        out.push_str(&format!("command was running {:.3}s, finished with exit code {exit_code}\n", duration.as_secs_f64()));
        Ok(out)
    };

    let timeout_duration = Duration::from_secs(cfg.timeout);
    let result = tokio::time::timeout(timeout_duration, command_future).await;

    match result {
        Ok(res) => res,
        Err(_) => Err(format!("command timed out after {:?}", timeout_duration)),
    }
}

async fn get_stdout_and_stderr(
    timeout_ms: u64,
    stdout: &mut BufReader<tokio::process::ChildStdout>,
    stderr: &mut BufReader<tokio::process::ChildStderr>,
) -> Result<(String, String), String> {
    let stdout_out = read_until_token_or_timeout(stdout, timeout_ms, "").await?;
    let stderr_out = read_until_token_or_timeout(stderr, timeout_ms, "").await?;
    Ok((stdout_out, stderr_out))
}

async fn read_until_text_in_output_or_timeout(
    timeout: Duration,
    stdout: &mut BufReader<tokio::process::ChildStdout>,
    stderr: &mut BufReader<tokio::process::ChildStderr>,
    text: &str,
) -> Result<(String, String), String> {

    let start = Instant::now();
    let step_duration = Duration::from_millis(100);
    let mut stdout_text = String::new();
    let mut stderr_text = String::new();

    while start.elapsed() < timeout {
        let stdout_out = read_until_token_or_timeout(stdout, step_duration.as_millis() as u64, text).await?;
        let stderr_out = read_until_token_or_timeout(stderr, step_duration.as_millis() as u64, text).await?;
        stdout_text.push_str(&stdout_out);
        stderr_text.push_str(&stderr_out);

        if !text.is_empty() && format!("{}{}", stdout_text, stderr_text).contains(text) {
            return Ok((stdout_text, stderr_text));
        }

        sleep(step_duration).await;
    }
    Err(format!("Timeout reached. Output:\nSTDOUT:{}\nSTDERR:\n{}", stdout_text, stderr_text))
}

async fn execute_background_command(
    gcx: Arc<ARwLock<GlobalContext>>,
    service_name: &str,
    command_str: &str,
    command_workdir: &String,
    bg_cfg: CmdlineToolBackground,
    action: &str,
) -> Result<String, String> {
    let session_key = format!("custom_service_{service_name}");
    let session_mb = gcx.read().await.integration_sessions.get(&session_key).cloned();
    let mut command_str = command_str.to_string();

    if session_mb.is_some() && action == "start" {
        return Ok(format!("the service '{service_name}' is running"));
    }

    if session_mb.is_none() && (action == "status" || action == "communicate") {
        return Err(format!("cannot execute this action on service '{service_name}'. Reason: service '{service_name}' is not running.\n"));
    }

    if action == "restart" || action == "stop" || action == "status" || action == "communicate" {
        let session = session_mb.clone().unwrap();
        let mut session_lock = session.lock().await;
        let session = session_lock.as_any_mut().downcast_mut::<CmdlineSession>()
            .ok_or("Failed to downcast CmdlineSession".to_string())?;

        if action == "communicate" {
            let (stdout_out, stderr_out) = get_stdout_and_stderr(100, &mut session.cmdline_stdout, &mut session.cmdline_stderr).await?;
            return Ok(format_output(&stdout_out, &stderr_out));
        }
        if action == "status" {
            return Ok(format!("service '{service_name}' is running.\n"));
        }
        kill_process_and_children(&session.cmdline_process, service_name).await
            .map_err(|e| format!("Failed to kill service '{service_name}'. Error: {}", e))?;
        command_str = session.cmdline_string.clone();
        drop(session_lock);
        gcx.write().await.integration_sessions.remove(&session_key);

        if action == "stop" {
            return Ok(format!("service '{service_name}' is stopped.\n"));
        }
    }

    if let Some(wait_port) = bg_cfg.startup_wait_port {
        if let Ok(_) = wait_until_port_gets_occupied(wait_port, &Duration::from_millis(1)).await {
            return Err(format!("port '{}' is already occupied", wait_port));
        }
    }

    let output = {
        info!("EXEC: {command_str}, workdir: '{command_workdir}'");
        let mut command = create_command_from_string(&command_str, command_workdir).await?;
        let mut process = command
           .stdout(Stdio::piped())
           .stderr(Stdio::piped())
           .spawn()
           .map_err(|e| format!("failed to create process: {e}"))?;

        let mut stdout_reader = BufReader::new(process.stdout.take().ok_or("Failed to open stdout")?);
        let mut stderr_reader = BufReader::new(process.stderr.take().ok_or("Failed to open stderr")?);

        let wait_timeout = Duration::from_secs(bg_cfg.startup_timeout);

        // todo: does not work for npm run
        let (stdout_out, stderr_out) = if let Some(wait_port) = bg_cfg.startup_wait_port {
            let resp = wait_until_port_gets_occupied(wait_port, &wait_timeout).await;
            let (s1, e1) = get_stdout_and_stderr(100, &mut stdout_reader, &mut stderr_reader).await?;
            resp?;
            (s1, e1)
        } else {
            read_until_text_in_output_or_timeout(
                wait_timeout, &mut stdout_reader, &mut stderr_reader,
                bg_cfg.startup_wait_keyword.clone().unwrap_or_default().as_str()
            ).await?
        };

        let out = format_output(&stdout_out, &stderr_out);

        let exit_status = process.try_wait().map_err(|e| e.to_string())?;
        if exit_status.is_some() {
            let status = exit_status.unwrap().code().unwrap();
            warn!("service process exited with status: {:?}. Output:\n{out}", status);
            return Err(format!("service process exited with status: {:?}; Output:\n{out}", status));
        }

        let session: Box<dyn IntegrationSession> = Box::new(CmdlineSession {
            cmdline_process: process,
            cmdline_string: command_str,
            cmdline_stdout: stdout_reader,
            cmdline_stderr: stderr_reader,
        });
        gcx.write().await.integration_sessions.insert(session_key.to_string(), Arc::new(AMutex::new(session)));

        out
    };

    return Ok(format!("service '{service_name}' is up and running in a background:\n{output}"));
}

#[async_trait]
impl Tool for ToolCmdline {
    async fn tool_execute(
        &mut self,
        ccx: Arc<AMutex<AtCommandsContext>>,
        tool_call_id: &String,
        args: &HashMap<String, serde_json::Value>,
    ) -> Result<(bool, Vec<ContextEnum>), String> {
        let gcx = ccx.lock().await.global_context.clone();

        let mut args_str: HashMap<String, String> = HashMap::new();
        let valid_params: Vec<String> = self.cfg.parameters.iter().map(|p| p.name.clone()).collect();

        for (k, v) in args.iter() {
            if !valid_params.contains(k) {
                return Err(format!("Unexpected argument `{}`", k));
            }
            match v {
                serde_json::Value::String(s) => { args_str.insert(k.clone(), s.clone()); },
                _ => return Err(format!("argument `{}` is not a string: {:?}", k, v)),
            }
        }

        for param in &self.cfg.parameters {
            if self.cfg.parameters_required.as_ref().map_or(false, |req| req.contains(&param.name)) && !args_str.contains_key(&param.name) {
                return Err(format!("Missing required argument `{}`", param.name));
            }
        }

        let command = _replace_args(self.cfg.command.as_str(), &args_str);
        let workdir = _replace_args(self.cfg.command_workdir.as_str(), &args_str);

        let resp = if let Some(background_cfg) = &self.cfg.background {
            let action = args_str.get("action").cloned().unwrap_or("start".to_string());
            if !["start", "restart", "stop", "status", "communicate"].contains(&action.as_str()) {
                return Err("Tool call is invalid. Param 'action' must be one of 'start', 'restart', 'stop', 'status', 'communicate'. Try again".to_string());
            }
            execute_background_command(
                gcx, &self.name, &command, &workdir, background_cfg.clone(), action.as_str()
            ).await

        } else if let Some(blocking_cfg) = &self.cfg.blocking {
            execute_blocking_command(&command, &blocking_cfg, &workdir, &self.cfg.output_filter).await

        } else {
            Err(format!("background command '{}' has invalid configuration. One of (blocking | background) is required. Must be fixed by user", command))
        }?;

        let result = vec![ContextEnum::ChatMessage(ChatMessage {
            role: "tool".to_string(),
            content: ChatContent::SimpleText(resp),
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
        let parameters_required = self.cfg.parameters_required.clone().unwrap_or_else(|| {
            self.cfg.parameters.iter().map(|param| param.name.clone()).collect()
        });
        ToolDesc {
            name: self.name.clone(),
            agentic: true,
            experimental: false,
            description: self.cfg.description.clone(),
            parameters: self.cfg.parameters.clone(),
            parameters_required,
        }
    }
}
