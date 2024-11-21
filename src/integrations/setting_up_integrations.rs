use std::path::PathBuf;
use std::sync::Arc;
use indexmap::IndexMap;
use tokio::sync::RwLock as ARwLock;
use serde::{Deserialize, Serialize};
use serde_json::json;
use tokio::fs::OpenOptions;
use tokio::io::AsyncWriteExt;

use crate::global_context::GlobalContext;
use crate::integrations::{get_integrations, integration_from_name};


#[derive(Serialize, Default)]
pub struct YamlError {
    pub integr_config_path: String,
    pub error_line: usize,  // starts with 1, zero if invalid
    pub error_msg: String,
}

#[derive(Default, Clone)]
pub struct IntegrationExtra {
    pub integr_path: String,
    pub on_your_laptop: bool,
    pub when_isolated: bool,
}

#[derive(Serialize, Default)]
pub struct IntegrationRecord {
    pub scope: String,
    pub integr_name: String,
    pub integr_config_exists: bool,
    pub on_your_laptop: bool,
    pub when_isolated: bool,
}

#[derive(Serialize, Default)]
pub struct IntegrationContent {
    pub scope: String,
    pub integr_name: String,
    pub integr_schema: serde_json::Value,
    pub integr_value: serde_json::Value,
    pub error_log: Vec<String>,
}

#[derive(Deserialize)]
pub struct IntegrationsFilter {
    #[serde(default)]
    pub scope: Option<String>,
    #[serde(default)]
    pub name: Option<String>
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

pub async fn get_integration_records(gcx: Arc<ARwLock<GlobalContext>>) -> Result<Vec<IntegrationRecord>, String> {
    let (integrations, _errors) = get_integrations(gcx.clone()).await?;
    let mut resutls = vec![];

    for (i_scope, scope_integrations) in integrations {
        for (i_name, (_i, i_extra)) in scope_integrations {
            let rec = IntegrationRecord {
                scope: i_scope.clone(),
                integr_name: i_name.clone(),
                integr_config_exists: PathBuf::from(i_extra.integr_path.clone()).exists(),
                on_your_laptop: i_extra.on_your_laptop,
                when_isolated: i_extra.when_isolated,
            };
            resutls.push(rec)
        }
    }
    Ok(resutls)
}

pub async fn get_integration_contents_with_filter(
    gcx: Arc<ARwLock<GlobalContext>>,
    filter: &IntegrationsFilter,
) -> Result<Vec<IntegrationContent>, String> {
    let (integrations, _errors) = get_integrations(gcx.clone()).await?;

    let filtered_integrations: IndexMap<_, _> = integrations.into_iter()
        .filter(|(scope, _)| filter.scope.as_ref().map_or(true, |s| s == scope))
        .collect();
    let filtered_integrations: IndexMap<_, _> = filtered_integrations.into_iter()
        .map(|(scope, scope_integrations)| {
            let filtered_scope_integrations: IndexMap<_, _> = scope_integrations
                .into_iter()
                .filter(|(i_name, _)| filter.name.as_ref().map_or(true, |n| n == i_name))
                .collect();
            (scope, filtered_scope_integrations)
        }).collect();

    let mut results = vec![];
    for (scope, scope_integrations) in filtered_integrations {
        for (i_name, (i, _i_extra)) in scope_integrations {
            let integr_schema_yaml: serde_yaml::Value = serde_yaml::from_str(i.integr_schema())
                .map_err(|e| format!("Failed to parse integration schema for integration {}: {}", i_name, e))?;
            let integr_schema = serde_json::to_value(integr_schema_yaml)
                .map_err(|e| format!("Failed to convert integration schema to JSON for integration {}: {}", i_name, e))?;
            
            let cont = IntegrationContent {
                scope: scope.clone(),
                integr_name: i_name.clone(),
                integr_schema,
                integr_value: i.integr_settings_as_json(),
                error_log: vec![], // todo: implement
            };
            results.push(cont);
        }
    }
    
    Ok(results)
}

pub async fn save_integration_value(
    gcx: Arc<ARwLock<GlobalContext>>,
    integr_scope: &String,
    integr_name: &String,
    integr_value: &serde_json::Value,
) -> Result<(), String> {
    let (integrations, _errors) = get_integrations(gcx.clone()).await?;
    let mut i = integration_from_name(integr_name)?;

    let integrations_in_scope = match integrations.get(integr_scope) {
        Some(s) => s,
        None => {
            return Err(format!("integration scope '{}' doesn't exist", integr_scope));
        }
    };
    
    let i_extra = integrations_in_scope.get(integr_name)
        .map(|(_i, i_extra)|i_extra).cloned()
        .unwrap_or(IntegrationExtra::default());
    
    let i_path = if i_extra.integr_path.is_empty() {
        get_integration_path(&integr_scope, integr_name)?
    } else {
        PathBuf::from(i_extra.integr_path.clone())
    };
    
    i.integr_settings_apply(integr_value)?;
    
    let mut j_value = i.integr_settings_as_json();
    j_value["available"] = json!({
        "on_your_laptop": i_extra.on_your_laptop,
        "when_isolated": i_extra.when_isolated,
    });
    
    let y_value = serde_yaml::to_value(&j_value).map_err(|e| format!("Failed to convert JSON to YAML: {}", e))?;
    let y_value_string = serde_yaml::to_string(&y_value).map_err(|e| format!("Failed to convert YAML to string: {}", e))?;

    let mut file = OpenOptions::new()
        .create(true)
        .truncate(true)
        .write(true)
        .open(&i_path)
        .await
        .map_err(|e| format!("Failed to open file {}: {}", i_path.display(), e))?;

    file.write_all(y_value_string.as_bytes()).await
        .map_err(|e| format!("Failed to write to file {}: {}", i_path.display(), e))?;

    Ok(())
}

pub fn get_integration_path(
    scope: &String,
    integr_name: &String,
) -> Result<PathBuf, String> {
    if scope.is_empty() || scope == "global" {
        return Err("cannot resolve integration path: scope is empty or 'global' and could not find integration config path".to_string());
    }
    let scope_as_path = PathBuf::from(scope);
    if scope_as_path.extension().unwrap_or_default() == "yaml" {
        Ok(scope_as_path)
    } else {
        Ok(scope_as_path.join(".refact").join(".integrations.d").join(integr_name).with_extension("yaml"))
    }
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
