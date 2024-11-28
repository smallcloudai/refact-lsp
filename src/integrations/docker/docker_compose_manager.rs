use std::path::PathBuf;
use std::sync::Arc;
use serde_yaml::Value;
use tokio::sync::RwLock as ARwLock;
use tokio::time::Duration;
use tokio::io::AsyncWriteExt;

use crate::files_correction::get_project_dirs;
use crate::global_context::GlobalContext;
use crate::integrations::running_integrations::load_integration_docker_services;
use crate::integrations::sessions::IntegrationSession;
use crate::integrations::docker::integr_docker::ToolDocker;
use crate::integrations::yaml_schema::DockerService;
use crate::integrations::docker::docker_container_manager::{
    DockerContainerConnectionEnum, 
    docker_container_get_lsp_command, 
    DEFAULT_CONTAINER_LSP_PATH, 
    TARGET_LSP_PORT,
    Port,
};


const DOCKER_COMPOSE_VERSION: &str = "3.8";

pub struct DockerComposeSession {
    lsp_connection: DockerContainerConnectionEnum,
    session_timeout_after_inactivity: Duration,
    last_usage_ts: u64,
}

impl IntegrationSession for DockerComposeSession {
    fn as_any_mut(&mut self) -> &mut dyn std::any::Any { self }

    fn is_expired(&self) -> bool { false }
}

pub async fn docker_compose_start(
    docker: &ToolDocker,
    chat_id: &str,
    gcx: Arc<ARwLock<GlobalContext>>,
) -> Result<PathBuf, String> {
    let current_project = get_project_dirs(gcx.clone()).await.into_iter().next()
        .ok_or_else(|| "No workspace folders found".to_string())?.to_string_lossy().to_string();
    let integration_docker_services = load_integration_docker_services(gcx.clone(), current_project).await;

    let compose_file_path = create_compose_file(
        docker, chat_id, &integration_docker_services, gcx.clone()).await?;

    Ok(compose_file_path)
}

async fn create_compose_file(
    docker: &ToolDocker,
    chat_id: &str,
    extra_services: &Vec<(String, DockerService)>,
    gcx: Arc<ARwLock<GlobalContext>>,
) -> Result<PathBuf, String> {
    let mut compose_yaml_content = serde_yaml::Mapping::new();
    compose_yaml_content.insert(Value::String("version".to_string()), Value::String(DOCKER_COMPOSE_VERSION.to_string()));

    let mut services_definition = serde_yaml::Mapping::new();

    let lsp_command = docker_container_get_lsp_command(gcx.clone()).await?;
    services_definition.insert(Value::String("lsp".to_string()), 
        serde_yaml::Value::Mapping(get_lsp_service_definition(docker, chat_id, &lsp_command)));

    for service in extra_services {
        services_definition.insert(Value::String(service.0.clone()), 
            serde_yaml::Value::Mapping(get_service_definition(&service.1)));
    }

    compose_yaml_content.insert(Value::String("services".to_string()), Value::Mapping(services_definition));
    let yaml_string = serde_yaml::to_string(&compose_yaml_content).map_err(|e| format!("Failed to serialize YAML: {:?}", e))?;
    
    let temp_file = tempfile::Builder::new().suffix(".yaml").tempfile()
        .map_err(|e| format!("Failed creating tempfile: {:?}", e))?;
    let temp_path = temp_file.path().to_path_buf();
    let mut file = tokio::fs::File::create(temp_file.path()).await.map_err(|e| format!("Failed to create file: {:?}", e))?;
    file.write_all(yaml_string.as_bytes()).await.map_err(|e| format!("Failed to write to file: {:?}", e))?;

    temp_file.keep().map_err(|e| format!("Failed to keep tempfile: {:?}", e))?;
    Ok(temp_path)
}

fn get_service_definition(service: &DockerService) -> serde_yaml::Mapping {
    let mut service_definition = serde_yaml::Mapping::new();
    service_definition.insert(Value::String("image".to_string()), Value::String(service.image.clone()));
    
    let mut env_vars = serde_yaml::Mapping::new();
    for (key, value) in &service.environment {
        env_vars.insert(Value::String(key.clone()), Value::String(value.clone()));
    }
    service_definition.insert(Value::String("environment".to_string()), Value::Mapping(env_vars));

    service_definition
}

fn get_lsp_service_definition(docker: &ToolDocker, chat_id: &str, lsp_command: &Vec<String>) -> serde_yaml::Mapping {
    let (docker_image_id, host_lsp_path, label, ssh_config_maybe) = {
        let settings = &docker.settings_docker;
        (settings.docker_image_id.clone(), settings.host_lsp_path.clone(), settings.label.clone(), settings.ssh_config.clone())
    };
    let mut ports_to_forward = if ssh_config_maybe.is_some() {
        docker.settings_docker.ports.iter()
            .map(|p| Port {published: "0".to_string(), target: p.target.clone()}).collect::<Vec<_>>()
    } else {
        docker.settings_docker.ports.clone()
    };
    ports_to_forward.insert(0, Port {published: "0".to_string(), target: TARGET_LSP_PORT.to_string()});

    let mut service_definition = serde_yaml::Mapping::new();
    service_definition.insert(Value::String("image".to_string()), Value::String(docker_image_id));
    service_definition.insert(Value::String("volumes".to_string()), Value::Sequence(vec![
        Value::String(format!("{host_lsp_path}:{DEFAULT_CONTAINER_LSP_PATH}"))
    ]));
    service_definition.insert(Value::String("labels".to_string()), Value::Sequence(vec![
        Value::String(label)
    ]));
    service_definition.insert(Value::String("entrypoint".to_string()), Value::Sequence(vec![
        Value::String("/bin/sh".to_string()),
        Value::String("-c".to_string())
    ]));
    service_definition.insert(Value::String("ports".to_string()), Value::Sequence(
        ports_to_forward.iter().map(|p| {
            Value::String(format!("{}:{}", p.published, p.target))
        }).collect::<Vec<_>>()
    ));
    service_definition.insert(Value::String("command".to_string()), Value::Sequence(
        lsp_command.iter().map(|s| Value::String(s.clone())).collect::<Vec<_>>()
    ));

    service_definition
}