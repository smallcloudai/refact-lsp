use std::fs;
use std::path::PathBuf;
use std::sync::Arc;
use indexmap::IndexMap;
use serde::Deserialize;
use tokio::sync::{Mutex as AMutex, RwLock as ARwLock};

use crate::global_context::GlobalContext;
use crate::tools::tools_description::Tool;
use crate::yaml_configs::create_configs::{integrations_enabled_cfg, read_yaml_into_value};

#[derive(Deserialize, Default)]
pub struct YamlError {
    pub integr_config_path: String,
    pub error_line: usize,  // starts with 1, zero if invalid
}

#[derive(Deserialize, Default)]
pub struct IntegrationWithIconRecord {
    pub integr_name: String,
    pub integr_icon: String,
    pub integr_config_path: String,
    pub integr_enable: bool,
}

#[derive(Deserialize, Default)]
pub struct IntegrationWithIconResult {
    pub integrations: Vec<IntegrationWithIconRecord>,
    pub error_log: Vec<YamlError>,
}

fn _load_everything_in_integrations_d(
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
                        });
                        tracing::warn!("failed to parse {}{}: {}", path.display(), location, e.to_string());
                    }
                },
                Err(e) => {
                    error_log.push(YamlError {
                        integr_config_path: path.display().to_string(),
                        error_line: 0,
                    });
                    tracing::warn!("failed to read {}: {}", path.display(), e.to_string());
                }
            }
        }
    }

    let d_path = config_dir.join("integrations.d");
    if let Ok(entries) = fs::read_dir(&d_path) {
        for entry in entries {
            if let Ok(entry) = entry {
                let path = entry.path();
                if path.extension().and_then(|s| s.to_str()) == Some("yaml") {
                    if !lst.iter().any(|&name| path.ends_with(format!("{}.yaml", name))) {
                        tracing::warn!("unrecognized file: {}", path.display());
                    }
                }
            }
        }
    }

    context_file_map
}

fn _calc_integr_config_path(config_dir: &PathBuf, integr_name: &str) -> String {
    config_dir.join("integrations.d").join(format!("{}.yaml", integr_name)).to_string_lossy().into_owned()
}

pub async fn all_integrations_with_icon(
    gcx: Arc<ARwLock<GlobalContext>>,
) {
    let config_dir = gcx.read().await.config_dir.clone();
    let lst: Vec<&str> = crate::integrations::integrations_list();
    let mut error_log: Vec<YamlError> = Vec::new();
    let name2cfg = _load_everything_in_integrations_d(&config_dir, &mut error_log, &lst);

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
                    tracing::info!("disabled {}", n);
                    false
                }
            } else {
                tracing::info!("no enable field {}", n);
                false
            }
        } else {
            tracing::info!("no config file {}", n);
            false
        };
    }
}

// pub async fn all_integrations_with_schema(
//     gcx: Arc<ARwLock<GlobalContext>>,
// ) -> Result<(), String> {
// // ) -> Result<IndexMap<String, Box<dyn IntegrationTrait + Send + Sync>>, String> {
//     let config_dir = gcx.read().await.config_dir.clone();
//     let lst: Vec<&str> = crate::integrations::integrations_list();
//     let error_log: Vec<YamlError> = Vec::new();
//     let name2value = _load_everything_in_integrations_d(config_dir, &mut error_log);
//     for n in lst.iter() {
//         let have_cfg = name2value.get(n);
//         if Some(cfg) = have_cfg {
//             let integration_box = crate::integrations::integration_from_name(n);
//         }
//     }

    // let integrations_yaml_value = read_yaml_into_value(&cache_dir.join("integrations.yaml")).await?;

    // let mut results = IndexMap::new();
    // for (i_name, mut i) in integrations {
    //     let path = get_integration_path(&cache_dir, &i_name);
    //     let j_value = json_for_integration(&path, integrations_yaml_value.get(&i_name), &i).await?;

    //     if j_value.get("detail").is_some() {
    //         tracing::warn!("failed to load integration {}: {}", i_name, j_value.get("detail").unwrap());
    //     } else {
    //         if let Err(e) = i.integr_settings_apply(&j_value) {
    //             tracing::warn!("failed to load integration {}: {}", i_name, e);
    //         };
    //     }
    //     results.insert(i_name.clone(), i);
    // }

    // Ok(results)
// }

