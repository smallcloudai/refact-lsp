use serde::{Deserialize, Serialize};
use serde_yaml;
use indexmap::IndexMap;
use std::collections::HashMap;
use std::sync::Arc;
use crate::call_validation::ChatMessage;


#[derive(Serialize, Deserialize, Debug, Default)]
pub struct DockerService {
    pub image: String,
    #[serde(default)]
    pub environment: IndexMap<String, String>,
    #[serde(default)]
    pub smartlinks: Vec<ISmartLink>,
}

#[derive(Serialize, Deserialize, Debug, Default)]
pub struct ISchemaField {
    pub f_type: String,
    #[serde(default)]
    pub f_desc: String,
    #[serde(default)]
    pub f_default: String,
    #[serde(default)]
    pub f_placeholder: String,
    #[serde(default)]
    pub smartlinks: Vec<ISmartLink>,
}

#[derive(Serialize, Deserialize, Debug, Default)]
pub struct ISmartLink {
    pub sl_label: String,
    #[serde(default)]
    pub sl_chat: Vec<ChatMessage>,
    #[serde(default)]
    pub sl_goto: String,
}

#[derive(Serialize, Deserialize, Debug, Default)]
pub struct ISchemaAvailable {
    pub possible: bool,
}

#[derive(Serialize, Deserialize, Debug, Default)]
pub struct ISchemaDocker {
    pub new_container_default: DockerService,
}

#[derive(Serialize, Deserialize, Debug, Default)]
pub struct ISchema {
    pub fields: HashMap<String, ISchemaField>,
    pub available: HashMap<String, ISchemaAvailable>,
    pub docker: ISchemaDocker,
    #[serde(default)]
    pub smartlinks: Vec<ISmartLink>,
}
