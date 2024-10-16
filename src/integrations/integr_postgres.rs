use crate::tools::tools_description::Tool;
use crate::at_commands::at_commands::AtCommandsContext;
use crate::call_validation::{ChatMessage, ChatUsage};
use crate::call_validation::ContextEnum;
use async_trait::async_trait;
use serde_json::Value;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::Mutex as AMutex;
use tokio::process::Command;
use serde_yaml;
use std::path::PathBuf;
use regex::Regex;

pub struct ToolPostgres {
    connection_string: String,
    psql_binary_path: PathBuf,
}

impl ToolPostgres {
    pub fn new_if_configured(integrations_value: &serde_yaml::Value) -> Option<Self> {
        let postgres = integrations_value.get("postgres")?;
        let connection_string = postgres.get("connection_string")?.as_str()?.to_string();
        let psql_binary_path = postgres.get("psql_binary_path")?.as_str()?;

        Some(ToolPostgres {
            connection_string,
            psql_binary_path: PathBuf::from(psql_binary_path),
        })
    }

    fn quote_identifier(s: &str) -> String {
        format!("\"{}\"", s.replace("\"", "\"\""))
    }

    fn sanitize_identifier(s: &str) -> Result<String, String> {
        if s == "*" {
            return Ok(s.to_string());
        }
        
        // Split the identifier into parts (for schema.table format)
        let parts: Vec<&str> = s.split('.').collect();
        
        let re = Regex::new(r"^[a-zA-Z_][a-zA-Z0-9_]*$").unwrap();
        
        let sanitized_parts: Result<Vec<String>, String> = parts.iter()
            .map(|part| {
                if re.is_match(part) {
                    Ok(part.to_string())
                } else {
                    Err(format!("Invalid identifier part: {}", part))
                }
            })
            .collect();
        
        sanitized_parts.map(|parts| parts.join("."))
    }

    fn sanitize_and_quote_columns(columns: &str) -> Result<String, String> {
        columns.split(',')
            .map(|col| {
                let trimmed = col.trim();
                if trimmed == "*" {
                    Ok("*".to_string())
                } else {
                    Self::sanitize_identifier(trimmed).map(|arg0: String| Self::quote_identifier(&arg0))
                }
            })
            .collect::<Result<Vec<_>, _>>()
            .map(|cols| cols.join(", "))
    }

    fn sanitize_where_clause(where_clause: &str) -> Result<String, String> {
        // This is a basic implementation. In a real-world scenario, you'd want to use a proper SQL parser.
        // For now, we'll just check for some common SQL injection patterns.
        let forbidden_patterns = [
            "--", ";", "DROP", "DELETE", "INSERT", "UPDATE", "ALTER", "CREATE", "TRUNCATE"
        ];

        for pattern in &forbidden_patterns {
            if where_clause.to_uppercase().contains(pattern) {
                return Err(format!("Forbidden pattern '{}' found in WHERE clause", pattern));
            }
        }

        // Remove any multiple spaces and trim
        let sanitized = where_clause.split_whitespace().collect::<Vec<&str>>().join(" ");
        Ok(sanitized)
    }

    fn construct_safe_query(
        columns: &str,
        table: &str,
        where_clause: &str,
        limit: Option<u64>,
    ) -> Result<String, String> {
        let sanitized_columns = Self::sanitize_and_quote_columns(columns)?;
        let sanitized_table = Self::sanitize_identifier(table)?;
        let quoted_table = sanitized_table.split('.')
            .map(Self::quote_identifier)
            .collect::<Vec<_>>()
            .join(".");

        let mut query = format!("SELECT {} FROM {}", sanitized_columns, quoted_table);

        if !where_clause.is_empty() {
            let sanitized_where = Self::sanitize_where_clause(where_clause)?;
            query.push_str(" WHERE ");
            query.push_str(&sanitized_where);
        }

        if let Some(limit_value) = limit {
            query.push_str(&format!(" LIMIT {}", limit_value));
        }

        Ok(query)
    }

    async fn run_psql_command(&self, query: &str) -> Result<String, String> {
        let mut cmd = Command::new(&self.psql_binary_path);
        cmd.arg(&self.connection_string)
            .arg("--tuples-only")
            .arg("--no-align")
            .arg("-v")
            .arg("ON_ERROR_STOP=1")
            .arg("-c")
            .arg(query);

        let output = cmd.output().await
            .map_err(|e| format!("Failed to execute psql command: {}", e))?;

        if output.status.success() {
            Ok(String::from_utf8_lossy(&output.stdout).to_string())
        } else {
            Err(format!("psql command failed: {}", String::from_utf8_lossy(&output.stderr)))
        }
    }
}

#[async_trait]
impl Tool for ToolPostgres {
    async fn tool_execute(
        &mut self,
        _ccx: Arc<AMutex<AtCommandsContext>>,
        tool_call_id: &String,
        args: &HashMap<String, Value>,
    ) -> Result<(bool, Vec<ContextEnum>), String> {
        let table = args.get("table")
            .and_then(Value::as_str)
            .ok_or("Missing table")?;
        let columns = args.get("columns")
            .and_then(Value::as_str)
            .ok_or("Missing columns")?;
        let where_clause = args.get("where_clause")
            .and_then(Value::as_str)
            .unwrap_or("");
        let limit = args.get("limit")
            .and_then(Value::as_u64);

        let query = Self::construct_safe_query(columns, table, where_clause, limit)?;
        let result = self.run_psql_command(&query).await?;

        let mut results = vec![];
        results.push(ContextEnum::ChatMessage(ChatMessage {
            role: "tool".to_string(),
            content: result,
            tool_calls: None,
            tool_call_id: tool_call_id.clone(),
            ..Default::default()
        }));
        Ok((true, results))
    }

    fn command_to_match_against_confirm_deny(
        &self,
        _args: &HashMap<String, Value>,
    ) -> Result<String, String> {
        Ok("postgres".to_string())
    }

    fn tool_depends_on(&self) -> Vec<String> {
        vec![]
    }

    fn usage(&mut self) -> &mut Option<ChatUsage> {
        static mut DEFAULT_USAGE: Option<ChatUsage> = None;
        #[allow(static_mut_refs)]
        unsafe { &mut DEFAULT_USAGE }
    }
}

