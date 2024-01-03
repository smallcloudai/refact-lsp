use std::time::Duration;
use reqwest::header::AUTHORIZATION;
use reqwest::header::CONTENT_TYPE;
use reqwest::header::HeaderMap;
use reqwest::header::HeaderValue;
use reqwest_eventsource::EventSource;
use serde::Serialize;
use serde_json::json;
use tokio::task::JoinHandle;
use tokio::time::sleep;
use crate::call_validation;
use crate::call_validation::SamplingParameters;


pub async fn forward_to_openai_style_endpoint(
    save_url: &mut String,
    bearer: String,
    model_name: &str,
    prompt: &str,
    client: &reqwest::Client,
    endpoint_template: &String,
    endpoint_chat_passthrough: &String,
    sampling_parameters: &SamplingParameters,
) -> Result<serde_json::Value, String> {
    let is_passthrough = prompt.starts_with("PASSTHROUGH ");
    let url = if !is_passthrough { endpoint_template.replace("$MODEL", model_name) } else { endpoint_chat_passthrough.clone() };
    save_url.clone_from(&&url);
    let mut headers = HeaderMap::new();
    headers.insert(CONTENT_TYPE, HeaderValue::from_str("application/json").unwrap());
    if !bearer.is_empty() {
        headers.insert(AUTHORIZATION, HeaderValue::from_str(format!("Bearer {}", bearer).as_str()).unwrap());
    }
    let mut data = json!({
        "model": model_name,
        "echo": false,
        "stream": false,
        "temperature": sampling_parameters.temperature,
        "max_tokens": sampling_parameters.max_new_tokens,
    });
    if is_passthrough {
        _passthrough_messages_to_json(&mut data, prompt);
    } else {
        data["prompt"] = serde_json::Value::String(prompt.to_string());
    }
    // When cancelling requests, coroutine ususally gets aborted here on the following line.
    let req = client.post(&url)
       .headers(headers)
       .body(data.to_string())
       .send()
       .await;
    let resp = req.map_err(|e| format!("{}", e))?;
    let status_code = resp.status().as_u16();
    let response_txt = resp.text().await.map_err(|e|
        format!("reading from socket {}: {}", url, e)
    )?;
    // info!("forward_to_openai_style_endpoint: {} {}\n{}", url, status_code, response_txt);
    if status_code != 200 {
        return Err(format!("{} status={} text {}", url, status_code, response_txt));
    }
    Ok(serde_json::from_str(&response_txt).unwrap())
}

pub async fn forward_to_openai_style_endpoint_streaming(
    save_url: &mut String,
    bearer: String,
    model_name: &str,
    prompt: &str,
    client: &reqwest::Client,
    endpoint_template: &String,
    endpoint_chat_passthrough: &String,
    sampling_parameters: &SamplingParameters,
) -> Result<EventSource, String> {
    let is_passthrough = prompt.starts_with("PASSTHROUGH ");
    let url = if !is_passthrough { endpoint_template.replace("$MODEL", model_name) } else { endpoint_chat_passthrough.clone() };
    save_url.clone_from(&&url);
    let mut headers = HeaderMap::new();
    headers.insert(CONTENT_TYPE, HeaderValue::from_str("application/json").unwrap());
    if !bearer.is_empty() {
        headers.insert(AUTHORIZATION, HeaderValue::from_str(format!("Bearer {}", bearer).as_str()).unwrap());
    }
    let mut data = json!({
        "model": model_name,
        "stream": true,
        "temperature": sampling_parameters.temperature,
        "max_tokens": sampling_parameters.max_new_tokens,
    });
    if is_passthrough {
        _passthrough_messages_to_json(&mut data, prompt);
    } else {
        data["prompt"] = serde_json::Value::String(prompt.to_string());
    }
    let builder = client.post(&url)
       .headers(headers)
       .body(data.to_string());
    let event_source: EventSource = EventSource::new(builder).map_err(|e|
        format!("can't stream from {}: {}", url, e)
    )?;
    Ok(event_source)
}

fn _passthrough_messages_to_json(
    data: &mut serde_json::Value,
    prompt: &str,
) {
    assert!(prompt.starts_with("PASSTHROUGH "));
    let messages_str = &prompt[12..];
    let messages: Vec<call_validation::ChatMessage> = serde_json::from_str(&messages_str).unwrap();
    data["messages"] = serde_json::json!(messages);
}


#[derive(Serialize)]
struct EmbeddingsPayloadOpenAI {
    pub input: String,
    pub model: String,
}


pub async fn get_embedding_openai_style(
    text: String,
    endpoint_template: &String,
    model_name: &String,
    api_key: &String,
) -> Result<Vec<f32>, String> {
    let client = reqwest::Client::new();
    let payload = EmbeddingsPayloadOpenAI {
        input: text,
        model: model_name.clone(),
    };
    let url = endpoint_template.clone();
    let api_key_clone = api_key.clone();

    let join_handle = tokio::spawn(async move {
        let maybe_response = client
            .post(&url)
            .bearer_auth(api_key_clone.clone())
            .json(&payload)
            .send()
            .await;

        return match maybe_response {
            Ok(response) => {
                if response.status().is_success() {
                    let response_json = response.json::<serde_json::Value>().await;

                    match response_json {
                        Ok(json) => match &json["data"][0]["embedding"] {
                            serde_json::Value::Array(embedding) => {
                                let embedding_values: Result<Vec<f32>, _> =
                                    serde_json::from_value(serde_json::Value::Array(embedding.clone()));
                                embedding_values.map_err(|err| {
                                    format!("Failed to parse the response: {:?}", err)
                                })
                            }
                            _ => Err("Response is missing 'data[0].embedding' field or it's not an array".to_string()),
                        },
                        Err(err) => Err(format!("Failed to parse the response: {:?}", err)),
                    }
                } else {
                    Err(format!("Failed to get a response: {:?}", response.status()))
                }
            }
            Err(err) => Err(format!("Failed to send a request: {:?}", err)),
        }
    });

    join_handle.await.unwrap_or_else(|_| Err("Task join error".to_string()))
}
