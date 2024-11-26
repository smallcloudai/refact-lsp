use std::path::PathBuf;
use std::sync::Arc;
use indexmap::IndexMap;
use tokio::sync::{Mutex as AMutex, RwLock as ARwLock};
use tracing::{info, warn};

pub mod integr_abstract;
pub mod integr_github;
pub mod integr_gitlab;
pub mod integr_pdb;
pub mod integr_chrome;
pub mod integr_postgres;

pub mod process_io_utils;
pub mod docker;
pub mod sessions;
pub mod config_chat;
pub mod yaml_schema;
pub mod setting_up_integrations;

use crate::global_context::GlobalContext;
use crate::tools::tools_description::Tool;
use integr_abstract::IntegrationTrait;
use crate::integrations::setting_up_integrations::{integration_extra_from_yaml, IntegrationError, IntegrationErrorYamlParsing, IntegrationExtra};

pub const INTEGRATION_NAMES: &[&str] = &[
    "github",
    "gitlab",
    "pdb",
    "postgres",
    "chrome",
];

pub fn integration_from_name(name: &str) -> Result<Box<dyn IntegrationTrait + Send + Sync>, String> {
    match name {
        "github" => Ok(Box::new(integr_github::ToolGithub { ..Default::default() }) as Box<dyn IntegrationTrait + Send + Sync>),
        "gitlab" => Ok(Box::new(integr_gitlab::ToolGitlab { ..Default::default() }) as Box<dyn IntegrationTrait + Send + Sync>),
        "pdb" => Ok(Box::new(integr_pdb::ToolPdb { ..Default::default() }) as Box<dyn IntegrationTrait + Send + Sync>),
        "postgres" => Ok(Box::new(integr_postgres::ToolPostgres { ..Default::default() }) as Box<dyn IntegrationTrait + Send + Sync>),
        "chrome" => Ok(Box::new(integr_chrome::ToolChrome { ..Default::default() }) as Box<dyn IntegrationTrait + Send + Sync>),
        _ => Err(format!("Unknown integration name: {}", name)),
    }
}

async fn read_yaml_into_value(yaml_path: &PathBuf) -> Result<serde_yaml::Value, IntegrationError> {
    let file = std::fs::File::open(&yaml_path).map_err(
        |e| IntegrationError::IntegrationErrorPlainText(format!("Failed to open {}: {}", yaml_path.display(), e))
    )?;
    let reader = std::io::BufReader::new(file);
    serde_yaml::from_reader(reader).map_err(|e| {
        IntegrationError::IntegrationErrorYamlParsing(IntegrationErrorYamlParsing {
            path: yaml_path.to_string_lossy().to_string(),
            line_n: e.location().map(|loc| loc.line()).unwrap_or(0),
            text: e.to_string(),
        })
    })
}

pub fn get_empty_integrations() -> IndexMap<String, Box<dyn IntegrationTrait + Send + Sync>> {
    let mut integrations = IndexMap::new();
    for i_name in INTEGRATION_NAMES.iter().cloned() {
        let i = integration_from_name(i_name).unwrap();
        integrations.insert(i_name.to_string(), i);
    }
    integrations
}

fn integr_yaml2json(
    value: &serde_yaml::Value, 
    integr_name: &str,
) -> Result<serde_json::Value, IntegrationError> {
    let mut integr_empty = integration_from_name(integr_name).map_err(|e|{
        IntegrationError::IntegrationErrorPlainText(format!("Failed to create empty integration: {}", e))
    })?;
    let j_value = serde_json::to_value(value).map_err(|e| {
        IntegrationError::IntegrationErrorYamlParsing(IntegrationErrorYamlParsing {
            path: "".to_string(),
            line_n: e.line(),
            text: format!("failed to convert yaml -> json: {e}")
        })
    })?;
    integr_empty.integr_settings_apply(&j_value).map_err(|e|{
        IntegrationError::IntegrationErrorPlainText(e)
    })?;
    Ok(integr_empty.integr_settings_as_json())
}

pub fn get_integration_path(cache_dir: &PathBuf, name: &str) -> PathBuf {
    cache_dir.join("integrations.d").join(format!("{}.yaml", name))
}

pub async fn get_integrations(
    gcx: Arc<ARwLock<GlobalContext>>,
) -> Result<
    IndexMap<
        String, 
        IndexMap<String, (Box<dyn IntegrationTrait + Send + Sync>, IntegrationExtra)>>,
    String
> {
    let (cache_dir, workspace_folders) = {
        let gcx_lock = gcx.read().await;
        (gcx_lock.cache_dir.clone(), gcx_lock.documents_state.workspace_folders.clone())
    };
    let workspace_folders = workspace_folders.lock().unwrap().clone();

    let integrations_yaml_path = cache_dir.join("integrations.yaml");
    let integrations_yaml_value = read_yaml_into_value(&integrations_yaml_path).await.map_err(|e|{
        format!("Failed to load integrations.yaml from {:?}: \n{:?}", integrations_yaml_path, e)
    })?;

    let mut results = IndexMap::new();
    results.entry("global".to_string()).or_insert_with(IndexMap::new);
    for (i_name, mut i) in get_empty_integrations() {
        let path = get_integration_path(&cache_dir, &i_name);
        let (j_value, mut i_extra) = json_for_integration_global(
            &path, integrations_yaml_value.get(&i_name), &i, &integrations_yaml_path
        ).await?;

        if !i_extra.error_log.is_empty() {
            
        } else if let Err(e) = i.integr_settings_apply(&j_value) {
            warn!("failed to load integration {}: {}", i_name, e);
            let e = IntegrationError::IntegrationErrorPlainText(e);
            i_extra.error_log.push(e);
        } else {
            i_extra.is_loaded = true;
        }
        
        results["global"].insert(i_name.clone(), (i, i_extra));
    }
    
    // gathering integrations from .refact that is present in each workdir
    for c_dir in workspace_folders {
        info!("Loading integrations from {}", c_dir.display());
        let c_dir_str = c_dir.to_string_lossy().to_string();
        results.entry(c_dir_str.clone()).or_insert_with(IndexMap::new);
        
        for (i_name, mut i) in get_empty_integrations() {
            let integr_path = c_dir.join(".refact").join("integrations.d").join(format!("{}.yaml", i_name));
            let (j_value, mut i_extra) = match json_for_integration_local(&integr_path, &i).await {
                Ok((v, i_extra)) => match v {
                    Some(v) => (v, i_extra),
                    None => continue
                },
                Err(e) => {
                    let mut extra = IntegrationExtra::default();
                    extra.error_log.push(e);
                    (serde_json::Value::Null, extra)
                }
            };

            if !i_extra.error_log.is_empty() {
                
            } else if let Err(e) = i.integr_settings_apply(&j_value) {
                let e = IntegrationError::IntegrationErrorPlainText(e);
                i_extra.error_log.push(e);
            } else {
                i_extra.is_loaded = true;
            }
            
            results[c_dir_str.as_str()].insert(i_name.clone(), (i, i_extra));
        }
    }

    Ok(results)
}

pub async fn load_integration_tools(
    gcx: Arc<ARwLock<GlobalContext>>,
    integr_scope: Option<String>,
) -> Result<IndexMap<String, Arc<AMutex<Box<dyn Tool + Send>>>>, String> {
    let enabled: IndexMap<String, bool> = IndexMap::new();

    let scope = integr_scope.unwrap_or("global".to_string());
    let integrations_dict= get_integrations(gcx.clone()).await?;
    
    let integrations = match integrations_dict.get(scope.as_str()) {
        Some(integrations) => integrations,
        None => {
            return Err(format!("No integrations found in scope: {}", scope));
        }
    };
    
    let mut tools = IndexMap::new();
    for (i_name, (i, i_extra)) in integrations.iter() {
        if !enabled.get(i_name).unwrap_or(&true) { // todo: placeholder: no enabled config rn
            info!("Integration {} is disabled", i_name);
            continue;
        }
        if !i_extra.is_loaded {
            info!("Integration {} has failed to load: {:#?}", i_name, i_extra.error_log);
            continue;
        }
        let tool = i.integr_upgrade_to_tool();
        tools.insert(i_name.clone(), Arc::new(AMutex::new(tool)));
    }
    Ok(tools)
}

async fn json_for_integration_local(
    yaml_path: &PathBuf,
    integration: &Box<dyn IntegrationTrait + Send + Sync>,
) -> Result<(Option<serde_json::Value>, IntegrationExtra), IntegrationError> {
    if yaml_path.exists() {
        match read_yaml_into_value(yaml_path).await {
            Ok(value) => integr_yaml2json(&value, integration.integr_name())
                .map(|x|Some(x))
                .map(|x| {
                    let mut extra = integration_extra_from_yaml(&value);
                    extra.integr_path = yaml_path.to_string_lossy().to_string();
                    (x, extra) 
                })
                .map_err(|e| {
                    let mut e = e;
                    if let IntegrationError::IntegrationErrorYamlParsing(ref mut e_yaml) = e {
                        e_yaml.path = yaml_path.to_string_lossy().to_string();
                    }
                    e
                }),
            Err(e) => Err(e)
        }
    } else {
        Ok((None, IntegrationExtra::default()))
    }
}

async fn json_for_integration_global(
    yaml_path: &PathBuf,
    value_from_integrations: Option<&serde_yaml::Value>,
    integration: &Box<dyn IntegrationTrait + Send + Sync>,
    integrations_yaml_path: &PathBuf,
) -> Result<(serde_json::Value, IntegrationExtra), String> {
    let tool_name = integration.integr_name().to_string();

    let (value, extra) = if yaml_path.exists() {
        match read_yaml_into_value(yaml_path).await {
            Ok(value) => match integr_yaml2json(&value, integration.integr_name()) {
                Ok(i) => {
                    let mut extra = integration_extra_from_yaml(&value);
                    extra.integr_path = yaml_path.to_string_lossy().to_string();
                    (Ok(i), extra)
                }
                Err(e) => {
                    (Err(e), IntegrationExtra::default())
                }
            },
            Err(e) => {
                let mut e = e;
                if let IntegrationError::IntegrationErrorYamlParsing(ref mut e_yaml) = e {
                    e_yaml.path = yaml_path.to_string_lossy().to_string();
                }
                (Err(e), IntegrationExtra::default())
            }
        }
    } else {
        let e = IntegrationError::IntegrationErrorPlainText(format!("Cannot read {}. Probably, file does not exist", yaml_path.display()));
        (Err(e), IntegrationExtra::default())
    };
    
    let value_from_integrations = value_from_integrations.map_or(
        Err(IntegrationError::IntegrationErrorPlainText(format!("tool {tool_name} is not defined in integrations.yaml"))),
        |value| integr_yaml2json(value, integration.integr_name())
    );
    
    match (value.is_ok(), value_from_integrations.is_ok()) {
        (true, true) => {
            Err(format!("Tool {tool_name} exists in both {tool_name}.yaml and integrations.yaml. Consider removing one of them."))
        },
        (false, true) => {
            let mut extra = IntegrationExtra::default();
            extra.integr_path = integrations_yaml_path.to_string_lossy().to_string();
            Ok((value_from_integrations.unwrap(), extra))
        },
        (true, false) => {
            Ok((value.unwrap(), extra))
        }
        (false, false) => {
            let mut extra = IntegrationExtra::default();
            let e = value.unwrap_err();
            extra.error_log.push(e);
            Ok((serde_json::Value::Null, extra))
        }
    }
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
#      remove_from_output: "process didn't exit"    # some lines are very long and unwanted, this is also a regular expression (default "")

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
