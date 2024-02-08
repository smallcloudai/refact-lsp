use tracing::{error, info};
use std::sync::{Arc, Mutex};
use tokio::sync::RwLock as ARwLock;
use reqwest_eventsource::Event;
use serde_json::json;
use futures::StreamExt;
use async_stream::stream;
use hyper::{Body, Response, StatusCode};

use crate::nicer_logs;
use crate::scratchpad_abstract::ScratchpadAbstract;
use crate::forward_to_hf_endpoint;
use crate::forward_to_openai_endpoint;
use crate::custom_error::ScratchError;
use crate::call_validation::SamplingParameters;
use crate::telemetry::telemetry_structs;
use crate::global_context::GlobalContext;
use crate::global_context::Slowdown;


struct SlowdownScoped {
    slowdown_arc: Arc<Mutex<Slowdown>>,
}

impl SlowdownScoped {
    fn new(slowdown_arc: Arc<Mutex<Slowdown>>) -> Self {
        let mut slowdown_locked = slowdown_arc.lock().unwrap();
        slowdown_locked.requests_in_flight += 1;
        // info!("slowdown_scoped new");
        Self {
            slowdown_arc: slowdown_arc.clone()
        }
    }

    async fn be_nice_slow_down(&mut self) {
        loop {
            {
                let slowdown_locked = self.slowdown_arc.lock().unwrap();
                if slowdown_locked.requests_in_flight <= 1 {   // one is this object itself
                    break;
                }
            }
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        }
    }
}

impl Drop for SlowdownScoped {
    fn drop(&mut self) {
        let mut slowdown_locked = self.slowdown_arc.lock().unwrap();
        slowdown_locked.requests_in_flight -= 1;
        // info!("slowdown_scoped /drop\n");
    }
}


pub async fn scratchpad_interaction_not_stream(
    global_context: Arc<ARwLock<GlobalContext>>,
    mut scratchpad: Box<dyn ScratchpadAbstract>,
    scope: String,
    prompt: &str,
    model_name: String,
    client: reqwest::Client,
    bearer: String,
    parameters: &SamplingParameters,
) -> Result<Response<Body>, ScratchError> {
    let t2 = std::time::SystemTime::now();
    let (endpoint_style, endpoint_template, endpoint_chat_passthrough, tele_storage, slowdown_arc) = {
        let cx = global_context.write().await;
        let caps = cx.caps.clone().unwrap();
        let caps_locked = caps.read().unwrap();
        (caps_locked.endpoint_style.clone(), caps_locked.endpoint_template.clone(), caps_locked.endpoint_chat_passthrough.clone(), cx.telemetry.clone(), cx.http_client_slowdown.clone())
    };
    let mut save_url: String = String::new();
    let mut slowdown_scoped = SlowdownScoped::new(slowdown_arc.clone());
    slowdown_scoped.be_nice_slow_down().await;
    let model_says = if endpoint_style == "hf" {
        forward_to_hf_endpoint::forward_to_hf_style_endpoint(
            &mut save_url,
            bearer.clone(),
            &model_name,
            &prompt,
            &client,
            &endpoint_template,
            &parameters,
        ).await
    } else {
        forward_to_openai_endpoint::forward_to_openai_style_endpoint(
            &mut save_url,
            bearer.clone(),
            &model_name,
            &prompt,
            &client,
            &endpoint_template,
            &endpoint_chat_passthrough,
            &parameters,
        ).await
    }.map_err(|e| {
        tele_storage.write().unwrap().tele_net.push(telemetry_structs::TelemetryNetwork::new(
                save_url.clone(),
                scope.clone(),
                false,
                e.to_string(),
            ));
        ScratchError::new_but_skip_telemetry(StatusCode::INTERNAL_SERVER_ERROR, format!("forward_to_endpoint: {}", e))
    })?;
    tele_storage.write().unwrap().tele_net.push(telemetry_structs::TelemetryNetwork::new(
        save_url.clone(),
        scope.clone(),
        true,
        "".to_string(),
    ));
    info!("forward to endpoint {:.2}ms, url was {}", t2.elapsed().unwrap().as_millis() as f64, save_url);
    crate::global_context::look_for_piggyback_fields(global_context.clone(), &model_says).await;

    let scratchpad_result: Result<serde_json::Value, String>;
    if let Some(hf_arr) = model_says.as_array() {
        let choices = hf_arr.iter()
            .map(|x| {
                x.get("generated_text").unwrap().as_str().unwrap().to_string()
            }).collect::<Vec<_>>();
        let stopped = vec![false; choices.len()];
        scratchpad_result = scratchpad.response_n_choices(choices, stopped);

    } else if let Some(oai_choices) = model_says.get("choices") {
        let choices = oai_choices.as_array().unwrap().iter()
            .map(|x| {
                x.get("text").unwrap().as_str().unwrap().to_string()
            }).collect::<Vec<_>>();
        let stopped = oai_choices.as_array().unwrap().iter()
            .map(|x| {
                x.get("finish_reason").unwrap_or(&json!("")).as_str().unwrap().to_string().starts_with("stop")
            }).collect::<Vec<_>>();
        scratchpad_result = scratchpad.response_n_choices(choices, stopped);

    } else if let Some(err) = model_says.get("error") {
        return Err(ScratchError::new(StatusCode::INTERNAL_SERVER_ERROR,
            format!("{}", err)
        ));

    } else if let Some(msg) = model_says.get("human_readable_message") {
        return Err(ScratchError::new(StatusCode::INTERNAL_SERVER_ERROR,
            format!("{}", msg)
        ));

    } else if let Some(msg) = model_says.get("detail") {
        return Err(ScratchError::new(StatusCode::INTERNAL_SERVER_ERROR,
            format!("{}", msg)
        ));

    } else {
        return Err(ScratchError::new(StatusCode::INTERNAL_SERVER_ERROR,
            format!("unrecognized response (1): {:?}", model_says))
        );
    }

    if let Err(scratchpad_result_str) = scratchpad_result {
        return Err(ScratchError::new(StatusCode::INTERNAL_SERVER_ERROR,
            format!("scratchpad: {}", scratchpad_result_str))
        );
    }
    let mut scratchpad_response_json = scratchpad_result.unwrap();
    scratchpad_response_json["created"] = json!(t2.duration_since(std::time::UNIX_EPOCH).unwrap().as_millis() as f64 / 1000.0);

    let txt = serde_json::to_string_pretty(&scratchpad_response_json).unwrap();
    // info!("handle_v1_code_completion return {}", txt);
    let response = Response::builder()
        .header("Content-Type", "application/json")
        .body(Body::from(txt))
        .unwrap();
    return Ok(response);
}

pub async fn scratchpad_interaction_stream(
    global_context: Arc<ARwLock<GlobalContext>>,
    mut scratchpad: Box<dyn ScratchpadAbstract>,
    scope: String,
    prompt: String,
    mut model_name: String,
    client: reqwest::Client,
    bearer: String,
    parameters: SamplingParameters,
) -> Result<Response<Body>, ScratchError> {
    let t1 = std::time::SystemTime::now();
    let evstream = stream! {
        let scratch: &mut Box<dyn ScratchpadAbstract> = &mut scratchpad;
        let (endpoint_style, endpoint_template, endpoint_chat_passthrough, tele_storage, slowdown_arc) = {
            let cx = global_context.write().await;
            let caps = cx.caps.clone().unwrap();
            let caps_locked = caps.read().unwrap();
            (caps_locked.endpoint_style.clone(), caps_locked.endpoint_template.clone(), caps_locked.endpoint_chat_passthrough.clone(), cx.telemetry.clone(), cx.http_client_slowdown.clone())
        };
        let mut save_url: String = String::new();
        let mut slowdown_scoped = SlowdownScoped::new(slowdown_arc.clone());
        slowdown_scoped.be_nice_slow_down().await;
        loop {
            {
                let value_maybe = scratch.response_spontaneous();
                if let Ok(value) = value_maybe {
                    for el in value {
                        let value_str = format!("data: {}\n\n", serde_json::to_string(&el).unwrap());
                        info!("yield: {:?}", nicer_logs::first_n_chars(&value_str, 40));
                        yield Result::<_, String>::Ok(value_str);
                    }
                } else {
                    let err_str = value_maybe.unwrap_err();
                    error!("response_spontaneous error: {}", err_str);
                    let value_str = format!("data: {}\n\n", serde_json::to_string(&json!({"detail": err_str})).unwrap());
                    yield Result::<_, String>::Ok(value_str);
                }
            }

            let event_source_maybe = if endpoint_style == "hf" {
                forward_to_hf_endpoint::forward_to_hf_style_endpoint_streaming(
                    &mut save_url,
                    bearer.clone(),
                    &model_name,
                    &prompt,
                    &client,
                    &endpoint_template,
                    &parameters,
                ).await
            } else {
                forward_to_openai_endpoint::forward_to_openai_style_endpoint_streaming(
                    &mut save_url,
                    bearer.clone(),
                    &model_name,
                    &prompt,
                    &client,
                    &endpoint_template,
                    &endpoint_chat_passthrough,
                    &parameters,
                ).await
            };
            let mut event_source = match event_source_maybe {
                Ok(event_source) => event_source,
                Err(e) => {
                    let e_str = format!("forward_to_endpoint: {:?}", e);
                    tele_storage.write().unwrap().tele_net.push(telemetry_structs::TelemetryNetwork::new(
                        save_url.clone(),
                        scope.clone(),
                        false,
                        e_str.to_string(),
                    ));
                    error!(e_str);
                    let value_str = serde_json::to_string(&json!({"detail": e_str})).unwrap();
                    yield Result::<_, String>::Ok(value_str);
                    break;
                }
            };
            let mut finished: bool = false;
            let mut problem_reported = false;
            let mut was_correct_output_even_if_error = false;
            // let mut test_countdown = 250;
            while let Some(event) = event_source.next().await {
                match event {
                    Ok(Event::Open) => {},
                    Ok(Event::Message(message)) => {
                        // info!("Message: {:#?}", message);
                        if message.data.starts_with("[DONE]") {
                            break;
                        }
                        // test_countdown -= 1;
                        // if test_countdown == 0 {
                        //     error!("test_countdown!");
                        //     let value_str = format!("data: {}\n\n", serde_json::to_string(&json!({"detail": "test_countdown happened"})).unwrap());
                        //     yield Result::<_, String>::Ok(value_str);
                        //     problem_reported = true;
                        //     break;
                        // }
                        let json = serde_json::from_str::<serde_json::Value>(&message.data).unwrap();
                        crate::global_context::look_for_piggyback_fields(global_context.clone(), &json).await;
                        let value_maybe = _push_streaming_json_into_scratchpad(
                            scratch,
                            &json,
                            &mut model_name,
                            &mut finished,
                            &mut was_correct_output_even_if_error,
                        );
                        if let Ok(mut value) = value_maybe {
                            value["created"] = json!(t1.duration_since(std::time::UNIX_EPOCH).unwrap().as_millis() as f64 / 1000.0);
                            let value_str = format!("data: {}\n\n", serde_json::to_string(&value).unwrap());
                            info!("yield: {:?}", value_str);
                            yield Result::<_, String>::Ok(value_str);
                        } else {
                            let err_str = value_maybe.unwrap_err();
                            error!("unexpected error: {}", err_str);
                            let value_str = format!("data: {}\n\n", serde_json::to_string(&json!({"detail": err_str})).unwrap());
                            yield Result::<_, String>::Ok(value_str);
                            // TODO: send telemetry
                            problem_reported = true;
                            break;
                        }
                        if finished {
                            break;
                        }
                    },
                    Err(err) => {
                        if was_correct_output_even_if_error {
                            // "restream error: Stream ended"
                            break;
                        }
                        error!("restream error: {}\n{:?}", err, err);
                        let problem_str = format!("restream error: {}", err);
                        {
                            tele_storage.write().unwrap().tele_net.push(telemetry_structs::TelemetryNetwork::new(
                                save_url.clone(),
                                scope.clone(),
                                false,
                                problem_str.clone(),
                            ));
                        }
                        yield Result::<_, String>::Ok(serde_json::to_string(&json!({"detail": problem_str})).unwrap());
                        problem_reported = true;
                        event_source.close();
                        break;
                    },
                }
            }
            if problem_reported {
                return;
            } else if !finished {
                let mut value: serde_json::Value;
                (value, _) = scratch.response_streaming("".to_string(), false, true).unwrap();
                value["created"] = json!(t1.duration_since(std::time::UNIX_EPOCH).unwrap().as_millis() as f64 / 1000.0);
                value["model"] = json!(model_name.clone());
                let value_str = format!("data: {}\n\n", serde_json::to_string(&value).unwrap());
                info!("yield final: {:?}", value_str);
                yield Result::<_, String>::Ok(value_str);
            }
            break;
        }
        info!("yield: [DONE]");
        yield Result::<_, String>::Ok("data: [DONE]\n\n".to_string());
        tele_storage.write().unwrap().tele_net.push(telemetry_structs::TelemetryNetwork::new(
            save_url.clone(),
            scope.clone(),
            true,
            "".to_string(),
        ));
    };

    let response = Response::builder()
        .header("Content-Type", "application/json")
        .body(Body::wrap_stream(evstream))
        .unwrap();
    return Ok(response);
}

fn _push_streaming_json_into_scratchpad(
    scratch: &mut Box<dyn ScratchpadAbstract>,
    json: &serde_json::Value,
    model_name: &mut String,
    finished: &mut bool,
    was_correct_output_even_if_error: &mut bool,
) -> Result<serde_json::Value, String> {
    if let Some(token) = json.get("token") { // hf style produces this
        let text = token.get("text").unwrap_or(&json!("")).as_str().unwrap_or("").to_string();
        let mut value: serde_json::Value;
        (value, *finished) = scratch.response_streaming(text, false, false)?;
        value["model"] = json!(model_name.clone());
        *was_correct_output_even_if_error |= json.get("generated_text").is_some();
        Ok(value)
    } else if let Some(choices) = json.get("choices") { // openai style
        let choice0 = &choices[0];
        let mut value: serde_json::Value;
        let finish_reason = choice0.get("finish_reason").unwrap_or(&json!("")).as_str().unwrap_or("").to_string();
        let stop_toks = !finish_reason.is_empty() && finish_reason.starts_with("stop");
        let stop_length = !finish_reason.is_empty() && !finish_reason.starts_with("stop");
        if let Some(delta) = choice0.get("delta") {
            // passthrough messages case
            let _role = delta.get("role").unwrap_or(&json!("")).as_str().unwrap_or("").to_string();
            let content = delta.get("content").unwrap_or(&json!("")).as_str().unwrap_or("").to_string();
            (value, *finished) = scratch.response_streaming(content, stop_toks, stop_length)?;
        } else {
            // normal case
            let text = choice0.get("text").unwrap_or(&json!("")).as_str().unwrap_or("").to_string();
            (value, *finished) = scratch.response_streaming(text, stop_toks, stop_length)?;
        }
        if let Some(model_value) = choice0.get("model") {
            model_name.clone_from(&model_value.as_str().unwrap_or("").to_string());
        }
        value["model"] = json!(model_name.clone());
        Ok(value)
    } else if let Some(err) = json.get("error") {
        Err(format!("{}", err))
    } else if let Some(msg) = json.get("human_readable_message") {
        Err(format!("{}", msg))
    } else if let Some(msg) = json.get("detail") {
        Err(format!("{}", msg))
    } else {
        Err(format!("unrecognized response (2): {:?}", json))
    }
}

pub async fn cached_not_stream(
    cached_json_value: &serde_json::Value,
) -> Result<Response<Body>, ScratchError> {
    let txt = serde_json::to_string_pretty(&cached_json_value).unwrap();
    let response = Response::builder()
       .header("Content-Type", "application/json")
      .body(Body::from(txt))
      .unwrap();
    return Ok(response);
}

pub async fn cached_stream(
    cached_json_value: &serde_json::Value,
) -> Result<Response<Body>, ScratchError> {
    info!("cached_stream");
    let txt = serde_json::to_string(&cached_json_value).unwrap();
    let evstream = stream! {
        yield Result::<_, String>::Ok(format!("data: {}\n\n", txt));
        yield Result::<_, String>::Ok("data: [DONE]\n\n".to_string());
    };
    let response = Response::builder()
       .header("Content-Type", "application/json")
       .body(Body::wrap_stream(evstream))
       .unwrap();
    return Ok(response);
}
