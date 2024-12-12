use crate::at_commands::at_commands::AtCommandsContext;
use crate::call_validation::ContextEnum;
use crate::call_validation::{ChatContent, ChatMessage, ChatUsage};
use crate::integrations::go_to_configuration_message;
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
pub struct SettingsMysql {
    #[serde(default)]
    pub mysql_binary_path: String,
    pub host: String,
    pub port: String,
    pub user: String,
    pub password: String,
    pub database: String,
}

#[derive(Default)]
pub struct ToolMysql {
    pub settings_mysql: SettingsMysql,
}

impl IntegrationTrait for ToolMysql {
    fn integr_settings_apply(&mut self, value: &Value) -> Result<(), String> {
        match serde_json::from_value::<SettingsMysql>(value.clone()) {
            Ok(settings_mysql) => self.settings_mysql = settings_mysql,
            Err(e) => {
                tracing::error!("Failed to apply settings: {}\n{:?}", e, value);
                return Err(e.to_string());
            }
        }
        Ok(())
    }

    fn integr_settings_as_json(&self) -> Value {
        serde_json::to_value(&self.settings_mysql).unwrap()
    }

    fn integr_upgrade_to_tool(&self, _integr_name: &String) -> Box<dyn Tool + Send> {
        Box::new(ToolMysql {
            settings_mysql: self.settings_mysql.clone()
        }) as Box<dyn Tool + Send>
    }

    fn integr_schema(&self) -> &str
    {
      MYSQL_INTEGRATION_SCHEMA
    }

    // fn icon_link(&self) -> String { "https://cdn-icons-png.flaticon.com/512/5968/5968342.png".to_string() }
}

impl ToolMysql {
  async fn run_mysql_command(&self, query: &str) -> Result<String, String> {
      let mut mysql_command = self.settings_mysql.mysql_binary_path.clone();
      if mysql_command.is_empty() {
          mysql_command = "mysql".to_string();
      }
      let output_future = Command::new(mysql_command)
          .arg("-h")
          .arg(&self.settings_mysql.host)
          .arg("-P")
          .arg(&self.settings_mysql.port)
          .arg("-u")
          .arg(&self.settings_mysql.user)
          .arg(format!("-p{}", &self.settings_mysql.password))
          .arg(&self.settings_mysql.database)
          .arg("-e")
          .arg(query)
          .output();
      if let Ok(output) = tokio::time::timeout(tokio::time::Duration::from_millis(10_000), output_future).await {
          if output.is_err() {
              let err_text = format!("{}", output.unwrap_err());
              tracing::error!("mysql didn't work:\n{}\n{}", query, err_text);
              return Err(format!("{}, mysql failed:\n{}", go_to_configuration_message("mysql"), err_text));
          }
          let output = output.unwrap();
          if output.status.success() {
              Ok(String::from_utf8_lossy(&output.stdout).to_string())
          } else {
              // XXX: limit stderr, can be infinite
              let stderr_string = String::from_utf8_lossy(&output.stderr);
              tracing::error!("mysql didn't work:\n{}\n{}", query, stderr_string);
              Err(format!("{}, mysql failed:\n{}", go_to_configuration_message("mysql"), stderr_string))
          }
      } else {
          tracing::error!("mysql timed out:\n{}", query);
          Err("mysql command timed out".to_string())
      }
  }
}

#[async_trait]
impl Tool for ToolMysql {
    fn as_any(&self) -> &dyn std::any::Any { self }

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

        let result = self.run_mysql_command(&query).await?;

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
        Ok(format!("mysql {}", query))
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


pub const MYSQL_INTEGRATION_SCHEMA: &str = r#"
fields:
  host:
    f_type: string_long
    f_desc: "Connect to this host, for example 127.0.0.1 or docker container name."
    f_placeholder: marketing_db_container
  port:
    f_type: string_short
    f_desc: "Which port to use."
    f_default: "5432"
  user:
    f_type: string_short
    f_placeholder: john_doe
  password:
    f_type: string_short
    f_default: "$MYSQL_PASSWORD"
    smartlinks:
      - sl_label: "Open passwords.yaml"
        sl_goto: "EDITOR:passwords.yaml"
  database:
    f_type: string_short
    f_placeholder: marketing_db
  mysql_binary_path:
    f_type: string_long
    f_desc: "If it can't find a path to `mysql` you can provide it here, leave blank if not sure."
    f_placeholder: "mysql"
    f_label: "MYSQL Binary Path"
    f_extra: true
description: |
  The Mysql tool is for the AI model to call, when it wants to look at data inside your database, or make any changes.
  On this page you can also see Docker containers with Mysql servers.
  You can ask model to create a new container with a new database for you,
  or ask model to configure the tool to use an existing container with existing database.
available:
  on_your_laptop_possible: true
  when_isolated_possible: true
smartlinks:
  - sl_label: "Test"
    sl_chat:
      - role: "user"
        content: |
          🔧 The mysql tool should be visible now. To test the tool, list the tables available, briefly describe the tables and express
          happiness, and change nothing. If it doesn't work or the tool isn't available, go through the usual plan in the system prompt.
          The current config file is %CURRENT_CONFIG%.
  - sl_label: "Look at the project, fill in automatically"
    sl_chat:
      - role: "user"
        content: |
          🔧 Your goal is to set up mysql client. Look at the project, especially files like "docker-compose.yaml" or ".env". Call tree() to see what files the project has.
          After that is completed, go through the usual plan in the system prompt.
          The current config file is %CURRENT_CONFIG%.
docker:
  filter_label: ""
  filter_image: "mysql"
  new_container_default:
    image: "mysql:8.4"
    environment:
      MYSQL_DATABASE: db_name
      MYSQL_USER: $MYSQL_USER
      MYSQL_PASSWORD: $MYSQL_PASSWORD
  smartlinks:
    - sl_label: "Add Database Container"
      sl_chat:
        - role: "user"
          content: |
            🔧 Your job is to create a mysql container, using the image and environment from new_container_default section in the current config file: %CURRENT_CONFIG%. Follow the system prompt.
  smartlinks_for_each_container:
    - sl_label: "Use for integration"
      sl_chat:
        - role: "user"
          content: |
            🔧 Your job is to modify mysql connection config in the current file to match the variables from the container, use docker tool to inspect the container if needed. Current config file: %CURRENT_CONFIG%.
"#;
