use std::fs;
use std::path::PathBuf;
use std::sync::Arc;
use indexmap::IndexMap;
use serde::Serialize;
use tokio::sync::RwLock as ARwLock;

use crate::global_context::GlobalContext;
use crate::tools::tools_description::Tool;
use crate::yaml_configs::create_configs::{integrations_enabled_cfg, read_yaml_into_value};


#[derive(Serialize, Default)]
pub struct YamlError {
    pub integr_config_path: String,
    pub error_line: usize,  // starts with 1, zero if invalid
    pub error_msg: String,
}

#[derive(Serialize, Default)]
pub struct IntegrationWithIconRecord {
    pub integr_name: String,
    pub integr_icon: String,
    pub integr_config_path: String,
    pub integr_enable: bool,
}

#[derive(Serialize, Default)]
pub struct IntegrationWithIconResult {
    pub integrations: Vec<IntegrationWithIconRecord>,
    pub error_log: Vec<YamlError>,
}

fn _read_integrations_d(
    config_dir: &PathBuf,
    error_log: &mut Vec<YamlError>,
    lst: &[&str],
) -> IndexMap<String, serde_yaml::Value> {
    let mut context_file_map = IndexMap::new();

    for integr_name in lst.iter() {
        let path = PathBuf::from(_calc_integr_config_path(config_dir, integr_name));
        if path.extension().and_then(|s| s.to_str()) == Some("yaml") {
            match fs::read_to_string(&path) {
                Ok(file_content) => match serde_yaml::from_str(&file_content) {
                    Ok(yaml_value) => {
                        context_file_map.insert(integr_name.to_string(), yaml_value);
                    }
                    Err(e) => {
                        let location = e.location().map(|loc| format!(" at line {}, column {}", loc.line(), loc.column())).unwrap_or_default();
                        error_log.push(YamlError {
                            integr_config_path: path.display().to_string(),
                            error_line: e.location().map(|loc| loc.line()).unwrap_or(0),
                            error_msg: e.to_string(),
                        });
                        tracing::warn!("failed to parse {}{}: {}", path.display(), location, e.to_string());
                    }
                },
                Err(e) => {
                    error_log.push(YamlError {
                        integr_config_path: path.display().to_string(),
                        error_line: 0,
                        error_msg: e.to_string(),
                    });
                    tracing::warn!("failed to read {}: {}", path.display(), e.to_string());
                }
            }
        }
    }

    // let d_path = config_dir.join("integrations.d");
    // if let Ok(entries) = fs::read_dir(&d_path) {
    //     for entry in entries {
    //         if let Ok(entry) = entry {
    //             let path = entry.path();
    //             if path.extension().and_then(|s| s.to_str()) == Some("yaml") {
    //                 if !lst.iter().any(|&name| path.ends_with(format!("{}.yaml", name))) {
    //                     tracing::warn!("unrecognized file: {}", path.display());
    //                 }
    //             }
    //         }
    //     }
    // }

    context_file_map
}

fn _calc_integr_config_path(config_dir: &PathBuf, integr_name: &str) -> String {
    config_dir.join("integrations.d").join(format!("{}.yaml", integr_name)).to_string_lossy().into_owned()
}

pub async fn integrations_all_with_icons(
    gcx: Arc<ARwLock<GlobalContext>>,
) -> IntegrationWithIconResult {
    let config_dir = gcx.read().await.config_dir.clone();
    let lst: Vec<&str> = crate::integrations::integrations_list();
    let mut error_log: Vec<YamlError> = Vec::new();
    let mut integrations = Vec::new();
    let name2cfg = _read_integrations_d(&config_dir, &mut error_log, &lst);
    for n in lst.iter() {
        let mut rec: IntegrationWithIconRecord = Default::default();
        rec.integr_name = n.to_string();
        rec.integr_config_path = _calc_integr_config_path(&config_dir, n);
        rec.integr_icon = crate::integrations::icon_from_name(n);
        rec.integr_enable = if let Some(cfg) = name2cfg.get(*n) {
            if let Some(enable) = cfg.get("enable").and_then(|v| v.as_bool()) {
                if enable {
                    true
                } else {
                    tracing::info!("disabled `{}`", n);
                    false
                }
            } else {
                tracing::info!("no enable field `{}`", n);
                false
            }
        } else {
            tracing::info!("no config file `{}`", n);
            false
        };
        integrations.push(rec);
    }
    IntegrationWithIconResult {
        integrations,
        error_log,
    }
}

#[derive(Serialize, Default)]
pub struct IntegrationGetResult {
    pub integr_name: String,
    pub integr_config_path: String,
    pub integr_schema: serde_json::Value,
    pub integr_values: serde_json::Value,
    pub error_log: Vec<YamlError>,
}

pub async fn integration_config_get(
    gcx: Arc<ARwLock<GlobalContext>>,
    integr_config_path: String,
) -> Result<IntegrationGetResult, String> {
    let sanitized_path = PathBuf::from(&integr_config_path);
    let integr_name = sanitized_path.file_stem().and_then(|s| s.to_str()).unwrap_or_default().to_string();
    if integr_name.is_empty() {
        return Err(format!("can't derive integration name from file name"));
    }

    let mut result = IntegrationGetResult {
        integr_name: integr_name.clone(),
        integr_config_path: integr_config_path.clone(),
        integr_schema: serde_json::Value::Null,
        integr_values: serde_json::Value::Null,
        error_log: Vec::new(),
    };

    // let config_dir = gcx.read().await.config_dir.clone();
    // let name2cfg = _read_integrations_d(&config_dir, &mut result.error_log, &vec![integr_name.as_str()]);
    // let cfg = if let Some(cfg) = name2cfg.get(integr_name.as_str()) {
    //     cfg
    // } else {
    //     return Err(format!("No configuration found for integration: {}", integr_name));
    // };
    let mut integration_box = crate::integrations::integration_from_name(integr_name.as_str())?;
    result.integr_schema = integration_box.integr_schema();
    if sanitized_path.exists() {
        match fs::read_to_string(&sanitized_path) {
            Ok(content) => {
                match serde_yaml::from_str::<serde_yaml::Value>(&content) {
                    Ok(y) => {
                        let j = serde_json::to_value(y).unwrap();
                        integration_box.integr_settings_apply(&j);
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
            let schema_json = integration_box.integr_schema();
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
