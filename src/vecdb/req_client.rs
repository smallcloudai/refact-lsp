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

fn get_base_url() -> String {
    #[cfg(test)]
    {
        // When running tests, return the mockito server URL
        mockito::server_url()
    }

    #[cfg(not(test))]
    {
        // In a regular run, return the actual URL
        "https://api-inference.huggingface.co".to_string()
    }
}

pub fn get_embedding(
    text: String,
    model_name: &String,
    api_key: String,
) -> JoinHandle<Result<Vec<f32>, String>> {
    let url = format!("{}/models/{}", get_base_url(), model_name);
    let client = reqwest::Client::new();
    let payload = Payload { inputs: text };
    tokio::spawn(async move {
        let response = client.post(url)
            .bearer_auth(api_key.clone())
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

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_valid_request() {
        let _m = mockito::mock("POST", "/models/valid_model")
            .with_status(200)
            .with_body(r#"{"embedding": [1.0, 2.0, 3.0]}"#)
            .create();

        let text = "sample text".to_string();
        let model_name = "valid_model".to_string();
        let api_key = "valid_api_key".to_string();

        let result = get_embedding(text, &model_name, api_key).await.unwrap();

        assert!(result.is_ok());
        assert_eq!(result.unwrap(), vec![1.0, 2.0, 3.0]);
    }

    #[tokio::test]
    async fn test_invalid_api_key() {
        let _m = mockito::mock("POST", "/models/valid_model")
            .with_status(401)
            .create();

        let text = "sample text".to_string();
        let model_name = "valid_model".to_string();
        let api_key = "invalid_api_key".to_string();

        let result = get_embedding(text, &model_name, api_key).await.unwrap();

        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_concurrent_requests() {
        let mock = mockito::mock("POST", "/models/valid_model")
            .with_status(200)
            .with_body(r#"{"embedding": [1.0, 2.0, 3.0]}"#)
            .expect(10)  // Expect 10 calls
            .create();

        let handles: Vec<_> = (0..10).map(|_| {
            let text = "sample text".to_string();
            let model_name = "valid_model".to_string();
            let api_key = "valid_api_key".to_string();

            get_embedding(text, &model_name, api_key)
        }).collect();

        for handle in handles {
            let result = handle.await.unwrap();
            assert!(result.is_ok());
            assert_eq!(result.unwrap(), vec![1.0, 2.0, 3.0]);
        }

        mock.assert();
    }

    #[tokio::test]
    async fn test_empty_text_input() {
        let _m = mockito::mock("POST", "/models/valid_model")
            .with_status(200)
            .with_body(r#"{"embedding": []}"#)
            .create();

        let text = "".to_string();
        let model_name = "valid_model".to_string();
        let api_key = "valid_api_key".to_string();

        let result = get_embedding(text, &model_name, api_key).await.unwrap();
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), Vec::<f32>::new());
    }

    #[tokio::test]
    async fn test_invalid_model_name() {
        let _m = mockito::mock("POST", "/models/invalid_model")
            .with_status(404)
            .create();

        let text = "sample text".to_string();
        let model_name = "invalid_model".to_string();
        let api_key = "valid_api_key".to_string();

        let result = get_embedding(text, &model_name, api_key).await.unwrap();
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_network_failure() {
        let _m = mockito::mock("POST", "/models/valid_model")
            .with_status(500) // Internal Server Error to simulate server-side failure
            .create();

        let text = "sample text".to_string();
        let model_name = "valid_model".to_string();
        let api_key = "valid_api_key".to_string();

        let result = get_embedding(text, &model_name, api_key).await.unwrap();
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_different_embeddings() {
        let mock1 = mockito::mock("POST", "/models/model1")
            .with_status(200)
            .with_body(r#"{"embedding": [1.0, 2.0]}"#)
            .create();

        let mock2 = mockito::mock("POST", "/models/model2")
            .with_status(200)
            .with_body(r#"{"embedding": [3.0, 4.0]}"#)
            .create();

        let text = "sample text".to_string();
        let model_names = vec!["model1", "model2"];
        let api_key = "valid_api_key".to_string();

        for model_name in model_names {
            let result = get_embedding(text.clone(), &model_name.to_string(), api_key.clone()).await.unwrap();
            assert!(result.is_ok());
        }

        mock1.assert();
        mock2.assert();
    }
}