use std::time::Duration;
use reqwest;
use serde::Serialize;
use tokio::task::JoinHandle;
use tokio::time::sleep;

#[derive(Serialize)]
struct PayloadHF {
    pub inputs: String,
}

#[derive(Serialize)]
struct PayloadOpenAI {
    pub input: String,
    pub model: String,
}


pub fn get_embedding(
    endpoint_style: &String,
    model_name: &String,
    url: &String,
    text: String,
    api_key: &String,
) -> JoinHandle<Result<Vec<f32>, String>> {

    if endpoint_style == "hf" {
        return get_embedding_hf_style(
            text,
            url,
            api_key,
            3,
            Duration::from_secs(5),
        )
    } else if endpoint_style == "openai" {
        return get_embedding_openai_style(
            text,
            model_name,
            url,
            api_key,
            3,
            Duration::from_secs(5),
        )
    }
    else {
        panic!("Invalid endpoint style: {}", endpoint_style);
    }
}


fn get_embedding_hf_style(
    text: String,
    url: &String,
    api_key: &String,
    max_attempts: i32,
    delay: Duration,
) -> JoinHandle<Result<Vec<f32>, String>> {
    let client = reqwest::Client::new();
    let payload = PayloadHF { inputs: text };

    let url_clone = url.clone();
    let api_key_clone = api_key.clone();

    tokio::spawn(async move {
        let mut attempts = 0;

        while attempts < max_attempts {
            let maybe_response = client
                .post(&url_clone)
                .bearer_auth(api_key_clone.clone())
                .json(&payload)
                .send()
                .await;

            match maybe_response {
                Ok(response) => {
                    if response.status().is_success() {
                        match response.json::<Vec<f32>>().await {
                            Ok(embedding) => return Ok(embedding),
                            Err(err) => return Err(format!("Failed to parse the response: {:?}", err)),
                        }
                    } else if response.status().is_server_error() {
                        // Retry in case of 5xx server errors
                        attempts += 1;
                        sleep(delay).await;
                        continue;
                    } else {
                        return Err(format!("Failed to get a response: {:?}", response.status()));
                    }
                },
                Err(err) => return Err(format!("Failed to send a request: {:?}", err)),
            }
        }

        Err("Exceeded maximum attempts to reach the server".to_string())
    })
}


fn get_embedding_openai_style(
    text: String,
    model_name: &String,
    url: &String,
    api_key: &String,
    max_attempts: i32,
    delay: Duration,
) -> JoinHandle<Result<Vec<f32>, String>> {
    let client = reqwest::Client::new();
    let payload = PayloadOpenAI { input: text, model: model_name.clone() };

    let url_clone = url.clone();
    let api_key_clone = api_key.clone();

    tokio::spawn(async move {
        let mut attempts = 0;

        while attempts < max_attempts {
            let maybe_response = client
                .post(&url_clone)
                .bearer_auth(api_key_clone.clone())
                .json(&payload)
                .send()
                .await;

            match maybe_response {
                Ok(response) => {
                    if response.status().is_success() {
                        let response_json = response.json::<serde_json::Value>().await;

                        return match response_json {
                            Ok(json) => {
                                match &json["data"][0]["embedding"] {
                                    serde_json::Value::Array(embedding) => {
                                        let embedding_values: Result<Vec<f32>, _> =
                                            serde_json::from_value(serde_json::Value::Array(embedding.clone()));
                                        embedding_values.map_err(|err| {
                                            format!("Failed to parse the response: {:?}", err)
                                        })
                                    }
                                    _ => {
                                        Err("Response is missing 'data[0].embedding' field or it's not an array".to_string())
                                    }
                                }
                            }
                            Err(err) => {
                                Err(format!("Failed to parse the response: {:?}", err))
                            }
                        }
                    } else if response.status().is_server_error() {
                        // Retry in case of 5xx server errors
                        attempts += 1;
                        sleep(delay).await;
                        continue;
                    } else {
                        return Err(format!("Failed to get a response: {:?}", response.status()));
                    }
                }
                Err(err) => return Err(format!("Failed to send a request: {:?}", err)),
            }
        }

        Err("Exceeded maximum attempts to reach the server".to_string())
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