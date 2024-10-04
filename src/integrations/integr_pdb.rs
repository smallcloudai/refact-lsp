use std::any::Any;
use std::sync::Arc;
use std::collections::HashMap;
use std::time::SystemTime;
use std::fmt::Debug;
use serde_json::Value;
use tokio::io::BufReader;
use tokio::sync::{Mutex as AMutex, RwLock as ARwLock};
use tokio::process::{Command, Child, ChildStdin, ChildStdout, ChildStderr};
use tokio::time::Duration;
use async_trait::async_trait;
use tracing::{error, info};
use serde::{Deserialize, Serialize};

use crate::at_commands::at_commands::AtCommandsContext;
use crate::call_validation::{ContextEnum, ChatMessage};
use crate::integrations::sessions::{IntegrationSession, get_session_hashmap_key};
use crate::global_context::GlobalContext;
use crate::tools::tools_description::Tool;
use crate::integrations::io_helper::{first_n_chars, last_n_lines, write_to_stdin_and_flush, read_until_token_or_timeout};

const SESSION_TIMEOUT_AFTER_INACTIVITY: Duration = Duration::from_secs(30 * 60);

#[derive(Clone, Serialize, Deserialize, Debug)]
pub struct IntegrationPdb {
    pub python_path: Option<String>,
}
pub struct ToolPdb {
    integration_pdb: IntegrationPdb,
}

pub struct PdbSession {
    process: Child,
    stdin: ChildStdin,
    stdout: BufReader<ChildStdout>,
    stderr: BufReader<ChildStderr>,
    last_usage_ts: u64,
}

impl IntegrationSession for PdbSession 
{
    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }

    fn is_expired(&self) -> bool {
        let current_time = SystemTime::now().duration_since(SystemTime::UNIX_EPOCH).unwrap().as_secs();
        self.last_usage_ts + SESSION_TIMEOUT_AFTER_INACTIVITY.as_secs() < current_time
    }
}

impl ToolPdb {
    pub fn new_if_configured(integrations_value: &serde_yaml::Value) -> Option<Self> {
        let integration_pdb_value = integrations_value.get("pdb")?;

        let integration_pdb = serde_yaml::from_value::<IntegrationPdb>(integration_pdb_value.clone()).or_else(|e| {
            error!("Failed to parse integration pdb: {:?}", e);
            Err(e)
        }).ok()?;

        Some(Self { integration_pdb })
    }
}

#[async_trait]
impl Tool for ToolPdb {
    async fn tool_execute(
        &mut self,
        ccx: Arc<AMutex<AtCommandsContext>>,
        tool_call_id: &String,
        args: &HashMap<String, Value>,
    ) -> Result<(bool, Vec<ContextEnum>), String> {
        let mut command_args = parse_command_args(args)?;
        
        let (gcx, chat_id) = {
            let ccx_lock = ccx.lock().await;
            (ccx_lock.global_context.clone(), ccx_lock.chat_id.clone())
        };

        let session_hashmap_key = get_session_hashmap_key("pdb", &chat_id);
        let python_command = self.integration_pdb.python_path.clone().unwrap_or_else(|| "python3".to_string());

        let is_trying_to_open_pdb_session = command_args.len() >= 3 && 
            matches!(command_args[0].as_str(), "python" | "python2" | "python3") && 
            command_args[1] == "-m" && 
            command_args[2] == "pdb";

        let output = if is_trying_to_open_pdb_session {
            start_pdb_session(&python_command, &mut command_args, &session_hashmap_key, gcx.clone()).await?
        } else {
            interact_with_pdb(&command_args.join(" "), &session_hashmap_key, gcx.clone()).await?
        };

        Ok((false, vec![
            ContextEnum::ChatMessage(ChatMessage {
                role: "tool".to_string(),
                content: output,
                tool_calls: None,
                tool_call_id: tool_call_id.clone(),
                ..Default::default()
            })    
        ]))
    }

    fn command_to_match_against_confirm_deny(
        &self,
        args: &HashMap<String, Value>,
    ) -> Result<String, String> {
        let command_args = parse_command_args(args)?;
        Ok(command_args.join(" "))
    }
}

fn parse_command_args(args: &HashMap<String, Value>) -> Result<Vec<String>, String> {
    let command = match args.get("command") {
        Some(Value::String(s)) => s,
        Some(v) => return Err(format!("argument `command` is not a string: {:?}", v)),
        None => return Err("Missing argument `command`".to_string())
    };

    let parsed_args = shell_words::split(&command).map_err(|e| e.to_string())?;
    if parsed_args.is_empty() {
        return Err("Parsed command is empty".to_string());
    }

    Ok(parsed_args)
}

async fn start_pdb_session(python_command: &String, command_args: &mut Vec<String>, session_hashmap_key: &String, gcx: Arc<ARwLock<GlobalContext>>) -> Result<String, String> 
{
    command_args.drain(0..3);
    info!("Starting pdb session with command: {} -m pdb {:?}", python_command, command_args);
    let mut process = Command::new(python_command)
        .arg("-m")
        .arg("pdb")
        .args(command_args)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .map_err(|e| {
            error!("Failed to start pdb process: {}", e);
            e.to_string()
        })?;

    let stdin = process.stdin.take().ok_or("Failed to open stdin for pdb process")?;
    let mut stdout = BufReader::new(process.stdout.take().ok_or("Failed to open stdout for pdb process")?);
    let mut stderr = BufReader::new(process.stderr.take().ok_or("Failed to open stderr for pdb process")?);

    let output = read_until_token_or_timeout(&mut stdout, 0, "(Pdb)").await?;
    let error = read_until_token_or_timeout(&mut stderr, 500, "").await?;
    
    let exit_status = process.try_wait().map_err(|e| e.to_string())?;
    if exit_status.is_none() {
        let last_usage_ts = SystemTime::now().duration_since(SystemTime::UNIX_EPOCH).unwrap().as_secs();
        let command_session: Box<dyn IntegrationSession> = Box::new(PdbSession {process, stdin, stdout, stderr, last_usage_ts});
        gcx.write().await.integration_sessions.insert(
            session_hashmap_key.clone(), Arc::new(AMutex::new(command_session)) 
        );
    } else {
        gcx.write().await.integration_sessions.remove(session_hashmap_key);
    }
    
    Ok(format!("{}\n{}", output, error))
}

async fn interact_with_pdb(
    input_command: &String, 
    session_hashmap_key: &String, 
    gcx: Arc<ARwLock<GlobalContext>>
) -> Result<String, String> {
    let command_session = {
        let gcx_locked = gcx.read().await;
        gcx_locked.integration_sessions.get(session_hashmap_key)
            .ok_or("There is no active pdb session in this chat, perhaps a new one should be opened.")?
            .clone()
    };
    
    let mut command_session_locked = command_session.lock().await;
    let mut pdb_session = command_session_locked.as_any_mut().downcast_mut::<PdbSession>()
        .ok_or("Failed to downcast to PdbSession")?;

    let (output_main_command, error_main_command) = send_command_and_get_output_and_error(&mut pdb_session, input_command, session_hashmap_key, gcx.clone()).await?;
    let (output_list, error_list) = send_command_and_get_output_and_error(&mut pdb_session, "list", session_hashmap_key, gcx.clone()).await?;
    let (output_where, error_where) = send_command_and_get_output_and_error(&mut pdb_session, "where", session_hashmap_key, gcx.clone()).await?;
    let (output_locals, error_locals) = send_command_and_get_output_and_error(&mut pdb_session, "p {k: v for k, v in locals().items() if not k.startswith('__')}", session_hashmap_key, gcx.clone()).await?;

    pdb_session.last_usage_ts = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap()
        .as_secs();
    
    Ok(format!(
        "Command output:\n{}\n{}Extra context:\nlist output:\n{}{}\nwhere output:\n{}{}locals output:\n{}{}",
        output_main_command,
        format_error("Command error", &error_main_command),
        output_list,
        format_error("list error", &error_list),
        last_n_lines(&output_where, 5),
        format_error("where error", &error_where),
        first_n_chars(&output_locals, 1000),
        format_error("locals error", &error_locals),
    ))
}

async fn send_command_and_get_output_and_error(pdb_session: &mut PdbSession, input_command: &str, session_hashmap_key: &str, gcx: Arc<ARwLock<GlobalContext>>) -> Result<(String, String), String>
{
    write_to_stdin_and_flush(&mut pdb_session.stdin, input_command).await?;
    let output = read_until_token_or_timeout(&mut pdb_session.stdout, 0, "(Pdb)").await?;
    let error = read_until_token_or_timeout(&mut pdb_session.stderr, 50, "").await?;

    let exit_status = pdb_session.process.try_wait().map_err(|e| e.to_string())?;
    if let Some(exit_status) = exit_status {
        gcx.write().await.integration_sessions.remove(session_hashmap_key);
        return Err(format!("Pdb process exited with status: {:?}", exit_status));
    }

    Ok((output, error))
}

fn format_error(error_title: &str, error: &str) -> String 
{
    if !error.is_empty() {
        format!("{}:\n{}\n", error_title, error)
    } else {
        "".to_string()
    }
}