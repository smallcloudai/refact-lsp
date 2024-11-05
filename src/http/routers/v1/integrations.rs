use std::path::PathBuf;
use std::sync::Arc;
use axum::Extension;
use axum::http::{Response, StatusCode};
use tokio::sync::RwLock as ARwLock;
use hyper::Body;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use url::Url;
use crate::custom_error::ScratchError;
use crate::global_context::GlobalContext;
use crate::integrations::{integrations_paths, load_integration_schema_and_json, validate_integration_value, INTEGRATION_ICONS};

use std::fs;
use std::io::Read;
#[allow(deprecated)]
use base64::encode;
use reqwest::Client;
use tokio::fs as async_fs;
use tracing::info;


#[derive(Serialize, Deserialize)]
struct IntegrationItem {
    name: String,
    schema: Option<Value>,
    value: Value,
}

#[derive(Serialize)]
struct IntegrationIcon {
    name: String,
    value: String,
}


pub async fn get_image_base64(
    cache_dir: &PathBuf, 
    icon_name: &str, 
    icon_url: &str,
) -> Result<String, String> {
    let assets_path = cache_dir.join("assets/integrations");

    // Parse the URL to get the file extension
    let url = Url::parse(icon_url).map_err(|e| e.to_string())?;
    let extension = url
        .path_segments()
        .and_then(|segments| segments.last())
        .and_then(|name| name.split('.').last())
        .unwrap_or("png"); // Default to "png" if no extension is found

    let file_path = assets_path.join(format!("{}.{}", icon_name, extension));

    // Check if the file already exists
    if file_path.exists() {
        info!("Using image from cache: {}", file_path.display());
        let mut file = fs::File::open(&file_path).map_err(|e| e.to_string())?;
        let mut buffer = Vec::new();
        file.read_to_end(&mut buffer).map_err(|e| e.to_string())?;
        #[allow(deprecated)]
        let b64_image = encode(&buffer);
        let image_str = format!("data:{};base64,{}", extension, b64_image);
        return Ok(image_str);
    }

    // Create the cache directory if it doesn't exist
    async_fs::create_dir_all(&assets_path).await.map_err(|e| e.to_string())?;

    // Download the image
    info!("Downloading image from {}", icon_url);
    let client = Client::new();
    let response = client.get(icon_url).send().await.map_err(|e| e.to_string())?;
    let bytes = response.bytes().await.map_err(|e| e.to_string())?;

    // Save the image to the cache directory
    async_fs::write(&file_path, &bytes).await.map_err(|e| e.to_string())?;

    // Return the base64 string
    #[allow(deprecated)]
    let b64_image = encode(&bytes);
    let image_str = format!("data:{};base64,{}", extension, b64_image);
    Ok(image_str)
}

pub async fn handle_v1_integrations_icons(
    Extension(gcx): Extension<Arc<ARwLock<GlobalContext>>>,
    _: hyper::body::Bytes,
) -> axum::response::Result<Response<Body>, ScratchError> {
    let cache_dir = gcx.read().await.cache_dir.clone();
    
    let mut results = vec![];
    for (integr, icon_url) in INTEGRATION_ICONS {
        let image_base64 = get_image_base64(&cache_dir, &integr, icon_url).await.map_err(|e|{
            ScratchError::new(StatusCode::INTERNAL_SERVER_ERROR, format!("Failed to get image: {}", e))
        })?;
        results.push(IntegrationIcon {
            name: integr.to_string(),
            value: image_base64,
        });
    }

    let payload = serde_json::to_string_pretty(&json!(results)).expect("Failed to serialize results");
    Ok(Response::builder()
        .status(StatusCode::OK)
        .header("Content-Type", "application/json")
        .body(Body::from(payload))
        .unwrap())
}

pub async fn handle_v1_integrations(
    Extension(gcx): Extension<Arc<ARwLock<GlobalContext>>>,
    _: hyper::body::Bytes,
) -> axum::response::Result<Response<Body>, ScratchError> {

    let schemas_and_json_dict = load_integration_schema_and_json(gcx).await.map_err(|e|{
        ScratchError::new(StatusCode::INTERNAL_SERVER_ERROR, format!("Failed to load integrations: {}", e))
    })?;
    
    let mut items = vec![];
    for (name, (schema, value)) in schemas_and_json_dict {
        let item = IntegrationItem {
            name,
            schema: Some(schema),
            value,
        };
        
        items.push(item);
    }
    
    let payload = serde_json::to_string_pretty(&json!(items)).expect("Failed to serialize items");
    Ok(Response::builder()
        .status(StatusCode::OK)
        .header("Content-Type", "application/json")
        .body(Body::from(payload))
        .unwrap())
}


pub async fn handle_v1_integrations_save(
    Extension(gcx): Extension<Arc<ARwLock<GlobalContext>>>,
    body_bytes: hyper::body::Bytes,
) -> axum::response::Result<Response<Body>, ScratchError> {
    let post = serde_json::from_slice::<IntegrationItem>(&body_bytes)
        .map_err(|e| ScratchError::new(StatusCode::UNPROCESSABLE_ENTITY, format!("JSON problem: {}", e)))?;

    let yaml_value: serde_yaml::Value = serde_json::to_string(&post.value).map_err(|e|e.to_string())
        .and_then(|s|serde_yaml::from_str(&s).map_err(|e|e.to_string()))
        .map_err(|e| ScratchError::new(StatusCode::UNPROCESSABLE_ENTITY, format!("ERROR converting JSON to YAML: {}", e)))?;

    let yaml_value = validate_integration_value(&post.name, yaml_value)
        .map_err(|e| ScratchError::new(StatusCode::UNPROCESSABLE_ENTITY, format!("ERROR validating integration value: {}", e)))?;
    
    let integr_paths = integrations_paths(gcx.clone()).await;
    
    let path = integr_paths.get(&post.name)
        .ok_or(ScratchError::new(StatusCode::UNPROCESSABLE_ENTITY, format!("Integration {} not found", post.name)))?;

    let mut file = tokio::fs::OpenOptions::new()
        .write(true)
        .truncate(true)
        .open(path)
        .await
        .map_err(|e| ScratchError::new(StatusCode::INTERNAL_SERVER_ERROR, format!("Failed to open file: {}", e)))?;

    let yaml_string = serde_yaml::to_string(&yaml_value)
        .map_err(|e| ScratchError::new(StatusCode::INTERNAL_SERVER_ERROR, format!("Failed to convert YAML to string: {}", e)))?;

    tokio::io::AsyncWriteExt::write_all(&mut file, yaml_string.as_bytes()).await
        .map_err(|e| ScratchError::new(StatusCode::INTERNAL_SERVER_ERROR, format!("Failed to write to file: {}", e)))?;

    Ok(Response::builder()
       .status(StatusCode::OK)
       .header("Content-Type", "application/json")
       .body(Body::from(format!("Integration {} saved", post.name)))
       .unwrap())
}
