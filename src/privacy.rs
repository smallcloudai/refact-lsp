use std::sync::Arc;
use std::path::Path;
use serde::{Deserialize, Serialize};
use tokio::sync::RwLock as ARwLock;
use tokio::fs;
use tokio::time::Duration;
use tracing::{error, info};
use glob::Pattern;

use crate::global_context::GlobalContext;

#[derive(PartialEq)]
pub enum FilePrivacyLevel {
    AllowToSendEverywhere,
    OnlySendToServersIControl,
    Blocked,
}

const PRIVACY_RELOAD_EACH_N_SECONDS: Duration = Duration::from_secs(1);

#[derive(Debug, Serialize, Deserialize, Default)]
pub struct PrivacySettings {
    pub file_privacy: FilePrivacySettings,
}

#[derive(Debug, Serialize, Deserialize, Clone, Default)]
pub struct FilePrivacySettings {
    #[serde(default)]
    pub only_send_to_servers_I_control: Vec<String>,
    #[serde(default)]
    pub blocked: Vec<String>,
}

pub async fn load_privacy(global_context: Arc<ARwLock<GlobalContext>>) -> PrivacySettings {
    let cache_dir = global_context.read().await.cache_dir.clone();
    let path = cache_dir.join("privacy.yaml");

    match fs::read_to_string(&path).await {
        Ok(content) => {
            match serde_yaml::from_str(&content) {
                Ok(privacy_settings) => {
                    // log info that privacy settings were loaded correctly
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
            error!("cannot read {}: {}, no privacy settings will be used", path.display(), e);
            PrivacySettings::default()
        }
    }
}

pub async fn privacy_background_reload(global_context: Arc<ARwLock<GlobalContext>>) {
    loop {
        let privacy_settings = load_privacy(global_context.clone()).await;
        global_context.write().await.privacy_settings = Arc::new(ARwLock::new(privacy_settings));
        tokio::time::sleep(PRIVACY_RELOAD_EACH_N_SECONDS).await;
    }
}

fn any_glob_matches_path(globs: &Vec<String>, path: &Path) -> bool {
    globs.iter().any(|glob| {
        let pattern = Pattern::new(glob).unwrap();
        pattern.matches_path(path)
    })
}

pub async fn get_file_privacy_level(global_context: Arc<ARwLock<GlobalContext>>, path: &Path) -> FilePrivacyLevel {
    let global_context_lock = global_context.read().await;
    let privacy_settings = global_context_lock.privacy_settings.read().await;

    if any_glob_matches_path(&privacy_settings.file_privacy.blocked, path) {
        FilePrivacyLevel::Blocked
    } else if any_glob_matches_path(&privacy_settings.file_privacy.only_send_to_servers_I_control, path) {
        FilePrivacyLevel::OnlySendToServersIControl
    } else {
        FilePrivacyLevel::AllowToSendEverywhere
    }
}


