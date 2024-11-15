use serde::{Deserialize, Serialize};
use serde_yaml;
use indexmap::IndexMap;
use std::collections::HashMap;
use std::sync::Arc;
use crate::call_validation::ChatMessage;


#[derive(Serialize, Deserialize, Default)]
pub struct DockerService {
    pub image: String,
    pub environment: IndexMap<String, String>,
    pub smartlinks: Vec<ISmartLink>,
}

#[derive(Serialize, Deserialize, Default)]
pub struct ISchemaField {
    pub f_type: String,
    pub f_desc: Option<String>,
    pub f_default: Option<String>,
    pub f_placeholder: Option<String>,
}

#[derive(Serialize, Deserialize, Default)]
pub struct ISmartLink {
    pub sl_label: String,
    pub sl_chat: Vec<ChatMessage>,
}

#[derive(Serialize, Deserialize, Default)]
pub struct ISchemaAvailable {
    pub possible: bool,
}

#[derive(Serialize, Deserialize, Default)]
pub struct ISchemaDocker {
    pub new_container_default: DockerService,
}

#[derive(Serialize, Deserialize, Default)]
pub struct ISchema {
    pub fields: HashMap<String, ISchemaField>,
    pub available: HashMap<String, ISchemaAvailable>,
    pub docker: ISchemaDocker,
    pub smartlinks: Vec<ISmartLink>,
}
