use std::fs;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::RwLock as ARwLock;
use regex::Regex;
use serde::Serialize;
use tokio::fs as async_fs;
use tokio::io::AsyncWriteExt;
use crate::global_context::GlobalContext;
use crate::integrations::get_integrations;


#[derive(Serialize, Default)]
pub struct YamlError {
    pub integr_config_path: String,
    pub error_line: usize,  // starts with 1, zero if invalid
    pub error_msg: String,
}

#[derive(Default)]
pub struct IntegrationExtra {
    pub integr_path: String,
    pub on_your_laptop: bool,
    pub when_isolated: bool,
}

#[derive(Serialize, Default)]
pub struct IntegrationRecord {
    pub project_path: String,
    pub integr_name: String,
    pub integr_config_path: String,
    pub integr_config_exists: bool,
    pub on_your_laptop: bool,
    pub when_isolated: bool,
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

pub fn integration_extra_from_yaml(value: &serde_yaml::Value) -> IntegrationExtra {
    let mut extra = IntegrationExtra::default();
    if let Some(available) = value.get("available").and_then(|v| v.as_mapping()) {
        extra.on_your_laptop = available.get("on_your_laptop").and_then(|v|    
            v.as_bool()).unwrap_or(false);
        extra.when_isolated = available.get("when_isolated").and_then(|v| v.as_bool()).unwrap_or(false);
    }
    extra
}

async fn get_integration_records(gcx: Arc<ARwLock<GlobalContext>>) -> Result<Vec<IntegrationRecord>, String> {
    let (integrations, _errors) = get_integrations(gcx.clone()).await?;
    let mut resutls = vec![];
    
    for (i_scope, scope_integrations) in integrations {
        for (i_name, (i, i_extra)) in scope_integrations {
            let rec = IntegrationRecord {
                project_path: if i_scope == "global" {"".to_string()} else {i_scope.clone()},
                integr_name: i_name.clone(),
                integr_config_path: i_extra.integr_path.clone(),
                integr_config_exists: PathBuf::from(i_extra.integr_path.clone()).exists(),
                on_your_laptop: i_extra.on_your_laptop,
                when_isolated: i_extra.when_isolated,
            };
            resutls.push(rec)
        }
    }
    Ok(resutls)
}

pub fn split_path_into_project_and_integration(cfg_path: &PathBuf) -> Result<(String, String), String> {
    let path_str = cfg_path.to_string_lossy();
    let re_per_project = Regex::new(r"^(.*)[\\/]\.refact[\\/](integrations\.d)[\\/](.+)\.yaml$").unwrap();
    let re_global = Regex::new(r"^(.*)[\\/]\.config[\\/](refact[\\/](integrations\.d)[\\/](.+)\.yaml$)").unwrap();

    if let Some(caps) = re_per_project.captures(&path_str) {
        let project_path = caps.get(1).map_or(String::new(), |m| m.as_str().to_string());
        let integr_name = caps.get(3).map_or(String::new(), |m| m.as_str().to_string());
        Ok((integr_name, project_path))
    } else if let Some(caps) = re_global.captures(&path_str) {
        let integr_name = caps.get(4).map_or(String::new(), |m| m.as_str().to_string());
        Ok((integr_name, String::new()))
    } else {
        Err(format!("invalid path: {}", cfg_path.display()))
    }
}

pub async fn integration_config_get(
    integr_config_path: String,
) -> Result<IntegrationGetResult, String> {
    let sanitized_path = crate::files_correction::canonical_path(&integr_config_path);
    let integr_name = sanitized_path.file_stem().and_then(|s| s.to_str()).unwrap_or_default().to_string();
    if integr_name.is_empty() {
        return Err(format!("can't derive integration name from file name"));
    }

    let (integr_name, project_path) = split_path_into_project_and_integration(&sanitized_path)?;
    let mut result = IntegrationGetResult {
        project_path,
        integr_name: integr_name.clone(),
        integr_config_path,
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

    let mut available = serde_json::json!({
        "on_your_laptop": false,
        "when_isolated": false
    });
    if sanitized_path.exists() {
        match fs::read_to_string(&sanitized_path) {
            Ok(content) => {
                match serde_yaml::from_str::<serde_yaml::Value>(&content) {
                    Ok(y) => {
                        let j = serde_json::to_value(y).unwrap();
                        available["on_your_laptop"] = j.get("available").and_then(|v| v.get("on_your_laptop")).and_then(|v| v.as_bool()).unwrap_or(false).into();
                        available["when_isolated"] = j.get("available").and_then(|v| v.get("when_isolated")).and_then(|v| v.as_bool()).unwrap_or(false).into();
                        let did_it_work = integration_box.integr_settings_apply(&j);
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
    result.integr_values["available"] = available;
    Ok(result)
}

pub async fn integration_config_save(
    integr_config_path: &String,
    integr_values: &serde_json::Value,
) -> Result<(), String> {
    let config_path = crate::files_correction::canonical_path(integr_config_path);
    let (integr_name, _project_path) = crate::integrations::setting_up_integrations::split_path_into_project_and_integration(&config_path)
        .map_err(|e| format!("Failed to split path: {}", e))?;
    let mut integration_box = crate::integrations::integration_from_name(integr_name.as_str())
        .map_err(|e| format!("Failed to load integrations: {}", e))?;

    integration_box.integr_settings_apply(integr_values)?;  // this will produce "no field XXX" errors

    let mut sanitized_json: serde_json::Value = integration_box.integr_settings_as_json();
    tracing::info!("posted values:\n{}", serde_json::to_string_pretty(integr_values).unwrap());
    if !sanitized_json.as_object_mut().unwrap().contains_key("available") {
        sanitized_json["available"] = serde_json::Value::Object(serde_json::Map::new());
    }
    sanitized_json["available"]["on_your_laptop"] = integr_values.pointer("/available/on_your_laptop").cloned().unwrap_or(serde_json::Value::Bool(false));
    sanitized_json["available"]["when_isolated"] = integr_values.pointer("/available/when_isolated").cloned().unwrap_or(serde_json::Value::Bool(false));
    tracing::info!("writing to {}:\n{}", config_path.display(), serde_json::to_string_pretty(&sanitized_json).unwrap());
    let sanitized_yaml = serde_yaml::to_value(sanitized_json).unwrap();

    let config_dir = config_path.parent().ok_or_else(|| {
        "Failed to get parent directory".to_string()
    })?;
    async_fs::create_dir_all(config_dir).await.map_err(|e| {
        format!("Failed to create {}: {}", config_dir.display(), e)
    })?;

    let mut file = async_fs::File::create(&config_path).await.map_err(|e| {
        format!("Failed to create {}: {}", config_path.display(), e)
    })?;
    let sanitized_yaml_string = serde_yaml::to_string(&sanitized_yaml).unwrap();
    file.write_all(sanitized_yaml_string.as_bytes()).await.map_err(|e| {
        format!("Failed to write to {}: {}", config_path.display(), e)
    })?;

    Ok(())
}

// todo: restore
// #[cfg(test)]
// mod tests {
//     // use super::*;
//     use crate::integrations::integr_abstract::IntegrationTrait;
//     use crate::integrations::yaml_schema::ISchema;
//     use serde_yaml;
//     use indexmap::IndexMap;
//     use std::fs::File;
//     use std::io::Write;
// 
//     #[tokio::test]
//     async fn test_integration_schemas() {
//         let integrations = crate::integrations::integrations_list();
//         for name in integrations {
//             let mut integration_box = crate::integrations::integration_from_name(name).unwrap();
//             let schema_json = {
//                 let y: serde_yaml::Value = serde_yaml::from_str(integration_box.integr_schema()).unwrap();
//                 let j = serde_json::to_value(y).unwrap();
//                 j
//             };
//             let schema_yaml: serde_yaml::Value = serde_json::from_value(schema_json.clone()).unwrap();
//             let compare_me1 = serde_yaml::to_string(&schema_yaml).unwrap();
//             let schema_struct: ISchema = serde_json::from_value(schema_json).unwrap();
//             let schema_struct_yaml = serde_json::to_value(&schema_struct).unwrap();
//             let compare_me2 = serde_yaml::to_string(&schema_struct_yaml).unwrap();
//             if compare_me1 != compare_me2 {
//                 eprintln!("schema mismatch for integration `{}`:\nOriginal:\n{}\nSerialized:\n{}", name, compare_me1, compare_me2);
//                 let original_file_path = format!("/tmp/original_schema_{}.yaml", name);
//                 let serialized_file_path = format!("/tmp/serialized_schema_{}.yaml", name);
//                 let mut original_file = File::create(&original_file_path).unwrap();
//                 let mut serialized_file = File::create(&serialized_file_path).unwrap();
//                 original_file.write_all(compare_me1.as_bytes()).unwrap();
//                 serialized_file.write_all(compare_me2.as_bytes()).unwrap();
//                 eprintln!("cat {}", original_file_path);
//                 eprintln!("cat {}", serialized_file_path);
//                 eprintln!("diff {} {}", original_file_path, serialized_file_path);
//                 panic!("oops");
//             }
//         }
//     }
// }
