use std::sync::{Arc, Weak};
use std::path::Path;
use serde::{Deserialize, Deserializer};
use tokio::sync::RwLock as ARwLock;
use tokio::time::Duration;
use tokio::fs;
use tracing::error;
use glob::Pattern;
use std::time::SystemTime;
use serde::de::{self, MapAccess, Visitor};
use std::fmt;

use crate::global_context::GlobalContext;


#[derive(Debug, PartialEq, PartialOrd)]
pub enum FilePrivacyLevel {
    Blocked = 0,
    OnlySendToServersIControl = 1,
    AllowToSendAnywhere = 2,
}

#[derive(Debug, Deserialize)]
pub struct PrivacySettings {
    pub privacy_rules: FilePrivacySettings,
    #[serde(default)]
    pub loaded_ts: u64,
}

impl Default for PrivacySettings {
    fn default() -> Self {
        PrivacySettings {
            privacy_rules: FilePrivacySettings::default(),
            loaded_ts: 0,
        }
    }
}

#[allow(non_snake_case)]
#[derive(Debug)]
pub struct FilePrivacySettings {
    pub only_send_to_servers_I_control: Vec<String>,
    pub blocked: Vec<String>,
    pub blacklisted: Vec<String>,
    pub whitelisted: Vec<String>,
}

const DEFAULT_BLACKLISTED_DIRS: &[&str] = &[
    "target", "node_modules", "vendor", "build", "dist",
    "bin", "pkg", "lib", "lib64", "obj",
    "out", "venv", "env", "tmp", "temp", "logs",
    "coverage", "backup", "__pycache__",
    "_trajectories", ".gradle",
    ".idea", ".git", ".hg", ".svn", ".bzr", ".DS_Store",
];

impl Default for FilePrivacySettings {
    fn default() -> Self {
        FilePrivacySettings {
            blocked: vec!["*".to_string()],
            only_send_to_servers_I_control: vec![],
            blacklisted: DEFAULT_BLACKLISTED_DIRS.iter().map(|s| s.to_string()).collect::<Vec<String>>(),
            whitelisted: vec![],
        }
    }
}

impl<'de> Deserialize<'de> for FilePrivacySettings {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        struct FilePrivacySettingsVisitor;

        impl<'de> Visitor<'de> for FilePrivacySettingsVisitor {
            type Value = FilePrivacySettings;

            fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
                formatter.write_str("struct FilePrivacySettings")
            }

            fn visit_map<V>(self, mut map: V) -> Result<Self::Value, V::Error>
            where
                V: MapAccess<'de>,
            {
                let default_privacy_settings = FilePrivacySettings::default();
                #[allow(non_snake_case)]
                let mut only_send_to_servers_I_control = default_privacy_settings.only_send_to_servers_I_control.clone();
                let mut blocked = default_privacy_settings.blocked.clone();
                let mut blacklisted = default_privacy_settings.blacklisted.clone();
                let mut whitelisted = default_privacy_settings.whitelisted.clone();

                while let Some(key) = map.next_key::<String>()? {
                    match key.as_str() {
                        "only_send_to_servers_I_control" => {
                            only_send_to_servers_I_control = map.next_value()?;
                        }
                        "blocked" => {
                            blocked = map.next_value()?;
                        }
                        "blacklisted" => {
                            blacklisted = map.next_value()?;
                        }
                        "whitelisted" => {
                            whitelisted = map.next_value()?;
                        }
                        _ => {
                            let _: de::IgnoredAny = map.next_value()?;
                        }
                    }
                }

                let r = FilePrivacySettings {
                    only_send_to_servers_I_control,
                    blocked,
                    blacklisted,
                    whitelisted,
                };

                Ok(r)
            }
        }

        deserializer.deserialize_map(FilePrivacySettingsVisitor)
    }
}

const PRIVACY_TOO_OLD: Duration = Duration::from_secs(3);

async fn read_privacy_yaml(path: &Path) -> PrivacySettings
{
    match fs::read_to_string(&path).await {
        Ok(content) => {
            match serde_yaml::from_str(&content) {
                Ok(privacy_settings) => {
                    privacy_settings
                }
                Err(e) => {
                    error!("parsing {} failed\n{}", path.display(), e);
                    return PrivacySettings::default();
                }
            }
        }
        Err(e) => {
            error!("unable to read content from {}\n{}", path.display(), e);
            return PrivacySettings::default();
        }
    }
}

pub async fn load_privacy_if_needed(gcx: Arc<ARwLock<GlobalContext>>) -> Arc<PrivacySettings>
{
    let current_time = SystemTime::now().duration_since(SystemTime::UNIX_EPOCH).unwrap().as_secs();
    let path = {
        let gcx_locked = gcx.read().await;
        let should_reload = gcx_locked.privacy_settings.loaded_ts + PRIVACY_TOO_OLD.as_secs() <= current_time;
        if !should_reload {
            return gcx_locked.privacy_settings.clone();
        }
        gcx_locked.config_dir.join("privacy.yaml")
    };

    let mut new_privacy_settings = read_privacy_yaml(&path).await;
    new_privacy_settings.loaded_ts = current_time;

    {
        let mut gcx_locked = gcx.write().await;
        gcx_locked.privacy_settings = Arc::new(new_privacy_settings);
        gcx_locked.privacy_settings.clone()
    }
}

pub async fn load_privacy_rules_if_needed(gcx: Arc<ARwLock<GlobalContext>>) -> Arc<FilePrivacySettings>
{
    let privacy_settings = load_privacy_if_needed(gcx.clone()).await;
    Arc::new(FilePrivacySettings{
        only_send_to_servers_I_control: privacy_settings.privacy_rules.only_send_to_servers_I_control.clone(),
        blocked: privacy_settings.privacy_rules.blocked.clone(),
        blacklisted: privacy_settings.privacy_rules.blacklisted.clone(),
        whitelisted: privacy_settings.privacy_rules.whitelisted.clone(),
    })
}

pub async fn load_privacy_rules_if_needed_weak(gcx_weak: Weak<ARwLock<GlobalContext>>) -> Arc<FilePrivacySettings>
{
    let mut privacy_rules = Arc::new(FilePrivacySettings::default());
    if let Some(gcx) = gcx_weak.clone().upgrade() {
        privacy_rules = load_privacy_rules_if_needed(gcx).await;
    }
    privacy_rules
}

fn any_glob_matches_path(globs: &Vec<String>, path: &Path) -> bool {
    globs.iter().any(|glob| {
        let pattern = Pattern::new(glob).unwrap();
        let matches = pattern.matches_path(path);
        matches
    })
}
fn get_file_privacy_level(privacy_settings: Arc<PrivacySettings>, path: &Path) -> FilePrivacyLevel
{
    if any_glob_matches_path(&privacy_settings.privacy_rules.blocked, path) {
        FilePrivacyLevel::Blocked
    } else if any_glob_matches_path(&privacy_settings.privacy_rules.only_send_to_servers_I_control, path) {
        FilePrivacyLevel::OnlySendToServersIControl
    } else {
        FilePrivacyLevel::AllowToSendAnywhere
    }
}

pub fn check_file_privacy(privacy_settings: Arc<PrivacySettings>, path: &Path, min_allowed_privacy_level: &FilePrivacyLevel) -> Result<(), String>
{
    let file_privacy_level = get_file_privacy_level(privacy_settings.clone(), path);
    if file_privacy_level < *min_allowed_privacy_level {
        return Err(format!("privacy level {:?}", file_privacy_level));
    }
    Ok(())
}


#[cfg(test)]
mod tests {
    use super::*;
    use std::{path::PathBuf, sync::Arc};

    #[test]
    fn test_privacy_patterns() {
        // Arrange
        let privacy_settings = Arc::new(PrivacySettings {
            privacy_rules: FilePrivacySettings {
                only_send_to_servers_I_control: vec!["*.pem".to_string(), "*/semi_private_dir/*.md".to_string()],
                blocked: vec!["*.pem".to_string(), "*/secret_dir/*".to_string(), "secret_passwords.txt".to_string()],
                blacklisted: vec![],
                whitelisted: vec![],
            },
            loaded_ts: 0,
        });

        let current_dir = std::env::current_dir().unwrap();

        let cases: Vec<(PathBuf, FilePrivacyLevel)> = vec![
            (current_dir.join("secret.pem"), FilePrivacyLevel::Blocked),          // matches both
            (current_dir.join("somedir/secret.pem"), FilePrivacyLevel::Blocked),  // matches both
            (current_dir.join("secret.pub"), FilePrivacyLevel::AllowToSendAnywhere),
            (current_dir.join("secret_passwords.txt"), FilePrivacyLevel::AllowToSendAnywhere),
            (current_dir.join("secret_passwords.jpeg"), FilePrivacyLevel::AllowToSendAnywhere),
            (current_dir.join("secret_dir/anything.jpg"), FilePrivacyLevel::Blocked),
            (current_dir.join("semi_private_dir/wow1.md"), FilePrivacyLevel::OnlySendToServersIControl),
            (current_dir.join("semi_private_dir/wow1.jpeg"), FilePrivacyLevel::AllowToSendAnywhere),
            (current_dir.join("1/2/3/semi_private_dir/wow1.md"), FilePrivacyLevel::OnlySendToServersIControl),
            (current_dir.join("1/2/3/semi_private_dir/4/5/6/wow1.md"), FilePrivacyLevel::OnlySendToServersIControl),
            (current_dir.join("wow1.md"), FilePrivacyLevel::AllowToSendAnywhere),
        ];

        for (path, expected_privacy_level) in cases {
            let actual_privacy_level = get_file_privacy_level(privacy_settings.clone(), &path);
            assert_eq!(
                actual_privacy_level,
                expected_privacy_level,
                "Testing get_file_privacy_level with path {} and expected privacy level {:?}, got {:?}",
                path.display(),
                expected_privacy_level,
                actual_privacy_level,
            );
        }
    }

    #[test]
    fn test_privacy_minimum() {
        let privacy_settings = Arc::new(PrivacySettings {
            privacy_rules: FilePrivacySettings {
                only_send_to_servers_I_control: vec!["*.cat.txt".to_string(), "*.md".to_string(), "*/.venv/*".to_string(), "**/tests_dir/**/*".to_string()],
                blocked: vec!["*/make.png".to_string(), "*.txt".to_string()],
                blacklisted: vec![],
                whitelisted: vec![],
            },
            loaded_ts: 0,
        });

        let current_dir = std::env::current_dir().unwrap();

        let cases: Vec<(PathBuf, FilePrivacyLevel, bool)> = vec![
            (current_dir.join("test.zip"), FilePrivacyLevel::AllowToSendAnywhere, true),
            (current_dir.join("test.md"), FilePrivacyLevel::AllowToSendAnywhere, false),
            (current_dir.join("test.md"), FilePrivacyLevel::OnlySendToServersIControl, true),
            (current_dir.join("test.cat.txt"), FilePrivacyLevel::OnlySendToServersIControl, false),
        ];

        for (path, expected_privacy_level, expected_result) in &cases {
            let result = check_file_privacy(privacy_settings.clone(), path, expected_privacy_level);
            if *expected_result {
                assert!(
                    result.is_ok(),
                    "Testing check_file_privacy with path {} and expected privacy level {:?}, got {:?} and it should have been ok",
                    path.display(),
                    expected_privacy_level,
                    result.unwrap_err(),
                );
            } else {
                assert!(
                    result.is_err(),
                    "Testing check_file_privacy with path {} and expected privacy level {:?}, got {:?} and it should have been err",
                    path.display(),
                    expected_privacy_level,
                    result.unwrap(),
                );
            }
        }
    }

    // TODO: test black/white lists
}








