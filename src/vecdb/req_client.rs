use reqwest;
use serde::{Deserialize, Serialize};
use tokio::task::JoinHandle;

#[derive(Serialize)]
struct Payload {
    inputs: String,
}

// Define a struct to deserialize the response
#[derive(Deserialize)]
struct ApiResponse {
    // Assuming the API returns an embedding in a field named 'embedding'
    embedding: Vec<f32>,
}


pub fn get_embedding(text: String, model_name: &String) -> JoinHandle<Result<Vec<f32>, String>> {
    let url = format!("https://api-inference.huggingface.co/models/{}", model_name);

    let client = reqwest::Client::new();

    let payload = Payload { inputs: text };

    tokio::spawn(async move {
        let response = client.post(url)
            .bearer_auth("your_api_token")
            .json(&payload)
            .send()
            .await
            .unwrap();
        if response.status().is_success() {
            let api_response: ApiResponse = response.json().await.unwrap();
            Ok(api_response.embedding)
        } else {
            Err(format!("Failed to get a response: {:?}", response.status()))
        }
    })
}

