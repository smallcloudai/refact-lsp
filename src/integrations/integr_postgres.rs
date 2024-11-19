use crate::at_commands::at_commands::AtCommandsContext;
use crate::call_validation::ContextEnum;
use crate::call_validation::{ChatContent, ChatMessage, ChatUsage};
use crate::tools::tools_description::Tool;
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::process::Command;
use tokio::sync::Mutex as AMutex;
use crate::integrations::integr_abstract::IntegrationTrait;


#[derive(Clone, Serialize, Deserialize, Debug, Default)]
pub struct SettingsPostgres {
    pub psql_binary_path: String,
    pub host: String,
    pub port: usize,
    pub user: String,
    pub password: String,
    pub database: String,
}

#[derive(Default)]
pub struct ToolPostgres {
    pub integration_postgres: SettingsPostgres,
}

impl IntegrationTrait for ToolPostgres {
    fn integr_name(&self) -> &str { "postgres" }

    fn integr_schema(&self) -> &str { POSTGRES_INTEGRATION_SCHEMA }

    fn integr_settings_apply(&mut self, value: &Value) -> Result<(), String> {
        match serde_json::from_value::<SettingsPostgres>(value.clone()) {
            Ok(integration_postgres) => self.integration_postgres = integration_postgres,
            Err(e) => {
                tracing::error!("Failed to apply settings: {}\n{:?}", e, value);
                return Err(e.to_string());
            }
        }
        Ok(())
    }

    fn integr_settings_as_json(&self) -> Value {
        serde_json::to_value(&self.integration_postgres).unwrap()
    }

    fn integr_upgrade_to_tool(&self) -> Box<dyn Tool + Send> {
        Box::new(ToolPostgres {integration_postgres: self.integration_postgres.clone()}) as Box<dyn Tool + Send>
    }

    fn integr_yaml2json(&self, value: &serde_yaml::Value) -> Result<Value, String> {
        let settings = serde_yaml::from_value::<SettingsPostgres>(value.clone()).map_err(|e| {
            let location = e.location().map(|loc| format!(" at line {}, column {}", loc.line(), loc.column())).unwrap_or_default();
            format!("{}{}", e.to_string(), location)
        })?;
        serde_json::to_value(&settings).map_err(|e| e.to_string())
    }

    // fn icon_link(&self) -> String { "https://cdn-icons-png.flaticon.com/512/5968/5968342.png".to_string() }
}

impl ToolPostgres {
    async fn run_psql_command(&self, query: &str) -> Result<String, String> {
        let mut psql_command = self.integration_postgres.psql_binary_path.clone();
        if psql_command.is_empty() {
            psql_command = "psql".to_string();
        }
        let output_future = Command::new(psql_command)
            .env("PGPASSWORD", &self.integration_postgres.password)
            .env("PGHOST", &self.integration_postgres.host)
            .env("PGUSER", &self.integration_postgres.user)
            .env("PGPORT", &format!("{}", self.integration_postgres.port))
            .env("PGDATABASE", &self.integration_postgres.database)
            .arg("-v")
            .arg("ON_ERROR_STOP=1")
            .arg("-c")
            .arg(query)
            .output();
        if let Ok(output) = tokio::time::timeout(tokio::time::Duration::from_millis(10_000), output_future).await {
            if output.is_err() {
                let err_text = format!("{}", output.unwrap_err());
                tracing::error!("psql didn't work:\n{}\n{}", query, err_text);
                return Err(format!("psql failed:\n{}", err_text));
            }
            let output = output.unwrap();
            if output.status.success() {
                Ok(String::from_utf8_lossy(&output.stdout).to_string())
            } else {
                // XXX: limit stderr, can be infinite
                let stderr_string = String::from_utf8_lossy(&output.stderr);
                tracing::error!("psql didn't work:\n{}\n{}", query, stderr_string);
                Err(format!("psql failed:\n{}", stderr_string))
            }
        } else {
            tracing::error!("psql timed out:\n{}", query);
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

// const DEFAULT_POSTGRES_INTEGRATION_YAML: &str = r#"
// postgres:
//   enable: true
//   psql_binary_path: "/path/to/psql"
//   host: "my_postgres_for_django"
//   user: "vasya1337"
//   password: "$POSTGRES_PASSWORD"
//   db: "mydjango"
//   available:
//     on_your_laptop:
//       - project_pattern: "*web_workspace/project1"
//         db: "mydjango2"
//         enable: true
//     when_isolated:
//       user: "vasya1338"
//       enable: true
//   docker:
//     my_postgres_for_django:
//       image: "postgres:13"
//       environment:
//         POSTGRES_DB: "mydjango"
//         POSTGRES_USER: "vasya1337"
//         POSTGRES_PASSWORD: "$POSTGRES_PASSWORD"
// "#;


pub const POSTGRES_INTEGRATION_SCHEMA: &str = r#"
fields:
  host:
    f_type: string
    f_desc: "Connect to this host, for example 127.0.0.1 or docker container name."
    f_placeholder: marketing_db_container
  port:
    f_type: int
    f_desc: "Which port to use."
    f_default: "5432"
  user:
    f_type: string
    f_placeholder: john_doe
  password:
    f_type: string
    f_default: "$POSTGRES_PASSWORD"
    smartlinks:
      - sl_label: "Open passwords.yaml"
        sl_goto: "EDITOR:passwords.yaml"
  database:
    f_type: string
    f_placeholder: marketing_db
available:
  on_your_laptop_possible: true
  when_isolated_possible: true
smartlinks:
  - sl_label: "Test"
    sl_chat:
      - role: "user"
        content: |
          🔧 Use postgres database to briefly list the tables available, express satisfaction and relief if it works, and change nothing.
          If it doesn't work, go through the usual plan in the system prompt.
          The current config file is @file %CURRENT_CONFIG%
docker:
  new_container_default:
    image: "postgres:13"
    environment:
      POSTGRES_DB: "marketing_db"
      POSTGRES_USER: "john_doe"
      POSTGRES_PASSWORD: "$POSTGRES_PASSWORD"
  smartlinks:
    - sl_label: "Add Database Container"
      sl_chat:
        - role: "user"
          content: |
            🔧 Your job is to create a new section under "docker" that will define a new postgres container, inside the current config file %CURRENT_CONFIG%. Follow the system prompt.
"#;


// available:
//   on_your_laptop:
//     possible: true
//   when_isolated:
//     possible: true
