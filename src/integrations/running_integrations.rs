use std::path::PathBuf;
use std::sync::Arc;
use indexmap::IndexMap;
use tokio::sync::RwLock as ARwLock;
use tokio::sync::Mutex as AMutex;

use crate::tools::tools_description::Tool;
use crate::global_context::GlobalContext;
use crate::integrations::setting_up_integrations::{IntegrationRecord, YamlError};
use crate::integrations::yaml_schema::DockerService;


pub async fn load_integration_tools(
    gcx: Arc<ARwLock<GlobalContext>>,
    current_project: String,
    _allow_experimental: bool,
) -> IndexMap<String, Arc<AMutex<Box<dyn Tool + Send>>>> {
    let (records, error_log) = load_integrations_records(gcx, current_project).await;
        
    let mut tools = IndexMap::new();
    for rec in records {
        if !rec.on_your_laptop {
            continue;
        }
        if !rec.integr_config_exists {
            continue;
        }
        let mut integr = match crate::integrations::integration_from_name(&rec.integr_name) {
            Ok(x) => x,
            Err(e) => {
                tracing::error!("don't have integration {}: {}", rec.integr_name, e);
                continue;
            }
        };
        integr.integr_settings_apply(&rec.config_unparsed);
        tools.insert(rec.integr_name.clone(), Arc::new(AMutex::new(integr.integr_upgrade_to_tool())));
    }

    for e in error_log {
        tracing::error!(
            "{}:{} {:?}",
            crate::nicer_logs::last_n_chars(&&e.integr_config_path, 30),
            e.error_line,
            e.error_msg,
        );
    }

    tools
}

async fn load_integrations_records(
    gcx: Arc<ARwLock<GlobalContext>>,
    _current_project: String,
) -> (Vec<IntegrationRecord>, Vec<YamlError>) {
    // XXX filter _workspace_folders_arc that fit _current_project
    let (config_dirs, global_config_dir) = crate::integrations::setting_up_integrations::get_config_dirs(gcx.clone()).await;
    let integrations_yaml_path = crate::integrations::setting_up_integrations::get_integrations_yaml_path(gcx.clone()).await;

    let mut error_log: Vec<crate::integrations::setting_up_integrations::YamlError> = Vec::new();
    let lst: Vec<&str> = crate::integrations::integrations_list();
    let vars_for_replacements = crate::integrations::setting_up_integrations::get_vars_for_replacements(gcx.clone()).await;
    let records = crate::integrations::setting_up_integrations::read_integrations_d(&config_dirs, &global_config_dir, &integrations_yaml_path, &vars_for_replacements, &lst, &mut error_log);
    (records, error_log)
}

pub async fn load_integration_docker_services(
    gcx: Arc<ARwLock<GlobalContext>>,
    current_project: String,
) -> Vec<(String, DockerService)> {
    let (records, error_log) = load_integrations_records(gcx, current_project).await;
    for e in error_log {
        tracing::error!(
            "{}:{} {:?}",
            crate::nicer_logs::last_n_chars(&&e.integr_config_path, 30),
            e.error_line,
            e.error_msg,
        );
    }

    let mut services = Vec::new();
    for rec in records {
        if !rec.when_isolated { continue; }
        if !rec.integr_config_exists { continue; }

        if let Err(e) = crate::integrations::integration_from_name(&rec.integr_name) {
            tracing::error!("Failed to load integration {}: {}", rec.integr_name, e);
            continue;
        }

        if let Some(docker_config) = rec.config_unparsed.get("docker")
            .and_then(|docker| docker.get("new_container_default")) 
        {
            match serde_json::from_value::<DockerService>(docker_config.clone()) {
                Ok(service) => services.push((rec.integr_name, service)),
                Err(e) => tracing::error!("Failed to parse DockerService for {}: {}", rec.integr_name, e),
            }
        }
    }

    services
}