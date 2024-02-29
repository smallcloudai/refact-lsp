use tracing::{info, error};
use serde::Deserialize;
use serde::Serialize;
use std::fs::File;
use std::collections::HashMap;
use std::io::Read;
use std::sync::Arc;
use std::sync::RwLock as StdRwLock;
use tokio::sync::RwLock;
use url::Url;
use crate::global_context::GlobalContext;
use crate::known_models::KNOWN_MODELS;

const CAPS_FILENAME: &str = "coding_assistant_caps.json";


#[derive(Debug, Serialize, Deserialize, Clone, Default)]
pub struct ModelRecord {
    pub n_ctx: usize,
    #[serde(default)]
    pub supports_scratchpads: HashMap<String, serde_json::Value>,
    #[serde(default)]
    pub default_scratchpad: String,
    #[serde(default)]
    pub similar_models: Vec<String>,
}

#[derive(Debug, Deserialize)]
pub struct ModelsOnly {
    pub code_completion_models: HashMap<String, ModelRecord>,
    pub code_chat_models: HashMap<String, ModelRecord>,
}

#[derive(Debug, Serialize, Deserialize, Clone, Default)]
pub struct CodeAssistantCaps {
    pub cloud_name: String,
    pub endpoint_style: String,
    pub endpoint_template: String,
    #[serde(default)]
    pub endpoint_chat_passthrough: String,
    pub tokenizer_path_template: String,
    pub tokenizer_rewrite_path: HashMap<String, String>,
    pub telemetry_basic_dest: String,
    #[serde(default)]
    pub telemetry_basic_retrieve_my_own: String,
    #[serde(default)]
    pub telemetry_corrected_snippets_dest: String,
    #[serde(default)]
    pub code_completion_models: HashMap<String, ModelRecord>,
    pub code_completion_default_model: String,
    #[serde(default)]
    pub code_completion_n_ctx: usize,
    #[serde(default)]
    pub code_chat_models: HashMap<String, ModelRecord>,
    pub code_chat_default_model: String,
    #[serde(default)]
    pub default_embeddings_model: String,
    #[serde(default)]
    pub endpoint_embeddings_template: String,
    #[serde(default)]
    pub endpoint_embeddings_style: String,
    #[serde(default)]
    pub size_embeddings: i32,
    pub running_models: Vec<String>,
    #[serde(default)]
    pub caps_version: i64,  // need to reload if it increases on server, that happens when server configuration changes
}

pub async fn load_caps(
    cmdline: crate::global_context::CommandLine,
    global_context: Arc<RwLock<GlobalContext>>,
) -> Result<Arc<StdRwLock<CodeAssistantCaps>>, String> {
    let mut buffer = String::new();
    let mut is_local_file = false;
    let mut is_remote_address = false;
    let caps_url: String;
    if cmdline.address_url == "Refact" {
        is_remote_address = true;
        caps_url = "https://inference.smallcloud.ai/coding_assistant_caps.json".to_string();
    } else if cmdline.address_url == "HF" {
        buffer = HF_DEFAULT_CAPS.to_string();
        caps_url = "<compiled-in-caps-hf>".to_string();
    } else {
        if cmdline.address_url.starts_with("http") {
            is_remote_address = true;
            let base_url = Url::parse(&cmdline.address_url.clone()).map_err(|_| "failed to parse address url (1)".to_string())?;
            let joined_url = base_url.join(&CAPS_FILENAME).map_err(|_| "failed to parse address url (2)".to_string())?;
            caps_url = joined_url.to_string();
        } else {
            is_local_file = true;
            caps_url = cmdline.address_url.clone();
        }
    }
    if is_local_file {
        let mut file = File::open(caps_url.clone()).map_err(|_| format!("failed to open file '{}'", caps_url))?;
        file.read_to_string(&mut buffer).map_err(|_| format!("failed to read file '{}'", caps_url))?;
    }
    if is_remote_address {
        let api_key = cmdline.api_key.clone();
        let http_client = global_context.read().await.http_client.clone();
        let mut headers = reqwest::header::HeaderMap::new();
        if !api_key.is_empty() {
            headers.insert(reqwest::header::AUTHORIZATION, reqwest::header::HeaderValue::from_str(format!("Bearer {}", api_key).as_str()).unwrap());
        }
        let response = http_client.get(caps_url.clone()).headers(headers).send().await.map_err(|e| format!("{}", e))?;
        let status = response.status().as_u16();
        buffer = response.text().await.map_err(|e| format!("failed to read response: {}", e))?;
        if status != 200 {
            return Err(format!("server responded with: {}", buffer));
        }
    }
    info!("reading caps from {}", caps_url);
    let r0: ModelsOnly = serde_json::from_str(&KNOWN_MODELS).map_err(|e| {
        let up_to_line = KNOWN_MODELS.lines().take(e.line()).collect::<Vec<&str>>().join("\n");
        error!("{}\nfailed to parse KNOWN_MODELS: {}", up_to_line, e);
        format!("failed to parse KNOWN_MODELS: {}", e)
    })?;
    let mut r1: CodeAssistantCaps = serde_json::from_str(&buffer).map_err(|e| {
        let up_to_line = buffer.lines().take(e.line()).collect::<Vec<&str>>().join("\n");
        error!("{}\nfailed to parse {}: {}", up_to_line, caps_url, e);
        format!("failed to parse {}: {}", caps_url, e)
    })?;
    _inherit_r1_from_r0(&mut r1, &r0);
    r1.endpoint_template = relative_to_full_url(&caps_url, &r1.endpoint_template)?;
    r1.endpoint_chat_passthrough = relative_to_full_url(&caps_url, &r1.endpoint_chat_passthrough)?;
    r1.telemetry_basic_dest = relative_to_full_url(&caps_url, &r1.telemetry_basic_dest)?;
    r1.telemetry_corrected_snippets_dest = relative_to_full_url(&caps_url, &r1.telemetry_corrected_snippets_dest)?;
    r1.telemetry_basic_retrieve_my_own = relative_to_full_url(&caps_url, &r1.telemetry_basic_retrieve_my_own)?;
    r1.endpoint_embeddings_template = relative_to_full_url(&caps_url, &r1.endpoint_embeddings_template)?;
    r1.tokenizer_path_template = relative_to_full_url(&caps_url, &r1.tokenizer_path_template)?;
    info!("caps {} completion models", r1.code_completion_models.len());
    info!("caps default completion model: \"{}\"", r1.code_completion_default_model);
    info!("caps {} chat models", r1.code_chat_models.len());
    info!("caps default chat model: \"{}\"", r1.code_chat_default_model);
    Ok(Arc::new(StdRwLock::new(r1)))
}


fn relative_to_full_url(
    caps_url: &String,
    maybe_relative_url: &str,
) -> Result<String, String> {
    if maybe_relative_url.starts_with("http") {
        Ok(maybe_relative_url.to_string())
    } else if maybe_relative_url.is_empty() {
        Ok("".to_string())
    } else {
        let base_url = Url::parse(caps_url.as_str()).map_err(|_| "failed to parse address url (3)".to_string())?;
        let joined_url = base_url.join(maybe_relative_url).map_err(|_| "failed to join URL \"{}\" and possibly relative \"{}\"".to_string())?;
        Ok(joined_url.to_string())
    }
}

fn _inherit_r1_from_r0(
    r1: &mut CodeAssistantCaps,
    r0: &ModelsOnly,
) {
    // inherit models from r0, only if not already present in r1
    for k in r0.code_completion_models.keys() {
        if !r1.code_completion_models.contains_key(k) {
            r1.code_completion_models.insert(k.to_string(), r0.code_completion_models[k].clone());
        }
    }
    for k in r0.code_chat_models.keys() {
        if !r1.code_chat_models.contains_key(k) {
            r1.code_chat_models.insert(k.to_string(), r0.code_chat_models[k].clone());
        }
    }
    // clone to "similar_models"
    let ccmodel_keys_copy = r1.code_completion_models.keys().cloned().collect::<Vec<String>>();
    for k in ccmodel_keys_copy {
        let model_rec = r1.code_completion_models[&k].clone();
        for similar_model in model_rec.similar_models.iter() {
            r1.code_completion_models.insert(similar_model.to_string(), model_rec.clone());
        }
    }
    let chatmodel_keys_copy = r1.code_chat_models.keys().cloned().collect::<Vec<String>>();
    for k in chatmodel_keys_copy {
        let model_rec = r1.code_chat_models[&k].clone();
        for similar_model in model_rec.similar_models.iter() {
            r1.code_chat_models.insert(similar_model.to_string(), model_rec.clone());
        }
    }
    r1.code_completion_models = r1.code_completion_models.clone().into_iter().filter(|(k, _)| r1.running_models.contains(&k)).collect();
    r1.code_chat_models = r1.code_chat_models.clone().into_iter().filter(|(k, _)| r1.running_models.contains(&k)).collect();

    for k in r1.running_models.iter() {
        if !r1.code_completion_models.contains_key(k) && !r1.code_chat_models.contains_key(k) {
            info!("indicated as running, unknown model {}", k);
        }
    }
}

pub fn which_model_to_use<'a>(
    models: &'a HashMap<String, ModelRecord>,
    user_wants_model: &str,
    default_model: &str,
) -> Result<(String, &'a ModelRecord), String> {
    let mut take_this_one = default_model;
    if user_wants_model != "" {
        take_this_one = user_wants_model;
    }
    if let Some(model_rec) = models.get(take_this_one) {
        return Ok((take_this_one.to_string(), model_rec));
    } else {
        return Err(format!(
            "Model '{}' not found. Server has these models: {:?}",
            take_this_one,
            models.keys()
        ));
    }
}

pub fn which_scratchpad_to_use<'a>(
    scratchpads: &'a HashMap<String, serde_json::Value>,
    user_wants_scratchpad: &str,
    default_scratchpad: &str,
) -> Result<(String, &'a serde_json::Value), String> {
    let mut take_this_one = default_scratchpad;
    if user_wants_scratchpad != "" {
        take_this_one = user_wants_scratchpad;
    }
    if default_scratchpad == "" {
        if scratchpads.len() == 1 {
            let key = scratchpads.keys().next().unwrap();
            return Ok((key.clone(), &scratchpads[key]));
        } else {
            return Err(format!(
                "There is no default scratchpad defined, requested scratchpad is empty. The model supports these scratchpads: {:?}",
                scratchpads.keys()
            ));
        }
    }
    if let Some(scratchpad_patch) = scratchpads.get(take_this_one) {
        return Ok((take_this_one.to_string(), scratchpad_patch));
    } else {
        return Err(format!(
            "Scratchpad '{}' not found. The model supports these scratchpads: {:?}",
            take_this_one,
            scratchpads.keys()
        ));
    }
}

const HF_DEFAULT_CAPS: &str = r#"
{
    "cloud_name": "Hugging Face",
    "endpoint_template": "https://api-inference.huggingface.co/models/$MODEL",
    "endpoint_style": "hf",
    "tokenizer_path_template": "https://huggingface.co/$MODEL/resolve/main/tokenizer.json",
    "tokenizer_rewrite_path": {
        "meta-llama/Llama-2-70b-chat-hf": "TheBloke/Llama-2-70B-fp16"
    },
    "code_completion_default_model": "bigcode/starcoder",
    "code_completion_n_ctx": 2048,
    "code_chat_default_model": "meta-llama/Llama-2-70b-chat-hf",
    "telemetry_basic_dest": "https://staging.smallcloud.ai/v1/telemetry-basic",
    "telemetry_corrected_snippets_dest": "https://www.smallcloud.ai/v1/feedback",
    "running_models": ["bigcode/starcoder", "meta-llama/Llama-2-70b-chat-hf"]
}
"#;
