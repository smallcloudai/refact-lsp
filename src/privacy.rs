use std::sync::Arc;
use std::path::Path;
use serde::{Deserialize, Serialize};
use tokio::sync::RwLock as ARwLock;
use tokio::time::Duration;
use tokio::fs;
use tracing::{error, info};
use glob::Pattern;
use std::time::SystemTime;
use std::io::Write;

use crate::global_context::GlobalContext;
use crate::privacy_compiled_in::COMPILED_IN_INITIAL_PRIVACY_YAML;

#[derive(Debug, PartialEq, PartialOrd)]
pub enum FilePrivacyLevel {
    AllowToSendEverywhere = 0,
    OnlySendToServersIControl = 1,
    Blocked = 2,
}

#[derive(Debug, Serialize, Deserialize, Default)]
pub struct PrivacySettings {
    pub file_privacy: FilePrivacySettings,
    #[serde(default = "default_expiry_time", skip)]
    pub expiry_time: u64,
}

const PRIVACY_RELOAD_EACH_N_SECONDS: Duration = Duration::from_secs(10);
fn default_expiry_time() -> u64 {
    SystemTime::now().duration_since(SystemTime::UNIX_EPOCH).unwrap().as_secs() + PRIVACY_RELOAD_EACH_N_SECONDS.as_secs()
}

#[derive(Debug, Serialize, Deserialize, Clone, Default)]
pub struct FilePrivacySettings {
    #[serde(default)]
    #[allow(non_snake_case)]
    pub only_send_to_servers_I_control: Vec<String>,
    #[serde(default)]
    pub blocked: Vec<String>,
}

// TODO: Move to other yaml files handling once that part is finished
async fn read_privacy_yaml(path: &Path) -> PrivacySettings {
    match fs::read_to_string(&path).await {
        Ok(content) => {
            match serde_yaml::from_str(&content) {
                Ok(privacy_settings) => {
                    info!("privacy settings loaded from {}", path.display());
                    privacy_settings
                }
                Err(e) => {
                    error!("failed to deserialize YAML from {}: {}, no privacy settings will be used", path.display(), e);
                    PrivacySettings::default()
                }
            }
        }
        Err(e) => {
            error!("failed to read content from {}: {}, no privacy settings will be used", path.display(), e);
            PrivacySettings::default()
        }
    }
}

// TODO: Move to other yaml files handling once that part is finished
async fn load_privacy_if_needed(global_context: Arc<ARwLock<GlobalContext>>) {
    let (should_reload, path) = {
        let global_context_lock = global_context.read().await;
        let current_time = SystemTime::now().duration_since(SystemTime::UNIX_EPOCH).unwrap().as_secs();
        let should_reload = global_context_lock.privacy_settings.expiry_time <= current_time;
        let path = global_context_lock.cache_dir.join("privacy.yaml");
        (should_reload, path)
    };

    if !should_reload {
        return;
    }

    if !path.exists() {
        match std::fs::File::create(&path) {
            Ok(mut file) => {
                if let Err(e) = file.write_all(COMPILED_IN_INITIAL_PRIVACY_YAML.as_bytes()) {
                    error!("Failed to write to file: {}", e);
                }
            }
            Err(e) => {
                error!("Failed to create file: {}", e);
            }
        }
    }

    let new_privacy_settings = read_privacy_yaml(&path).await;

    let mut global_context_lock = global_context.write().await;
    global_context_lock.privacy_settings = Arc::new(new_privacy_settings);
}

fn any_glob_matches_path(globs: &Vec<String>, path: &Path) -> bool {
    globs.iter().any(|glob| {
        let pattern = Pattern::new(glob).unwrap();
        pattern.matches_path(path)
    })
}

async fn get_file_privacy_level(global_context: Arc<ARwLock<GlobalContext>>, path: &Path) -> FilePrivacyLevel {
    let global_context_lock = global_context.read().await;
    let privacy_settings = &global_context_lock.privacy_settings;

    if any_glob_matches_path(&privacy_settings.file_privacy.blocked, path) {
        FilePrivacyLevel::Blocked
    } else if any_glob_matches_path(&privacy_settings.file_privacy.only_send_to_servers_I_control, path) {
        FilePrivacyLevel::OnlySendToServersIControl
    } else {
        FilePrivacyLevel::AllowToSendEverywhere
    }
}

pub async fn check_file_privacy(global_context: Arc<ARwLock<GlobalContext>>, path: &Path, max_allowed_privacy_level: FilePrivacyLevel) -> Result<(), String> {
    load_privacy_if_needed(global_context.clone()).await;

    let file_privacy_level = get_file_privacy_level(global_context.clone(), path).await;
    if file_privacy_level > max_allowed_privacy_level {
        Err(format!("File privacy level for file is too restrictive, {} is {:?}", path.display(), file_privacy_level))
    } else {
        Ok(())
    }
}

pub fn check_file_privacy_sync(global_context: Arc<ARwLock<GlobalContext>>, path: &Path, max_allowed_privacy_level: FilePrivacyLevel) -> Result<(), String> {
    let file_privacy_level = futures::executor::block_on(async {
        load_privacy_if_needed(global_context.clone()).await;
        get_file_privacy_level(global_context.clone(), path).await
    });

    if file_privacy_level > max_allowed_privacy_level {
        Err(format!("File privacy level for file is too restrictive, {} is {:?}", path.display(), file_privacy_level))
    } else {
        Ok(())
    }
}


