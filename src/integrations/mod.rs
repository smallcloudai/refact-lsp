use std::path::PathBuf;
use std::sync::Arc;
use indexmap::IndexMap;
use tracing::warn;
use tokio::sync::{Mutex as AMutex, RwLock as ARwLock};
use crate::global_context::GlobalContext;
use crate::tools::tools_description::Tool;
use crate::yaml_configs::create_configs::read_yaml_into_value;

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


pub const DEFAULT_INTEGRATION_VALUES: &[(&str, &str)] = &[
    ("github.yaml", integr_github::DEFAULT_GITHUB_INTEGRATION_YAML),
    ("gitlab.yaml", integr_gitlab::DEFAULT_GITLAB_INTEGRATION_YAML),
    ("pdb.yaml", integr_pdb::DEFAULT_PDB_INTEGRATION_YAML),
    ("postgres.yaml", integr_postgres::DEFAULT_POSTGRES_INTEGRATION_YAML),
    ("chrome.yaml", integr_chrome::DEFAULT_CHROME_INTEGRATION_YAML),
];

pub async fn load_integration_tools(
    gcx: Arc<ARwLock<GlobalContext>>,
) -> IndexMap<String, Arc<AMutex<Box<dyn Tool + Send>>>> {
    let cache_dir = gcx.read().await.cache_dir.clone();
    let integrations_d= cache_dir.join("integrations.d");

    let github_yaml = integrations_d.join("github.yaml");
    let gitlab_yaml = integrations_d.join("gitlab.yaml");
    let pdb_yaml = integrations_d.join("pdb.yaml");
    let postgres_yaml = integrations_d.join("postgres.yaml");
    let chrome_yaml = integrations_d.join("chrome.yaml");

    let mut integrations = IndexMap::new();
    load_tool_from_yaml(&github_yaml, integr_github::ToolGithub::new_from_yaml, &mut integrations).await;
    load_tool_from_yaml(&gitlab_yaml, integr_gitlab::ToolGitlab::new_from_yaml, &mut integrations).await;
    load_tool_from_yaml(&pdb_yaml, integr_pdb::ToolPdb::new_from_yaml, &mut integrations).await;
    load_tool_from_yaml(&postgres_yaml, integr_postgres::ToolPostgres::new_from_yaml, &mut integrations).await;
    load_tool_from_yaml(&chrome_yaml, integr_chrome::ToolChrome::new_from_yaml, &mut integrations).await;

    integrations
}

async fn load_tool_from_yaml<T: Tool + Send + 'static>(
    yaml_path: &PathBuf,
    tool_constructor: fn(&serde_yaml::Value) -> Result<T, String>,
    integrations: &mut IndexMap<String, Arc<AMutex<Box<dyn Tool + Send>>>>,
) {
    let tool_name = yaml_path.file_stem().expect("No file name").to_str().expect("No file name").to_string();
    if yaml_path.exists() {
        match read_yaml_into_value(yaml_path).await {
            Ok(value) => {
                match tool_constructor(&value) {
                    Ok(tool) => {
                        integrations.insert(tool_name, Arc::new(AMutex::new(Box::new(tool) as Box<dyn Tool + Send>)));
                    }
                    Err(e) => {
                        warn!("Problem in {}: {}", yaml_path.display(), e);
                    }
                }
            }
            Err(e) => {
                warn!("Problem reading {:?}: {}", yaml_path, e);
            }
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
