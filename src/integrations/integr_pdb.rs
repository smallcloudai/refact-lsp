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

    async fn start_pdb_session(&mut self, parsed_args: Vec<String>, gcx: Arc<ARwLock<GlobalContext>>) -> Result<(), String> {
        let python_command = self.integration_pdb.python_path.clone().unwrap_or("python3".to_string());

        info!("Starting pdb session with command: {} -m pdb {:?}", python_command, parsed_args);
        let mut process = Command::new(python_command)
            .arg("-m")
            .arg("pdb")
            .args(&parsed_args)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .spawn()
            .map_err(|e| {
                error!("Failed to start pdb process: {}", e);
                e.to_string()
            })?;

        let stdin = process.stdin.take().ok_or("Failed to open stdin for pdb process")?;
        let stdout = process.stdout.take().ok_or("Failed to open stdout for pdb process")?;

        let gcx_locked = gcx.write().await;
        let mut pdb_sessions_locked = gcx_locked.pdb_sessions.lock().await;
        if pdb_sessions_locked.len() == 0 {
            pdb_sessions_locked.push(
                PdbSession {
                    process,
                    stdin,
                    stdout: BufReader::new(stdout),
                }
            );
        } else {
            return Err("Too many active pdb sessions".to_string());
        }

        info!("PDB session started successfully");
        Ok(())
    }

    async fn interact_with_pdb(&mut self, input_command: &str, gcx: Arc<ARwLock<GlobalContext>>) -> Result<String, String> {
        let output = {
            let mut output = String::new();
            {
                let gcx_locked = gcx.write().await;
                let mut pdb_sessions_locked = gcx_locked.pdb_sessions.lock().await;
                let pdb_session = pdb_sessions_locked.last_mut().ok_or("No active pdb sessions")?;

                // Check if the process is still running
                // if pdb_session.process.try_wait().is_ok() {
                //     return Err("PDB process has already exited".to_string());
                // }

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

    // async fn stop_pdb_session(&mut self) -> Result<(), String> {
    //     if let Some(process) = &mut self.process {
    //         process.kill().await.map_err(|e| e.to_string())?;
    //         self.process = None;
    //         self.stdin = None;
    //         self.stdout = None;
    //         Ok(())
    //     } else {
    //         Err("No active pdb session to stop".to_string())
    //     }
    // }
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

        let gcx = {
            let ccx_lock = ccx.lock().await;
            ccx_lock.global_context.clone()
        };

        let is_process_active = {
            let gcx_locked = gcx.read().await;
            let is_process_active = gcx_locked.pdb_sessions.lock().await.len() > 0; 
            is_process_active
        };

        // If session exists, interact with it
        if is_process_active {
            let output = self.interact_with_pdb(command, gcx.clone()).await?;
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

        // Split command into args to start a new pdb session
        let mut parsed_args = shell_words::split(command).map_err(|e| e.to_string())?;
        if parsed_args.is_empty() {
            return Err("Parsed command is empty".to_string());
        }

        // Strip python -m pdb part if present
        if parsed_args.len() >= 3 && parsed_args[0] == "python" && parsed_args[1] == "-m" && parsed_args[2] == "pdb" {
            parsed_args.drain(0..3);
        }

        self.start_pdb_session(parsed_args, gcx.clone()).await?;

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