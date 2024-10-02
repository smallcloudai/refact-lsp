use std::any::Any;
use std::sync::Arc;
use std::collections::HashMap;
use serde_json::Value;
use tokio::io::{AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::sync::{Mutex as AMutex, RwLock as ARwLock};
use tokio::process::{Command, Child, ChildStdin, ChildStdout, ChildStderr};
use tokio::time::{timeout, Duration};
use async_trait::async_trait;
use tracing::{error, info};
use serde::{Deserialize, Serialize};

use crate::at_commands::at_commands::AtCommandsContext;
use crate::call_validation::{ContextEnum, ChatMessage};
use crate::command_sessions::CommandSession;
use crate::global_context::GlobalContext;
use crate::tools::tools_description::Tool;

#[derive(Clone, Serialize, Deserialize, Debug)]
pub struct IntegrationPdb {
    pub python_path: Option<String>,
}
pub struct ToolPdb {
    integration_pdb: IntegrationPdb,
}

#[derive(Default)]
pub struct PdbData {
    sessions: HashMap<String, PdbSession>,
}

pub struct PdbSession {
    process: Child,
    stdin: ChildStdin,
    stdout: BufReader<ChildStdout>,
    stderr: BufReader<ChildStderr>,
}

#[async_trait]
impl CommandSession for PdbSession 
{
    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }

    async fn kill_process(&mut self) -> Result<(), String> {
        self.process.kill().await.map_err(|e| {
            error!("Failed to kill pdb process: {}", e);
            e.to_string()
        })?;
        Ok(())
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
        let command = match args.get("command") {
            Some(Value::String(s)) => s,
            Some(v) => return Err(format!("argument `command` is not a string: {:?}", v)),
            None => return Err("Missing argument `command`".to_string())
        };

        let (gcx, chat_id) = {
            let ccx_lock = ccx.lock().await;
            (ccx_lock.global_context.clone(), ccx_lock.chat_id.clone())
        };

        let mut parsed_args = shell_words::split(command).map_err(|e| e.to_string())?;
        if parsed_args.is_empty() {
            return Err("Parsed command is empty".to_string());
        }

        if parsed_args.len() >= 3 && matches!(parsed_args[0].as_str(), "python" | "python2" | "python3") && parsed_args[1] == "-m" && parsed_args[2] == "pdb" {
            parsed_args.drain(0..3);
        }

        let python_command = self.integration_pdb.python_path.as_ref().unwrap_or(&"python3".to_string()).clone();

        let is_process_active = {
            let session = {
                let gcx_locked = gcx.read().await;
                gcx_locked.command_sessions.get(&chat_id).cloned()
            };
        
            if let Some(session) = session {
                let mut session_locked = session.lock().await;
                session_locked.as_any_mut().downcast_mut::<PdbSession>()
                    .map_or(false, |pdb_session| pdb_session.process.try_wait().ok().flatten().is_none())
            } else {
                false
            }
        };

        let output = if is_process_active {
            interact_with_pdb(&parsed_args.join(" "), &chat_id, gcx.clone()).await?
        } else {
            start_pdb_session(&python_command, &parsed_args, &chat_id, gcx.clone()).await?
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
}

async fn start_pdb_session(python_command: &String, parsed_args: &Vec<String>, chat_id: &String, gcx: Arc<ARwLock<GlobalContext>>) -> Result<String, String> 
{
    info!("Starting pdb session with command: {} -m pdb {:?}", python_command, parsed_args);
    let mut process = Command::new(python_command)
        .arg("-m")
        .arg("pdb")
        .args(parsed_args)
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

    let output = read_until_pdb_or_timeout(&mut stdout, 0).await?;
    let error = read_until_pdb_or_timeout(&mut stderr, 500).await?;
    
    if let Some(old_session) = {
        let mut gcx_locked = gcx.write().await;
        gcx_locked.command_sessions.remove(&command_session_key(chat_id))
    } {
        old_session.lock().await.kill_process().await?;
    }

    let status = process.try_wait().map_err(|e| e.to_string())?;
    if status.is_none() {
        let command_session: Box<dyn CommandSession> = Box::new(PdbSession { process, stdin, stdout, stderr });
        let mut gcx_locked = gcx.write().await;
        gcx_locked.command_sessions.insert(
            command_session_key(chat_id), Arc::new(AMutex::new(command_session)) 
        );
    }
    
    Ok(format!("{}{}{}", output, "\n", error))
}

async fn interact_with_pdb(input_command: &String, chat_id: &String, gcx: Arc<ARwLock<GlobalContext>>) -> Result<String, String> 
{
    let command_session = {
        let gcx_locked = gcx.read().await;
        gcx_locked.command_sessions.get(&command_session_key(chat_id))
            .ok_or(format!("Error getting pdb session for chat_id: {}", chat_id))?
            .clone()
    };
    
    let mut command_session_locked = command_session.lock().await;
    let pdb_command_session = command_session_locked.as_any_mut().downcast_mut::<PdbSession>().ok_or("Failed to downcast to PdbSession")?;

    pdb_command_session.stdin.write_all(format!("{}\n", input_command).as_bytes()).await.map_err(|e| {
        error!("Failed to write to pdb stdin: {}", e);
        e.to_string()
    })?;
    pdb_command_session.stdin.flush().await.map_err(|e| {
        error!("Failed to flush pdb stdin: {}", e);
        e.to_string()
    })?;

    Ok(read_until_pdb_or_timeout(&mut pdb_command_session.stdout, 0).await?)
}

pub async fn read_until_pdb_or_timeout<R>(buffer: &mut R, timeout_ms: u64) -> Result<String, String>
where
    R: AsyncReadExt + Unpin,
{
    let mut output = String::new();
    let mut buf = [0u8; 1024];

    while let Ok(bytes_read) = if timeout_ms > 0 {
        match timeout(Duration::from_millis(timeout_ms), buffer.read(&mut buf)).await {
            Ok(Ok(bytes)) => Ok(bytes),
            Ok(Err(e)) => Err(e),
            Err(_) => Ok(0), // Handle timeout as 0 bytes read
        }
    } else {
        buffer.read(&mut buf).await
    } {
        if bytes_read == 0 {
            break;
        }
        output.push_str(&String::from_utf8_lossy(&buf[..bytes_read]));
        if output.trim_end().ends_with("(Pdb)") {
            break;
        }
    }

    Ok(output)
}

fn command_session_key(chat_id: &String) -> String {
    "pdb ⚡ ".to_string() + chat_id
}