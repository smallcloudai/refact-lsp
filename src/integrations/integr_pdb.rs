use std::sync::Arc;
use std::collections::HashMap;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::sync::{Mutex as AMutex, RwLock as ARwLock};
use tokio::process::{Command, Child, ChildStdin, ChildStdout};
use tokio::time::{timeout, Duration};
use async_trait::async_trait;
use tracing::{error, info};
use serde::{Deserialize, Serialize};

use crate::at_commands::at_commands::AtCommandsContext;
use crate::call_validation::{ContextEnum, ChatMessage};

use crate::global_context::GlobalContext;
use crate::tools::tools_description::Tool;
use serde_json::Value;

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

        let is_process_active = {
            let mut pdb_data_locked = pdb_data.lock().await;
            pdb_data_locked.sessions.get_mut(&chat_id)
                .map_or(false, |session| session.process.try_wait().is_ok())
        };

        if is_process_active {
            let output = interact_with_pdb(command, &chat_id, gcx.clone()).await?;
            let mut results = vec![];
            results.push(ContextEnum::ChatMessage(ChatMessage {
                role: "tool".to_string(),
                content: output,
                tool_calls: None,
                tool_call_id: tool_call_id.clone(),
                ..Default::default()
            }));

            return Ok((false, results));
        }

        let mut parsed_args = shell_words::split(command).map_err(|e| e.to_string())?;
        if parsed_args.is_empty() {
            return Err("Parsed command is empty".to_string());
        }

        if parsed_args.len() >= 3 && matches!(parsed_args[0].as_str(), "python" | "python2" | "python3") && parsed_args[1] == "-m" && parsed_args[2] == "pdb" {
            parsed_args.drain(0..3);
        }

        let python_command = self.integration_pdb.python_path.as_ref().unwrap_or(&"python3".to_string()).clone();
        start_pdb_session(&python_command, &parsed_args, &chat_id, gcx.clone()).await?;

        let mut results = vec![];
        results.push(ContextEnum::ChatMessage(ChatMessage {
            role: "tool".to_string(),
            content: "PDB session started. You can now send commands to the debugger.".to_string(),
            tool_calls: None,
            tool_call_id: tool_call_id.clone(),
            ..Default::default()
        }));

        Ok((false, results))
    }
}

async fn start_pdb_session(python_command: &String, parsed_args: &Vec<String>, chat_id: &String, gcx: Arc<ARwLock<GlobalContext>>) -> Result<(), String> 
{
    info!("Starting pdb session with command: {} -m pdb {:?}", python_command, parsed_args);
    let mut process = Command::new(python_command)
        .arg("-m")
        .arg("pdb")
        .args(parsed_args)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .spawn()
        .map_err(|e| {
            error!("Failed to start pdb process: {}", e);
            e.to_string()
        })?;

    let stdin = process.stdin.take().ok_or("Failed to open stdin for pdb process")?;
    let stdout = process.stdout.take().ok_or("Failed to open stdout for pdb process")?;

    let pdb_data = {
        let gcx_lock = gcx.read().await;
        gcx_lock.tools_data.pdb_data.clone()
    };
    let mut pdb_data_locked = pdb_data.lock().await;
    
    if let Some(mut old_session) = pdb_data_locked.sessions.remove(chat_id) {
        info!("Terminating old pdb session for chat_id: {}", chat_id);
        if let Err(e) = old_session.process.kill().await {
            error!("Failed to kill old pdb process: {}", e);
        }
    }

    pdb_data_locked.sessions.insert(
        chat_id.to_string(), PdbSession { process, stdin, stdout: BufReader::new(stdout) }
    );
    info!("PDB session started successfully");
    Ok(())
}

async fn interact_with_pdb(input_command: &String, chat_id: &String, gcx: Arc<ARwLock<GlobalContext>>) -> Result<String, String> {
    let output = {
        let mut output = String::new();
        {
            let pdb_data = {
                let gcx_locked = gcx.read().await;
                gcx_locked.tools_data.pdb_data.clone()
            };
            let mut pdb_data_locked = pdb_data.lock().await;
            let pdb_session = pdb_data_locked.sessions.get_mut(chat_id)
                .ok_or(format!("Error getting pdb session for chat_id: {}", chat_id))?;

            info!("Writing command to pdb stdin: {}", input_command);
            pdb_session.stdin.write_all(format!("{}\n", input_command).as_bytes()).await.map_err(|e| {
                error!("Failed to write to pdb stdin: {}", e);
                e.to_string()
            })?;
            pdb_session.stdin.flush().await.map_err(|e| {
                error!("Failed to flush pdb stdin: {}", e);
                e.to_string()
            })?;

            // Variables for timeout mechanism
            let mut received_first_line = false;
            let timeout_duration = Duration::from_millis(200);
            let mut line = String::new();

            // Read output with a timeout after the first line is received
            loop {
                let result = if received_first_line {
                    // Apply timeout only after receiving the first line
                    timeout(timeout_duration, pdb_session.stdout.read_line(&mut line)).await
                } else {
                    // No timeout for the first line
                    Ok(pdb_session.stdout.read_line(&mut line).await)
                };

                match result {
                    Ok(Ok(bytes_read)) => {
                        if bytes_read == 0 {
                            break; // No more output
                        }

                        output.push_str(&line);
                        info!("Received line from pdb stdout: {}", line);

                        // Mark that the first line was received
                        received_first_line = true;

                        // Check if the line ends with the (Pdb) prompt
                        if line.trim_end().ends_with("(Pdb)") {
                            break;
                        }

                        line.clear(); // Clear the line for the next read
                    }
                    Ok(Err(e)) => {
                        error!("Failed to read from pdb stdout: {}", e);
                        return Err(e.to_string());
                    }
                    Err(_) => {
                        // Timeout happened after receiving the first line, stop reading
                        info!("No output received for 200ms, stopping read.");
                        break;
                    }
                }
            }
        }
        output
    };

    info!("Received output from pdb stdout: {}", output);
    Ok(output)
}