use std::sync::{Arc, RwLock as StdRwLock};
use tokio::sync::Mutex as AMutex;
use tokio::sync::RwLock as ARwLock;
use tokio::sync::mpsc;
use async_stream::stream;
use futures::StreamExt;
use hyper::{Body, Response, StatusCode};
use reqwest_eventsource::Event;
use serde_json::json;
use tracing::{error, info};

use crate::call_validation::SamplingParameters;
use crate::custom_error::ScratchError;
use crate::forward_to_hf_endpoint;
use crate::forward_to_openai_endpoint;
use crate::nicer_logs;
use crate::scratchpad_abstract::ScratchpadAbstract;
use crate::telemetry::telemetry_structs;
use crate::at_commands::at_commands::AtCommandsContext;


async fn _get_endpoint_and_stuff_from_model_name(
    gcx: Arc<ARwLock<crate::global_context::GlobalContext>>,
    caps: Arc<StdRwLock<crate::caps::CodeAssistantCaps>>,
    model_name: String,
) -> (String, String, String, String)
{
    let (
        custom_apikey,
        mut endpoint_style,
        custom_endpoint_style,
        mut endpoint_template,
        custom_endpoint_template,
        endpoint_chat_passthrough
    ) = {
        let caps_locked = caps.read().unwrap();
        let is_chat = caps_locked.code_chat_models.contains_key(&model_name);
        if is_chat {
            (
                caps_locked.chat_apikey.clone(),
                caps_locked.endpoint_style.clone(),      // abstract
                caps_locked.chat_endpoint_style.clone(), // chat-specific
                caps_locked.endpoint_template.clone(),   // abstract
                caps_locked.chat_endpoint.clone(),       // chat-specific
                caps_locked.endpoint_chat_passthrough.clone(),
            )
        } else {
            (
                caps_locked.completion_apikey.clone(),
                caps_locked.endpoint_style.clone(),             // abstract
                caps_locked.completion_endpoint_style.clone(),  // completion-specific
                caps_locked.endpoint_template.clone(),          // abstract
                caps_locked.completion_endpoint.clone(),        // completion-specific
                "".to_string(),
            )
        }
    };
    let api_key = {
        let gcx_locked = gcx.write().await;
        if custom_apikey.is_empty() {
            gcx_locked.cmdline.api_key.clone()
        } else if custom_apikey.starts_with("$") {
            let env_var_name = &custom_apikey[1..];
            match std::env::var(env_var_name) {
                Ok(env_value) => env_value,
                Err(e) => {
                    error!("Tried to read API key from env var {}, but failed: {}", env_var_name, e);
                    gcx_locked.cmdline.api_key.clone()
                }
            }
        } else {
            custom_apikey
        }
    };
    if !custom_endpoint_style.is_empty() {
        endpoint_style = custom_endpoint_style;
    }
    if !custom_endpoint_template.is_empty() {
        endpoint_template = custom_endpoint_template;
    }
    return (
        api_key,
        endpoint_template,
        endpoint_style,
        endpoint_chat_passthrough,
    )
}

pub async fn scratchpad_interaction_not_stream_json(
    ccx: Arc<AMutex<AtCommandsContext>>,
    scratchpad: &mut Box<dyn ScratchpadAbstract>,
    scope: String,
    prompt: &str,
    model_name: String,
    parameters: &SamplingParameters,  // includes n
    only_deterministic_messages: bool,
) -> Result<serde_json::Value, ScratchError> {
    let t2 = std::time::SystemTime::now();
    let gcx = ccx.lock().await.global_context.clone();
    let (client, caps, tele_storage, slowdown_arc) = {
        let gcx_locked = gcx.write().await;
        let caps = gcx_locked.caps.clone()
            .ok_or(ScratchError::new(StatusCode::INTERNAL_SERVER_ERROR, "No caps available".to_string()))?;
        (
            gcx_locked.http_client.clone(),
            caps,
            gcx_locked.telemetry.clone(),
            gcx_locked.http_client_slowdown.clone()
        )
    };
    let (
        bearer,
        endpoint_template,
        endpoint_style,
        endpoint_chat_passthrough,
    ) = _get_endpoint_and_stuff_from_model_name(gcx.clone(), caps.clone(), model_name.clone()).await;

    let mut save_url: String = String::new();
    let _ = slowdown_arc.acquire().await;
    let mut model_says = if only_deterministic_messages {
        Ok(serde_json::Value::Object(serde_json::Map::new()))
    } else if endpoint_style == "hf" {
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
            &parameters,  // includes n
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
    crate::global_context::look_for_piggyback_fields(gcx.clone(), &model_says).await;

    let scratchpad_result: Result<serde_json::Value, String>;
    if only_deterministic_messages {
        if let Ok(det_msgs) = scratchpad.response_spontaneous() {
            model_says["deterministic_messages"] = json!(det_msgs);
            model_says["choices"] = serde_json::Value::Array(vec![]);
        }
        scratchpad_result = Ok(model_says.clone());
    } else if let Some(hf_arr) = model_says.as_array() {
        let choices = hf_arr.iter().map(|x| x.get("generated_text").unwrap().as_str().unwrap().to_string()).collect::<Vec<_>>();
        let stopped = vec![false; choices.len()];
        scratchpad_result = scratchpad.response_n_choices(choices, stopped);

    } else if let Some(oai_choices) = model_says.get("choices") {
        let choice0 = oai_choices.as_array().unwrap().get(0).unwrap();
        if let Some(_msg) = choice0.get("message") {
            if let Ok(det_msgs) = scratchpad.response_spontaneous() {
                model_says["deterministic_messages"] = json!(det_msgs);
            }
            // new style openai response, used in passthrough
            scratchpad_result = Ok(model_says.clone());
        } else {
            // TODO: restore order using 'index'
            // for oai_choice in oai_choices.as_array().unwrap() {
            //     let index = oai_choice.get("index").unwrap().as_u64().unwrap() as usize;
            // }
            let choices = oai_choices.as_array().unwrap().iter().map(|x| x.get("text").unwrap().as_str().unwrap().to_string()).collect::<Vec<_>>();
            let stopped = oai_choices.as_array().unwrap().iter().map(|x| x.get("finish_reason").unwrap_or(&json!("")).as_str().unwrap().to_string().starts_with("stop")).collect::<Vec<_>>();
            scratchpad_result = scratchpad.response_n_choices(choices, stopped);
        }

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

    if let Err(problem) = scratchpad_result {
        return Err(ScratchError::new(StatusCode::INTERNAL_SERVER_ERROR,
            format!("scratchpad: {}", problem))
        );
    }
    return Ok(scratchpad_result.unwrap());
}

pub async fn scratchpad_interaction_not_stream(
    ccx: Arc<AMutex<AtCommandsContext>>,
    scratchpad: &mut Box<dyn ScratchpadAbstract>,
    scope: String,
    model_name: String,
    parameters: &mut SamplingParameters,
    only_deterministic_messages: bool,
) -> Result<Response<Body>, ScratchError> {
    let t1 = std::time::Instant::now();
    let prompt = scratchpad.prompt(
        ccx.clone(),
        parameters,
    ).await.map_err(|e|
        ScratchError::new(StatusCode::INTERNAL_SERVER_ERROR, format!("Prompt: {}", e))
    )?;
    info!("scratchpad_interaction_not_stream prompt {:?}", t1.elapsed());

    let t2 = std::time::SystemTime::now();
    let mut scratchpad_response_json = scratchpad_interaction_not_stream_json(
        ccx.clone(),
        scratchpad,
        scope,
        prompt.as_str(),
        model_name,
        parameters,
        only_deterministic_messages,
    ).await?;
    scratchpad_response_json["created"] = json!(t2.duration_since(std::time::UNIX_EPOCH).unwrap().as_millis() as f64 / 1000.0);

    try_insert_usage(&mut scratchpad_response_json);

    let txt = serde_json::to_string_pretty(&scratchpad_response_json).unwrap();
    // info!("handle_v1_code_completion return {}", txt);
    let response = Response::builder()
        .header("Content-Type", "application/json")
        .body(Body::from(txt))
        .unwrap();
    return Ok(response);
}

pub async fn scratchpad_interaction_stream(
    ccx: Arc<AMutex<AtCommandsContext>>,
    mut scratchpad: Box<dyn ScratchpadAbstract>,
    scope: String,
    mut model_name: String,
    parameters: SamplingParameters,
    only_deterministic_messages: bool,
) -> Result<Response<Body>, ScratchError> {
    let t1 = std::time::SystemTime::now();
    let evstream = stream! {
        let my_scratchpad: &mut Box<dyn ScratchpadAbstract> = &mut scratchpad;
        let mut my_parameters = parameters.clone();
        let my_ccx = ccx.clone();

        let gcx = ccx.lock().await.global_context.clone();
        let (client, caps, tele_storage, slowdown_arc) = {
            let gcx_locked = gcx.write().await;
            let caps = gcx_locked.caps.clone().unwrap();
            (
                gcx_locked.http_client.clone(),
                caps,
                gcx_locked.telemetry.clone(),
                gcx_locked.http_client_slowdown.clone()
            )
        };
        let (
            bearer,
            endpoint_template,
            endpoint_style,
            endpoint_chat_passthrough,
        ) = _get_endpoint_and_stuff_from_model_name(gcx.clone(), caps.clone(), model_name.clone()).await;

        let t0 = std::time::Instant::now();
        let mut prompt = String::new();
        {
            let subchat_tx: Arc<AMutex<mpsc::UnboundedSender<serde_json::Value>>> = my_ccx.lock().await.subchat_tx.clone();
            let subchat_rx: Arc<AMutex<mpsc::UnboundedReceiver<serde_json::Value>>> = my_ccx.lock().await.subchat_rx.clone();
            let mut prompt_future = Some(my_scratchpad.prompt(
                my_ccx.clone(),
                &mut my_parameters,
            ));
            // horrible loop that waits for prompt() future, and at the same time retranslates any streaming via my_ccx.subchat_rx/tx to the user
            // (without streaming the rx/tx is never processed, disposed with the ccx)
            loop {
                tokio::select! {
                    value = async {
                        subchat_rx.lock().await.recv().await
                    } => {
                        if let Some(value) = value {
                            let tmp = serde_json::to_string(&value).unwrap();
                            if tmp == "1337" {
                                break;  // the only way out of this loop
                            }
                            let value_str = format!("data: {}\n\n", tmp);
                            yield Result::<_, String>::Ok(value_str);
                        }
                    },
                    prompt_maybe = async {
                        if let Some(fut) = prompt_future.as_mut() {
                            fut.await
                        } else {
                            std::future::pending().await
                        }
                    } => {
                        if let Some(_fut) = prompt_future.take() {
                            prompt = match prompt_maybe {
                                Ok(x) => x,
                                Err(e) => {
                                    // XXX: tool errors go here, check again if this what we want
                                    tracing::warn!("prompt or tool use problem inside prompt: {}", e);
                                    let value_str = format!("data: {}\n\n", serde_json::to_string(&json!({"detail": e})).unwrap());
                                    yield Result::<_, String>::Ok(value_str);
                                    return;
                                }
                            };
                            let _ = subchat_tx.lock().await.send(serde_json::json!(1337));
                        }
                    }
                }
            }
        }
        info!("scratchpad_interaction_stream prompt {:?}", t0.elapsed());

        let mut save_url: String = String::new();
        let _ = slowdown_arc.acquire().await;
        loop {
            let value_maybe = my_scratchpad.response_spontaneous();
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
            if only_deterministic_messages {
                break;
            }
            // info!("prompt: {:?}", prompt);
            let event_source_maybe = if endpoint_style == "hf" {
                forward_to_hf_endpoint::forward_to_hf_style_endpoint_streaming(
                    &mut save_url,
                    bearer.clone(),
                    &model_name,
                    prompt.as_str(),
                    &client,
                    &endpoint_template,
                    &parameters,
                ).await
            } else {
                forward_to_openai_endpoint::forward_to_openai_style_endpoint_streaming(
                    &mut save_url,
                    bearer.clone(),
                    &model_name,
                    prompt.as_str(),
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
                        let json = serde_json::from_str::<serde_json::Value>(&message.data).unwrap();
                        crate::global_context::look_for_piggyback_fields(gcx.clone(), &json).await;
                        let value_maybe = _push_streaming_json_into_scratchpad(
                            my_scratchpad,
                            &json,
                            &mut model_name,
                            &mut finished,
                            &mut was_correct_output_even_if_error,
                        );
                        if let Ok(mut value) = value_maybe {
                            finished = try_insert_usage(&mut value);
                            value["created"] = json!(t1.duration_since(std::time::UNIX_EPOCH).unwrap().as_millis() as f64 / 1000.0);
                            let value_str = format!("data: {}\n\n", serde_json::to_string(&value).unwrap());
                            let last_60_chars: String = crate::nicer_logs::first_n_chars(&value_str, 60);
                            info!("yield: {:?}", last_60_chars);
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
                (value, _) = my_scratchpad.response_streaming("".to_string(), false, true).unwrap();
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

pub fn try_insert_usage(msg_value: &mut serde_json::Value) -> bool {
    let map = match msg_value.as_object() {
        Some(map) => map,
        None => {
            return false;
        }
    };
    let get_field_as_usize = |field: &str| -> Option<usize> {
        map.get(field).and_then(|v| v.as_u64()).map(|v| v as usize)
    };

    let metering_prompt_tokens_n = match get_field_as_usize("metering_prompt_tokens_n") {
        Some(value) => value,
        None => return false,
    };
    let metering_generated_tokens_n = match get_field_as_usize("metering_generated_tokens_n") {
        Some(value) => value,
        None => return false,
    };

    if let Some(map) = msg_value.as_object_mut() {
        ["pp1000t_prompt", "pp1000t_generated", "metering_prompt_tokens_n", "metering_generated_tokens_n"]
            .iter()
            .for_each(|&field| { map.remove(field); });

        let usage = json!({
            "prompt_tokens": metering_prompt_tokens_n,
            "completion_tokens": metering_generated_tokens_n,
            "total_tokens": metering_prompt_tokens_n + metering_generated_tokens_n
        });
        map.insert("usage".to_string(), usage);
        return true;
    }
    return false;
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
        if let Some(_delta) = choice0.get("delta") {
            // passthrough messages case
            // let _role = delta.get("role").unwrap_or(&json!("")).as_str().unwrap_or("").to_string();
            // let content = delta.get("content").unwrap_or(&json!("")).as_str().unwrap_or("").to_string();
            // (value, *finished) = scratch.response_streaming(content, stop_toks, stop_length)?;
            value = json.clone();
            *finished = !finish_reason.is_empty();
        } else if choices.as_array().map_or(true, |arr|arr.is_empty())  {
            value = json.clone();
            *finished = !finish_reason.is_empty();
        } else {
            // normal case
            let stop_toks = !finish_reason.is_empty() && finish_reason.starts_with("stop");
            let stop_length = !finish_reason.is_empty() && !finish_reason.starts_with("stop");
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
