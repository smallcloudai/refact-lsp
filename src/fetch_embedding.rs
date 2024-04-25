use std::sync::Arc;

use tokio::sync::Mutex as AMutex;
use tracing::error;

use crate::forward_to_openai_endpoint::get_embedding_openai_style;


pub async fn get_embedding(
    client: Arc<AMutex<reqwest::Client>>,
    endpoint_embeddings_style: &String,
    model_name: &String,
    endpoint_template: &String,
    text: Vec<String>,
    api_key: &String,
) -> Result<Vec<Vec<f32>>, String> {
    match endpoint_embeddings_style.to_lowercase().as_str() {
        "openai" => get_embedding_openai_style(client, text, endpoint_template, model_name, api_key).await,
        _ => {
            error!("Invalid endpoint_embeddings_style: {}", endpoint_embeddings_style);
            Err("Invalid endpoint_embeddings_style".to_string())
        }
    }
}
