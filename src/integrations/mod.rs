use std::path::PathBuf;
use std::sync::Arc;
use indexmap::IndexMap;
use serde_json::json;
use tracing::{info, warn};
use tokio::sync::{Mutex as AMutex, RwLock as ARwLock};

use crate::global_context::GlobalContext;
use crate::integrations::integr::Integration;
use crate::integrations::integr_chrome::IntegrationChrome;
use crate::integrations::integr_github::IntegrationGitHub;
use crate::integrations::integr_gitlab::IntegrationGitLab;
use crate::integrations::integr_pdb::IntegrationPdb;
use crate::integrations::integr_postgres::IntegrationPostgres;
use crate::tools::tools_description::Tool;
use crate::yaml_configs::create_configs::{integrations_enabled_cfg, read_yaml_into_value};

pub mod sessions;
pub mod process_io_utils;
pub mod integr_github;
pub mod integr_gitlab;
pub mod integr_pdb;
pub mod integr_chrome;
pub mod docker;
pub mod sessions;
pub mod process_io_utils;
pub mod integr_postgres;
mod integr;


// hint: when adding integration, update:
// DEFAULT_INTEGRATION_VALUES, INTEGRATION_ICONS, integrations_paths, validate_integration_value, load_integration_tools, load_integration_schema_and_json


pub const DEFAULT_INTEGRATION_VALUES: &[(&str, &str)] = &[
    ("github.yaml", integr_github::DEFAULT_GITHUB_INTEGRATION_YAML),
    ("gitlab.yaml", integr_gitlab::DEFAULT_GITLAB_INTEGRATION_YAML),
    ("pdb.yaml", integr_pdb::DEFAULT_PDB_INTEGRATION_YAML),
    ("postgres.yaml", integr_postgres::DEFAULT_POSTGRES_INTEGRATION_YAML),
    ("chrome.yaml", integr_chrome::DEFAULT_CHROME_INTEGRATION_YAML),
];

pub const INTEGRATION_ICONS: &[(&str, &str)] = &[
    ("github", "https://cdn-icons-png.flaticon.com/512/25/25231.png"),
    ("gitlab", "https://cdn-icons-png.flaticon.com/512/5968/5968853.png"),
    ("pdb", "https://cdn-icons-png.flaticon.com/512/919/919852.png"),
    ("postgres", "https://cdn-icons-png.flaticon.com/512/5968/5968342.png"),
    ("chrome", "https://cdn-icons-png.flaticon.com/512/732/732205.png"),
];

pub async fn integrations_paths(gcx: Arc<ARwLock<GlobalContext>>) -> IndexMap<String, PathBuf> {
    let cache_dir = gcx.read().await.cache_dir.clone();
    let integrations_d = cache_dir.join("integrations.d");
    let integration_names = ["github", "gitlab", "pdb", "postgres", "chrome"];

    integration_names.iter().map(|&name| {
        (name.to_string(), integrations_d.join(format!("{}.yaml", name)))
    }).collect()
}

pub fn validate_integration_value(name: &str, value: serde_yaml::Value) -> Result<serde_yaml::Value, String> {
    match name {
        "github" => {
            let integration: IntegrationGitHub = serde_yaml::from_value(value).map_err(|e| e.to_string())?;
            serde_yaml::to_value(integration).map_err(|e| e.to_string())
        }
        "gitlab" => {
            let integration: IntegrationGitLab = serde_yaml::from_value(value).map_err(|e| e.to_string())?;
            serde_yaml::to_value(integration).map_err(|e| e.to_string())
        }
        "pdb" => {
            let integration: IntegrationPdb = serde_yaml::from_value(value).map_err(|e| e.to_string())?;
            serde_yaml::to_value(integration).map_err(|e| e.to_string())
        }
        "postgres" => {
            let integration: IntegrationPostgres = serde_yaml::from_value(value).map_err(|e| e.to_string())?;
            serde_yaml::to_value(integration).map_err(|e| e.to_string())
        }
        "chrome" => {
            let integration: IntegrationChrome = serde_yaml::from_value(value).map_err(|e| e.to_string())?;
            serde_yaml::to_value(integration).map_err(|e| e.to_string())
        }
        _ => Err(format!("Unknown integration type: {}", name)),
    }
}

pub async fn load_integration_tools(
    gcx: Arc<ARwLock<GlobalContext>>,
) -> IndexMap<String, Arc<AMutex<Box<dyn Tool + Send>>>> {
    let paths = integrations_paths(gcx.clone()).await;
    let integrations_yaml_value = {
        let cache_dir = gcx.read().await.cache_dir.clone();
        let yaml_path = cache_dir.join("integrations.yaml");
        read_yaml_into_value(&yaml_path).await?
    };
    let cache_dir = gcx.read().await.cache_dir.clone();
    let enabled_path = cache_dir.join("integrations-enabled.yaml");
    let enabled = match integrations_enabled_cfg(&enabled_path).await {
        serde_yaml::Value::Mapping(map) => map.into_iter().filter_map(|(k, v)| {
            if let (serde_yaml::Value::String(key), serde_yaml::Value::Bool(value)) = (k, v) {
                Some((key, value))
            } else {
                None
            }
        }).collect::<std::collections::HashMap<String, bool>>(),
        _ => std::collections::HashMap::new(),
    };

    let mut integrations = IndexMap::new();
    load_tool_from_yaml(paths.get("github"), integr_github::ToolGithub::new_from_yaml, integrations_yaml_value.get("github"), enabled.get("github"), &mut integrations).await?;
    load_tool_from_yaml(paths.get("gitlab"), integr_gitlab::ToolGitlab::new_from_yaml, integrations_yaml_value.get("gitlab"), enabled.get("gitlab"), &mut integrations).await?;
    load_tool_from_yaml(paths.get("pdb"), integr_pdb::ToolPdb::new_from_yaml, integrations_yaml_value.get("pdb"), enabled.get("pdb"), &mut integrations).await?;
    load_tool_from_yaml(paths.get("postgres"), integr_postgres::ToolPostgres::new_from_yaml, integrations_yaml_value.get("postgres"), enabled.get("postgres"), &mut integrations).await?;
    load_tool_from_yaml(paths.get("chrome"), integr_chrome::ToolChrome::new_from_yaml, integrations_yaml_value.get("chrome"), enabled.get("chrome"), &mut integrations).await?;

    Ok(integrations)
}

pub async fn load_integration_schema_and_json(
    gcx: Arc<ARwLock<GlobalContext>>,
) -> Result<IndexMap<String, (serde_json::Value, serde_json::Value)>, String> {
    let paths = integrations_paths(gcx.clone()).await;
    let integrations_yaml_value = {
        let cache_dir = gcx.read().await.cache_dir.clone();
        let yaml_path = cache_dir.join("integrations.yaml");
        read_yaml_into_value(&yaml_path).await?
    };

    let mut integrations = IndexMap::new();
    schema_and_json_from_integration(paths.get("github"), integr_github::ToolGithub::to_schema_json, integr_github::ToolGithub::new_from_yaml, integrations_yaml_value.get("github"), &mut integrations).await?;
    schema_and_json_from_integration(paths.get("gitlab"), integr_gitlab::ToolGitlab::to_schema_json, integr_gitlab::ToolGitlab::new_from_yaml, integrations_yaml_value.get("gitlab"), &mut integrations).await?;
    schema_and_json_from_integration(paths.get("pdb"), integr_pdb::ToolPdb::to_schema_json, integr_pdb::ToolPdb::new_from_yaml, integrations_yaml_value.get("pdb"), &mut integrations).await?;
    schema_and_json_from_integration(paths.get("postgres"), integr_postgres::ToolPostgres::to_schema_json, integr_postgres::ToolPostgres::new_from_yaml, integrations_yaml_value.get("postgres"), &mut integrations).await?;
    schema_and_json_from_integration(paths.get("chrome"), integr_chrome::ToolChrome::to_schema_json, integr_chrome::ToolChrome::new_from_yaml, integrations_yaml_value.get("chrome"), &mut integrations).await?;

    Ok(integrations)
}

async fn schema_and_json_from_integration<T: Integration>(
    yaml_path: Option<&PathBuf>,
    schema_constructor: fn() -> Result<serde_json::Value, String>,
    tool_constructor: fn(&serde_yaml::Value) -> Result<T, String>,
    value_from_integrations: Option<&serde_yaml::Value>,
    data: &mut IndexMap<String, (serde_json::Value, serde_json::Value)>,
) -> Result<(), String> {
    let yaml_path = yaml_path.expect("No yaml path provided");
    let tool_name = yaml_path.file_stem().expect("No file name").to_str().expect("Invalid file name").to_string();

    let schema = match schema_constructor() {
        Ok(schema) => schema,
        Err(e) => {
            let e = format!("Problem generating schema for {}: {}", tool_name, e);
            warn!("{e}");
            json!({"detail": e.to_string()})
        }
    };

    let value = if yaml_path.exists() {
        match read_yaml_into_value(yaml_path).await {
            Ok(yaml_value) => match tool_constructor(&yaml_value) {
                Ok(integr) => match integr.to_json() {
                    Ok(json_value) => json_value,
                    Err(e) => {
                        let e = format!("Problem converting integration to JSON for {}: {}", tool_name, e);
                        warn!("{e}");
                        json!({"detail": e.to_string()})
                    }
                },
                Err(e) => {
                    let e = format!("Problem constructing tool from {}: {}", yaml_path.display(), e);
                    warn!("{e}");
                    json!({"detail": e.to_string()})
                }
            },
            Err(e) => {
                let e = format!("Problem reading YAML from {}: {}", yaml_path.display(), e);
                warn!("{e}");
                json!({"detail": e.to_string()})
            }
        }
    } else {
        json!({"detail": format!("Cannot read {}. Probably, file does not exist", yaml_path.display())})
    };

    let value_from_integrations = value_from_integrations.map_or(json!({"detail": format!("tool {tool_name} is not defined in integrations.yaml")}), |value| {
        match tool_constructor(&value) {
            Ok(tool) => match tool.to_json() {
                Ok(json_value) => json_value,
                Err(e) => {
                    let e = format!("Problem converting integration to JSON for {}: {}", tool_name, e);
                    warn!("{e}");
                    json!({"detail": e.to_string()})
                }
            },
            Err(e) => {
                warn!("Problem in {}: {}", yaml_path.display(), e);
                json!({"detail": e.to_string()})
            }
        }
    });

    match (value.get("detail"), value_from_integrations.get("detail")) {
        (None, None) => {
            return Err(format!("Tool {tool_name} exists in both {tool_name}.yaml and integrations.yaml. Consider removing one of them."));
        },
        (Some(_), None) => {
            data.insert(tool_name, (schema, value_from_integrations));
        },
        (None, Some(_)) => {
            data.insert(tool_name, (schema, value));
        }
        (Some(_), Some(_)) => {
            data.insert(tool_name, (schema, value));
        }
    }

    Ok(())
}

async fn load_tool_from_yaml<T: Tool + Integration + Send + 'static>(
    yaml_path: Option<&PathBuf>,
    tool_constructor: fn(&serde_yaml::Value) -> Result<T, String>,
    value_from_integrations: Option<&serde_yaml::Value>,
    enabled: Option<&bool>,
    integrations: &mut IndexMap<String, Arc<AMutex<Box<dyn Tool + Send>>>>,
) -> Result<(), String> {
    let yaml_path = yaml_path.as_ref().expect("No yaml path");
    let tool_name = yaml_path.file_stem().expect("No file name").to_str().expect("No file name").to_string();
    if !enabled.unwrap_or(&false) {
        info!("Integration {} is disabled", tool_name);
        return Ok(());
    }
    let tool = if yaml_path.exists() {
        match read_yaml_into_value(yaml_path).await {
            Ok(value) => {
                match tool_constructor(&value) {
                    Ok(tool) => {
                        // integrations.insert(tool_name, Arc::new(AMutex::new(Box::new(tool) as Box<dyn Tool + Send>)));
                        Some(tool)
                    }
                    Err(e) => {
                        warn!("Problem in {}: {}", yaml_path.display(), e);
                        None
                    }
                }
            }
            Err(e) => {
                warn!("Problem reading {:?}: {}", yaml_path, e);
                None
            }
        }
    } else {
        None
    };

    let tool_from_integrations = value_from_integrations
        .and_then(|value| match tool_constructor(&value) {
            Ok(tool) => Some(tool),
            Err(_) => None
        });

    match (tool, tool_from_integrations) {
        (Some(_), Some(_)) => {
            return Err(format!("Tool {tool_name} exists in both {tool_name}.yaml and integrations.yaml. Consider removing one of them."));
        },
        (Some(tool), None) | (None, Some(tool)) => {
            integrations.insert(tool_name.clone(), Arc::new(AMutex::new(Box::new(tool) as Box<dyn Tool + Send>)));
        },
        _ => {}
    }

    Ok(())
}

pub const INTEGRATIONS_DEFAULT_YAML: &str = r#"# This file is used to configure integrations in Refact Agent.
# If there is a syntax error in this file, no integrations will work.
#
# Here you can set up which commands require confirmation or must be denied. If both apply, the command is denied.
# Rules use glob patterns for wildcard matching (https://en.wikipedia.org/wiki/Glob_(programming))
#

commands_need_confirmation:
  - "gh * delete*"
  - "glab * delete*"
  - "psql*[!SELECT]*"
commands_deny:
  - "docker* rm *"
  - "docker* remove *"
  - "docker* rmi *"
  - "docker* pause *"
  - "docker* stop *"
  - "docker* kill *"
  - "gh auth token*"
  - "glab auth token*"


# Command line: things you can call and immediately get an answer
#cmdline:
#  run_make:
#    command: "make"
#    command_workdir: "%project_path%"
#    timeout: 600
#    description: "Run `make` inside a C/C++ project, or a similar project with a Makefile."
#    parameters:    # this is what the model needs to produce, you can use %parameter% in command and workdir
#      - name: "project_path"
#        description: "absolute path to the project"
#    output_filter:                   # output filter is optional, can help if the output is very long to reduce it, preserving valuable information
#      limit_lines: 50
#      limit_chars: 10000
#      valuable_top_or_bottom: "top"  # the useful infomation more likely to be at the top or bottom? (default "top")
#      grep: "(?i)error|warning"      # in contrast to regular grep this doesn't remove other lines from output, just prefers matching when approaching limit_lines or limit_chars (default "(?i)error")
#      grep_context_lines: 5          # leave that many lines around a grep match (default 5)
#      remove_from_output: "process didn't exit"    # some lines and very long and unwanted, this is also a regular expression (default "")

#cmdline_services:
#  manage_py_runserver:
#    command: "python manage.py runserver"
#    command_workdir: "%project_path%"
#    description: "Start or stop `python manage.py runserver` running in the background"
#    parameters:
#      - name: "project_path"
#        description: "absolute path to the project"
#    startup_wait: 10
#    startup_wait_port: 8000


# --- Docker integration ---
docker:
  connect_to_daemon_at: "unix:///var/run/docker.sock"  # Path to the Docker daemon. For remote Docker, the path to the daemon on the remote server.
  # docker_cli_path: "/usr/local/bin/docker"  # Uncomment to set a custom path for the docker cli, defaults to "docker"

  # Uncomment the following to connect to a remote Docker daemon
  # Docker and necessary ports will be forwarded for container communication. No additional commands will be executed over SSH.
  # ssh_config:
  #   host: "<your_server_domain_or_ip_here>"
  #   user: "root"
  #   port: 22
  #   identity_file: "~/.ssh/id_rsa"

  run_chat_threads_inside_container: false

  # The folder inside the container where the workspace is mounted, refact-lsp will start there, defaults to "/app"
  # container_workspace_folder: "/app"

  # Image ID for running containers, which can later be selected in the UI before starting a chat thread.
  # docker_image_id: "079b939b3ea1"

  # Map container ports to local ports
  # ports:
  #   - local_port: 4000
  #     container_port: 3000

  # Path to the LSP binary on the host machine, to be bound into the containers.
  host_lsp_path: "/opt/refact/bin/refact-lsp"

  # Will be added as a label to containers, images, and other resources created by Refact Agent, defaults to "refact"
  label: "refact"

  # Uncomment to execute a command inside the container when the thread starts. Regardless, refact-lsp will run independently of this setting.
  # command: "npm run dev"

  # The time in minutes that the containers will be kept alive while not interacting with the chat thread, defaults to 60.
  keep_containers_alive_for_x_minutes: 60
"#;
