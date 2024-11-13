use reqwest::header::{HeaderMap, CONTENT_TYPE, HeaderValue};

use reqwest_eventsource::EventSource;
use serde_json::{json, Value};
use tracing::info;
use crate::call_validation::SamplingParameters;


fn embed_messages_and_tools_from_prompt(
    data: &mut Value, prompt: &str
) {
    assert!(prompt.starts_with("PASSTHROUGH "));
    let messages_str = &prompt[12..];
    let big_json: Value = serde_json::from_str(&messages_str).unwrap();
    
    if let Some(messages) = big_json["messages"].as_array() {
        data["messages"] = Value::Array(
            messages.iter().filter(|msg| msg["role"] != "system").cloned().collect()
        );
        let system_string = messages.iter()
            .filter(|msg| msg["role"] == "system")
            .map(|msg| msg["content"].as_str().unwrap_or(""))
            .collect::<Vec<_>>()
            .join("\n");

        if !system_string.is_empty() {
            data["system"] = Value::String(system_string);
        }

    }
    
    if let Some(tools) = big_json.get("tools") {
        data["tools"] = tools.clone();
    }
}

fn make_headers(bearer: &str) -> Result<HeaderMap, String> {
    let mut headers = HeaderMap::new();
    headers.insert(CONTENT_TYPE, HeaderValue::from_str("application/json").unwrap());
    // see https://docs.anthropic.com/en/api/versioning
    headers.insert("anthropic-version", HeaderValue::from_static("2023-06-01"));

    if !bearer.is_empty() {
        headers.insert("x-api-key", HeaderValue::from_str(bearer)
            .map_err(|e| format!("Failed to insert header: {}", e))?);
    }
    Ok(headers)
}

pub async fn forward_to_anthropic_endpoint(
    save_url: &mut String,
    bearer: String,
    model_name: &str,
    prompt: &str,
    client: &reqwest::Client,
    endpoint_chat_passthrough: &String,
    sampling_parameters: &SamplingParameters,
) -> Result<Value, String> {
    *save_url = endpoint_chat_passthrough.clone();
    let headers = make_headers(bearer.as_str())?;
    
    let mut data = json!({
        "model": model_name,
        "stream": false,
        "temperature": sampling_parameters.temperature,
        "max_tokens": sampling_parameters.max_new_tokens,
    });

    embed_messages_and_tools_from_prompt(&mut data, prompt);
    
    let req = client.post(save_url.as_str())
        .headers(headers)
        .body(data.to_string())
        .send()
        .await;
    let resp = req.map_err(|e| format!("{}", e))?;
    let status_code = resp.status().as_u16();
    let response_txt = resp.text().await.map_err(|e|
        format!("reading from socket {}: {}", save_url, e)
    )?;

    if status_code != 200 && status_code != 400 {
        return Err(format!("{} status={} text {}", save_url, status_code, response_txt));
    }
    if status_code != 200 {
        info!("forward_to_openai_style_endpoint: {} {}\n{}", save_url, status_code, response_txt);
    }
    let parsed_json: Value = match serde_json::from_str(&response_txt) {
        Ok(json) => json,
        Err(e) => return Err(format!("Failed to parse JSON response: {}\n{}", e, response_txt)),
    };
    Ok(parsed_json)
}

pub async fn forward_to_anthropic_endpoint_streaming(
    save_url: &mut String,
    bearer: String,
    model_name: &str,
    prompt: &str,
    client: &reqwest::Client,
    endpoint_chat_passthrough: &String,
    sampling_parameters: &SamplingParameters,
) -> Result<EventSource, String> {
    *save_url = endpoint_chat_passthrough.clone();
    let headers = make_headers(bearer.as_str())?;
    
    let mut data = json!({
        "model": model_name,
        "stream": true,
        "temperature": sampling_parameters.temperature,
        "max_tokens": sampling_parameters.max_new_tokens,
    });

    embed_messages_and_tools_from_prompt(&mut data, prompt);
    
    let builder = client.post(save_url.as_str())
        .headers(headers)
        .body(data.to_string());
    let event_source: EventSource = EventSource::new(builder).map_err(|e|
        format!("can't stream from {}: {}", save_url, e)
    )?;
    
    Ok(event_source)
}
