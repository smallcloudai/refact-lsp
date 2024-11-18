use std::fs;
use std::path::PathBuf;
use std::sync::Arc;
use serde::Serialize;
use tokio::sync::RwLock as ARwLock;

use crate::global_context::GlobalContext;
// use crate::tools::tools_description::Tool;
// use crate::yaml_configs::create_configs::{integrations_enabled_cfg, read_yaml_into_value};


#[derive(Serialize, Default)]
pub struct YamlError {
    pub integr_config_path: String,
    pub error_line: usize,  // starts with 1, zero if invalid
    pub error_msg: String,
}

#[derive(Serialize, Default)]
pub struct IntegrationWithIconRecord {
    pub project_path: String,
    pub integr_name: String,
    pub integr_config_path: String,
    pub integr_config_exists: bool,
    pub on_your_laptop: bool,
    pub when_isolated: bool,
    // pub unparsed:
}

#[derive(Serialize, Default)]
pub struct IntegrationWithIconResult {
    pub integrations: Vec<IntegrationWithIconRecord>,
    pub error_log: Vec<YamlError>,
}

fn _read_integrations_d(
    config_folders: &Vec<PathBuf>,
    lst: &[&str],
    error_log: &mut Vec<YamlError>,
) -> Vec<IntegrationWithIconRecord> {
    let mut integrations = Vec::new();
    for config_dir in config_folders {
        for integr_name in lst.iter() {
            let path_str = join_config_path(config_dir, integr_name);
            let path = PathBuf::from(path_str.clone());
            let mut rec: IntegrationWithIconRecord = Default::default();
            let (_integr_name, project_path) = match split_config_path(&path) {
                Ok(x) => x,
                Err(e) => {
                    tracing::error!("error deriving project path: {}", e);
                    continue;
                }
            };
            rec.project_path = project_path.clone();
            rec.integr_name = integr_name.to_string();
            rec.integr_config_path = path_str.clone();
            rec.integr_config_exists = path.exists();
            if rec.integr_config_exists {
                match fs::read_to_string(&path) {
                    Ok(file_content) => match serde_yaml::from_str::<serde_yaml::Value>(&file_content) {
                        Ok(yaml_value) => {
                            if let Some(available) = yaml_value.get("available").and_then(|v| v.as_mapping()) {
                                rec.on_your_laptop = available.get("on_your_laptop").and_then(|v| v.as_bool()).unwrap_or(false);
                                rec.when_isolated = available.get("when_isolated").and_then(|v| v.as_bool()).unwrap_or(false);
                            } else {
                                tracing::info!("no 'available' mapping in `{}`", integr_name);
                            }
                        }
                        Err(e) => {
                            let location = e.location().map(|loc| format!(" at line {}, column {}", loc.line(), loc.column())).unwrap_or_default();
                            error_log.push(YamlError {
                                integr_config_path: path_str.to_string(),
                                error_line: e.location().map(|loc| loc.line()).unwrap_or(0),
                                error_msg: e.to_string(),
                            });
                            tracing::warn!("failed to parse {}{}: {}", path_str, location, e.to_string());
                        }
                    },
                    Err(e) => {
                        error_log.push(YamlError {
                            integr_config_path: path_str.to_string(),
                            error_line: 0,
                            error_msg: e.to_string(),
                        });
                        tracing::warn!("failed to read {}: {}", path_str, e.to_string());
                    }
                }
            } else {
                tracing::info!("no config file `{}`", integr_name);
            }
            integrations.push(rec);
        }
    }
    integrations
}

pub fn join_config_path(config_dir: &PathBuf, integr_name: &str) -> String {
    config_dir.join("integrations.d").join(format!("{}.yaml", integr_name)).to_string_lossy().into_owned()
}

pub async fn config_dirs(
    gcx: Arc<ARwLock<GlobalContext>>,
) -> Vec<PathBuf> {
    let (config_dir, workspace_folders_arc) = {
        let gcx_locked = gcx.read().await;
        (gcx_locked.config_dir.clone(), gcx_locked.documents_state.workspace_folders.clone())
    };
    let mut config_folders = workspace_folders_arc.lock().unwrap().clone();
    config_folders = config_folders.iter().map(|folder| folder.join(".refact")).collect();
    config_folders.push(config_dir);
    config_folders
}

pub fn split_config_path(cfg_path: &PathBuf) -> Result<(String, String), String>
{
    let integr_name = cfg_path.file_stem().and_then(|s| s.to_str()).unwrap_or_default().to_string();
    if integr_name.is_empty() {
        return Err(format!("can't derive integration name from file name"));
    }
    let project_path = if let Some(parent) = cfg_path.parent() {
        let parent_str = parent.to_string_lossy();
        if parent_str.contains(".refact/integrations.d") {
            parent_str.split(".refact/integrations.d").next().unwrap().to_string()
        } else if parent_str.contains(".config/refact/integrations.d") {
            String::new()
        } else {
            return Err(format!("invalid path: {}", cfg_path.display()));
        }
    } else {
        return Err(format!("invalid path: {}", cfg_path.display()));
    };
    let extension = cfg_path.extension().and_then(|ext| ext.to_str()).unwrap_or_default();
    if extension != "yaml" {
        return Err(format!("invalid file extension: {}", extension));
    }
    Ok((integr_name, project_path))
}

pub async fn integrations_all_with_icons(
    gcx: Arc<ARwLock<GlobalContext>>,
) -> IntegrationWithIconResult {
    let config_folders = config_dirs(gcx).await;
    let lst: Vec<&str> = crate::integrations::integrations_list();
    let mut error_log: Vec<YamlError> = Vec::new();
    let integrations = _read_integrations_d(&config_folders, &lst, &mut error_log);
    // rec.integr_icon = crate::integrations::icon_from_name(integr_name);
    IntegrationWithIconResult {
        integrations,
        error_log,
    }
}

#[derive(Serialize, Default)]
pub struct IntegrationGetResult {
    pub project_path: String,
    pub integr_name: String,
    pub integr_config_path: String,
    pub integr_schema: serde_json::Value,
    pub integr_values: serde_json::Value,
    pub error_log: Vec<YamlError>,
}

pub async fn integration_config_get(
    integr_config_path: String,
) -> Result<IntegrationGetResult, String> {
    let sanitized_path = PathBuf::from(&integr_config_path);
    let integr_name = sanitized_path.file_stem().and_then(|s| s.to_str()).unwrap_or_default().to_string();
    if integr_name.is_empty() {
        return Err(format!("can't derive integration name from file name"));
    }

    let (integr_name, project_path) = split_config_path(&sanitized_path)?;
    let mut result = IntegrationGetResult {
        project_path: project_path.clone(),
        integr_name: integr_name.clone(),
        integr_config_path: integr_config_path.clone(),
        integr_schema: serde_json::Value::Null,
        integr_values: serde_json::Value::Null,
        error_log: Vec::new(),
    };

    let mut integration_box = crate::integrations::integration_from_name(integr_name.as_str())?;
    result.integr_schema = {
        let y: serde_yaml::Value = serde_yaml::from_str(integration_box.integr_schema()).unwrap();
        let j = serde_json::to_value(y).unwrap();
        j
    };

    if sanitized_path.exists() {
        match fs::read_to_string(&sanitized_path) {
            Ok(content) => {
                match serde_yaml::from_str::<serde_yaml::Value>(&content) {
                    Ok(y) => {
                        // XXX: wrong way to read, use _read_integrations_d
                        let j = serde_json::to_value(y).unwrap();
                        let _ = integration_box.integr_settings_apply(&j);
                    }
                    Err(e) => {
                        return Err(format!("failed to parse: {}", e.to_string()));
                    }
                };
            }
            Err(e) => {
                return Err(format!("failed to read configuration file: {}", e.to_string()));
            }
        };
    }
    result.integr_values = integration_box.integr_settings_as_json();
    Ok(result)
}


#[cfg(test)]
mod tests {
    // use super::*;
    use crate::integrations::integr_abstract::IntegrationTrait;
    use crate::integrations::yaml_schema::ISchema;
    use serde_yaml;
    use indexmap::IndexMap;
    use std::fs::File;
    use std::io::Write;

    #[tokio::test]
    async fn test_integration_schemas() {
        let integrations = crate::integrations::integrations_list();
        for name in integrations {
            let mut integration_box = crate::integrations::integration_from_name(name).unwrap();
            let schema_json = {
                let y: serde_yaml::Value = serde_yaml::from_str(integration_box.integr_schema()).unwrap();
                let j = serde_json::to_value(y).unwrap();
                j
            };
            let schema_yaml: serde_yaml::Value = serde_json::from_value(schema_json.clone()).unwrap();
            let compare_me1 = serde_yaml::to_string(&schema_yaml).unwrap();
            let schema_struct: ISchema = serde_json::from_value(schema_json).unwrap();
            let schema_struct_yaml = serde_json::to_value(&schema_struct).unwrap();
            let compare_me2 = serde_yaml::to_string(&schema_struct_yaml).unwrap();
            if compare_me1 != compare_me2 {
                eprintln!("schema mismatch for integration `{}`:\nOriginal:\n{}\nSerialized:\n{}", name, compare_me1, compare_me2);
                let original_file_path = format!("/tmp/original_schema_{}.yaml", name);
                let serialized_file_path = format!("/tmp/serialized_schema_{}.yaml", name);
                let mut original_file = File::create(&original_file_path).unwrap();
                let mut serialized_file = File::create(&serialized_file_path).unwrap();
                original_file.write_all(compare_me1.as_bytes()).unwrap();
                serialized_file.write_all(compare_me2.as_bytes()).unwrap();
                eprintln!("cat {}", original_file_path);
                eprintln!("cat {}", serialized_file_path);
                eprintln!("diff {} {}", original_file_path, serialized_file_path);
                panic!("oops");
            }
        }
    }
}
