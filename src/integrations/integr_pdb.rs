use std::sync::Arc;
use std::collections::HashMap;
use serde_json::Value;
use tokio::io::{AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::sync::Mutex as AMutex;
use tokio::process::{Command, Child, ChildStdin, ChildStdout, ChildStderr};
use tokio::time::{timeout, Duration};
use async_trait::async_trait;
use tracing::{error, info};
use serde::{Deserialize, Serialize};

use crate::at_commands::at_commands::AtCommandsContext;
use crate::call_validation::{ContextEnum, ChatMessage};
use crate::tools::tools_description::Tool;

const END_OF_LINE: &str = if cfg!(windows) { "\r\n" } else { "\n" };

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
        let pdb_data = {
            let gcx_locked = gcx.read().await;
            gcx_locked.tools_data.pdb_data.clone()
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
            let mut pdb_data_locked = pdb_data.lock().await;
            pdb_data_locked.sessions.get_mut(&chat_id)
                .map_or(false, |session| matches!(session.process.try_wait(), Ok(None)))
        };

        let output = if is_process_active {
            interact_with_pdb(&parsed_args.join(" "), &chat_id, pdb_data.clone()).await?
        } else {
            start_pdb_session(&python_command, &parsed_args, &chat_id, pdb_data.clone()).await?
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

async fn start_pdb_session(python_command: &String, parsed_args: &Vec<String>, chat_id: &String, pdb_data: Arc<AMutex<PdbData>>) -> Result<String, String> 
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

    let mut pdb_data_locked = pdb_data.lock().await;
    
    if let Some(mut old_session) = pdb_data_locked.sessions.remove(chat_id) {
        info!("Terminating old pdb session for chat_id: {}", chat_id);
        if let Err(e) = old_session.process.kill().await {
            error!("Failed to kill old pdb process: {}", e);
        }
    }

    let output = read_until_pdb_or_timeout(&mut stdout, 0).await?;
    let error = read_until_pdb_or_timeout(&mut stderr, 500).await?;

    let status = process.try_wait().map_err(|e| e.to_string())?;
    if status.is_none() {
        pdb_data_locked.sessions.insert(
            chat_id.to_string(), PdbSession { process, stdin, stdout, stderr }
        );
    }
    
    Ok(format!("{}{}{}", output, END_OF_LINE, error))
}

async fn interact_with_pdb(input_command: &String, chat_id: &String, pdb_data: Arc<AMutex<PdbData>>) -> Result<String, String> 
{
    let mut pdb_data_locked = pdb_data.lock().await;
    let pdb_session = pdb_data_locked.sessions.get_mut(chat_id)
        .ok_or(format!("Error getting pdb session for chat_id: {}", chat_id))?;

    pdb_session.stdin.write_all(format!("{}\n", input_command).as_bytes()).await.map_err(|e| {
        error!("Failed to write to pdb stdin: {}", e);
        e.to_string()
    })?;
    pdb_session.stdin.flush().await.map_err(|e| {
        error!("Failed to flush pdb stdin: {}", e);
        e.to_string()
    })?;

    Ok(read_until_pdb_or_timeout(&mut pdb_session.stdout, 0).await?)
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