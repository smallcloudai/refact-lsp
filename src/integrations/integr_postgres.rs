use crate::at_commands::at_commands::AtCommandsContext;
use crate::call_validation::ContextEnum;
use crate::call_validation::{ChatContent, ChatMessage, ChatUsage};
use crate::tools::tools_description::Tool;
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use serde_yaml;
use std::collections::HashMap;
use std::sync::Arc;
use schemars::JsonSchema;
use tokio::process::Command;
use tokio::sync::Mutex as AMutex;
use crate::integrations::integr::{json_schema, Integration};


#[derive(Clone, Serialize, Deserialize, Debug, JsonSchema, Default)]
pub struct IntegrationPostgres {
    #[schemars(description = "Path to the psql binary.")]
    pub psql_binary_path: Option<String>,
    #[schemars(description = "Connection string for the PSQL database.")]
    pub connection_string: String,
}

#[derive(Default)]
pub struct ToolPostgres {
    pub integration_postgres: IntegrationPostgres,
}

impl Integration for ToolPostgres {
    fn integr_name(&self) -> String {
        "postgres".to_string()
    }

    fn integr_update_settings(&mut self, value: &Value) -> Result<(), String> {
        let integration_postgres = serde_json::from_value::<IntegrationPostgres>(value.clone())
            .map_err(|e|e.to_string())?;
        self.integration_postgres = integration_postgres;
        Ok(())
    }

    fn integr_yaml2json(&self, value: &serde_yaml::Value) -> Result<Value, String> {
        let integration_github = serde_yaml::from_value::<IntegrationPostgres>(value.clone()).map_err(|e| {
            let location = e.location().map(|loc| format!(" at line {}, column {}", loc.line(), loc.column())).unwrap_or_default();
            format!("{}{}", e.to_string(), location)
        })?;
        serde_json::to_value(&integration_github).map_err(|e| e.to_string())
    }

    fn integr_upgrade_to_tool(&self) -> Box<dyn Tool + Send> {
        Box::new(ToolPostgres {integration_postgres: self.integration_postgres.clone()}) as Box<dyn Tool + Send>
    }

    fn integr_settings_to_json(&self) -> Result<Value, String> {
        serde_json::to_value(&self.integration_postgres).map_err(|e| e.to_string())
    }

    fn integr_to_schema(&self) -> Value {
        json_schema::<IntegrationPostgres>().unwrap()
    }

    fn integr_settings_default(&self) -> String { DEFAULT_POSTGRES_INTEGRATION_YAML.to_string() }
    fn icon_link(&self) -> String { "https://cdn-icons-png.flaticon.com/512/5968/5968342.png".to_string() }
}

impl ToolPostgres {

    async fn run_psql_command(&self, query: &str) -> Result<String, String> {
        let psql_command = self.integration_postgres.psql_binary_path.as_deref().unwrap_or("psql");
        let output_future = Command::new(psql_command)
            .arg(&self.integration_postgres.connection_string)
            .arg("ON_ERROR_STOP=1")
            .arg("-c")
            .arg(query)
            .output();
        if let Ok(output) = tokio::time::timeout(tokio::time::Duration::from_millis(10_000), output_future).await {
            if output.is_err() {
                let err_text = format!("{}", output.unwrap_err());
                tracing::error!("psql didn't work:\n{}\n{}\n{}", self.integration_postgres.connection_string, query, err_text);
                return Err(format!("psql failed:\n{}", err_text));
            }
            let output = output.unwrap();
            if output.status.success() {
                Ok(String::from_utf8_lossy(&output.stdout).to_string())
            } else {
                // XXX: limit stderr, can be infinite
                let stderr_string = String::from_utf8_lossy(&output.stderr);
                tracing::error!("psql didn't work:\n{}\n{}\n{}", self.integration_postgres.connection_string, query, stderr_string);
                Err(format!("psql failed:\n{}", stderr_string))
            }
        } else {
            tracing::error!("psql timed out:\n{}\n{}", self.integration_postgres.connection_string, query);
            Err("psql command timed out".to_string())
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
        let query = match args.get("query") {
            Some(Value::String(v)) => v.clone(),
            Some(v) => return Err(format!("argument `query` is not a string: {:?}", v)),
            None => return Err("no `query` argument found".to_string()),
        };

        let result = self.run_psql_command(&query).await?;

        let mut results = vec![];
        results.push(ContextEnum::ChatMessage(ChatMessage {
            role: "tool".to_string(),
            content: ChatContent::SimpleText(serde_json::to_string(&result).unwrap()),
            tool_calls: None,
            tool_call_id: tool_call_id.clone(),
            ..Default::default()
        }));
        Ok((true, results))
    }

    fn command_to_match_against_confirm_deny(
        &self,
        args: &HashMap<String, Value>,
    ) -> Result<String, String> {
        let query = match args.get("query") {
            Some(Value::String(v)) => v.clone(),
            Some(v) => return Err(format!("argument `query` is not a string: {:?}", v)),
            None => return Err("no `query` argument found".to_string()),
        };
        Ok(format!("psql {}", query))
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

const DEFAULT_POSTGRES_INTEGRATION_YAML: &str = r#"
postgres:
  psql_binary_path: "/path/to/psql"
  host: "my_postgres_for_django"
  user: "vasya1337"
  password: "$POSTGRES_PASSWORD"
  db: "mydjango"
  available:
    on_your_laptop:
      - project_pattern: "*web_workspace/project1"
        db: "mydjango2"
        enable: true
    when_isolated:
      user: "vasya1338"
      enable: true
  docker:
    my_postgres_for_django:
      image: "postgres:13"
      environment:
        POSTGRES_DB: "mydjango"
        POSTGRES_USER: "vasya1337"
        POSTGRES_PASSWORD: "$POSTGRES_PASSWORD"
"#;


const POSTGRES_INTEGRATION_SCHEMA: &str = r#"
postgres:
  fields:
    host:
      type: string
      desc: "Connect to this host, for example 127.0.0.1 or docker container name."
      placeholder: marketing_db_container
    port:
      type: int
      desc: "Which port to use."
      default: 5432
    user:
      type: string
      placeholder: john_doe
    password:
      type: string
      default: "$POSTGRES_PASSWORD"
    db:
      type: string
      placeholder: marketing_db
  smartlinks:
    - label: "Test"
      chat:
        - role: "user"
          content: |
            Connect to the postgres database, list and briefly describe the tables available.
            If it doesn't work, try to interpret why.
  available:
    on_your_laptop:
      possible: true
    when_isolated:
      possible: true
  docker:
    add_new:
      image: "postgres:13"
      environment:
        POSTGRES_DB: marketing_db
        POSTGRES_USER: "john_doe"
        POSTGRES_PASSWORD: "$POSTGRES_PASSWORD"
    smartlinks:
      - label: "✨ Wizard"
        chat:
          - role: "user"
            content: |
              Connect to the postgres database, list and briefly describe the tables available.
              If it doesn't work, try to interpret why.
"#;
