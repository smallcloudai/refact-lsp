use std::any::Any;
use std::sync::Arc;
use std::collections::HashMap;
use std::time::SystemTime;
use std::fmt::Debug;
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
use crate::command_sessions::{CommandSession, get_session_hashmap_key};
use crate::global_context::GlobalContext;
use crate::tools::tools_description::Tool;

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

impl PdbSession {
    pub fn new(
        process: Child,
        stdin: ChildStdin,
        stdout: BufReader<ChildStdout>,
        stderr: BufReader<ChildStderr>,
    ) -> Self {
        PdbSession {
            process,
            stdin,
            stdout,
            stderr,
            last_usage_ts: SystemTime::now().duration_since(SystemTime::UNIX_EPOCH).unwrap().as_secs(),
        }
    }
}

impl CommandSession for PdbSession 
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
                gcx_locked.command_sessions.get(&get_session_hashmap_key("pdb", &chat_id)).cloned()
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
    
    {
        let mut gcx_locked = gcx.write().await;
        gcx_locked.command_sessions.remove(&get_session_hashmap_key("pdb", chat_id));
    }

    let status = process.try_wait().map_err(|e| e.to_string())?;
    if status.is_none() {
        let command_session: Box<dyn CommandSession> = Box::new(PdbSession::new(process, stdin, stdout, stderr));
        let mut gcx_locked = gcx.write().await;
        gcx_locked.command_sessions.insert(
            get_session_hashmap_key("pdb", chat_id), Arc::new(AMutex::new(command_session)) 
        );
    }
    
    Ok(format!("{}\n{}", output, error))
}

async fn interact_with_pdb(input_command: &String, chat_id: &String, gcx: Arc<ARwLock<GlobalContext>>) -> Result<String, String> 
{
    let command_session = {
        let gcx_locked = gcx.read().await;
        gcx_locked.command_sessions.get(&get_session_hashmap_key("pdb", chat_id))
            .ok_or(format!("Error getting pdb session for chat_id: {}", chat_id))?
            .clone()
    };
    
    let mut command_session_locked = command_session.lock().await;
    let pdb_session = command_session_locked.as_any_mut().downcast_mut::<PdbSession>().ok_or("Failed to downcast to PdbSession")?;

    pdb_session.stdin.write_all(format!("{}\n", input_command).as_bytes()).await.map_err(|e| {
        error!("Failed to write to pdb stdin: {}", e);
        e.to_string()
    })?;
    pdb_session.stdin.flush().await.map_err(|e| {
        error!("Failed to flush pdb stdin: {}", e);
        e.to_string()
    })?;

    let output = read_until_pdb_or_timeout(&mut pdb_session.stdout, 0).await?;
    let error = read_until_pdb_or_timeout(&mut pdb_session.stderr, 50).await?;

    Ok(format!("{}\n{}", output, error))
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