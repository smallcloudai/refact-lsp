use uuid::Uuid;

const FUNCTION_CALL_TAG_BEGIN: &str = "<functioncall>";
const FUNCTION_CALL_TAG_END: &str = "</functioncall>";

#[derive(Debug)]
pub struct DeltaDeltaChatStreamer {
    // This class helps chat implementations to stop at two-token phrases (at most) when streaming,
    // by delaying output by 1 token.
    // (the problem is the naive approach would have already sent the first token to the user, instead of stopping)
    pub buffer: String,
    pub finished: bool,
    pub stop_list: Vec<String>,
    pub role: String,
    pub is_function_call: bool,
}

impl DeltaDeltaChatStreamer {
    pub fn new() -> Self {
        Self {
            buffer: String::new(),
            finished: false,
            stop_list: Vec::new(),
            role: String::new(),
            is_function_call: false,
        }
    }

    pub fn response_n_choices(
        &mut self,
        choices: Vec<String>,
        stopped: Vec<bool>,
    ) -> Result<serde_json::Value, String> {
        assert!(!self.finished, "already finished");
        let mut json_choices = Vec::<serde_json::Value>::new();
        for (i, x) in choices.iter().enumerate() {
            let (content, delimiter) =
                split_buffer_min_prefix(&x, &self.stop_list, true);
            let finished = stopped[i] | self.stop_list.contains(&delimiter);

            let (content, _, postfix) =
                split_buffer(&content, &FUNCTION_CALL_TAG_BEGIN.to_string());
            let (function_call_text, _, _) =
                split_buffer(&postfix, &FUNCTION_CALL_TAG_END.to_string());

            let mut message_json = serde_json::json!({
                "role": self.role.clone(),
                "content": content,
            });
            if let Ok(function_call) = serde_json::from_str::<serde_json::Value>(&function_call_text.as_str()) {
                message_json["tool_calls"] = function_call
                    .as_array().unwrap().iter()
                    .map(|item| {
                        serde_json::json!({
                            "id": Uuid::new_v4().to_string(),
                            "function": item,
                            "type": "function"
                        })
                    }).collect();
            }

            json_choices.push(serde_json::json!({
                "index": i,
                "message": message_json,
                "finish_reason": (if finished { "stop" } else { "length" }).to_string(),
            }));
        }
        Ok(serde_json::json!(
            {
                "choices": json_choices,
            }
        ))
    }

    pub fn response_streaming(
        &mut self,
        delta: String,
        stopped: bool,
    ) -> Result<(serde_json::Value, bool), String>
    {
        self.buffer += delta.as_str();

        let (mut content, mut middle) =
            split_buffer_min_prefix(&self.buffer, &self.stop_list, delta.is_empty());
        let mut finished = stopped | self.stop_list.contains(&middle);

        let mut finish_reason: serde_json::Value =
            if !delta.is_empty() { serde_json::Value::Null }
            else { serde_json::Value::String("length".to_string()) };
        if finished {
            finish_reason = serde_json::Value::String("stop".to_string());
        }

        let mut function_call = serde_json::Value::Null;
        if !self.is_function_call {
            // trying to find that function call tag begins
            let delimiter: String;
            let postfix: String;
            (content, delimiter, postfix) =
                split_buffer(&content, &FUNCTION_CALL_TAG_BEGIN.to_string());
            if delimiter == FUNCTION_CALL_TAG_BEGIN {
                // immediately process function call parsing, skip previous content
                content = postfix.clone();
                self.is_function_call = true;
            } else {
                // we found partial function call tag, so we need to add it into buffer
                middle = delimiter + middle.as_str();
            }
        }
        if self.is_function_call {
            // in function call mode we don't send any content, but hold it in buffer
            let delimiter: String;
            (middle, delimiter, _) =
                split_buffer(&content, &FUNCTION_CALL_TAG_END.to_string());
            // if function call tag found we need now to parse and send it's content
            if delimiter == FUNCTION_CALL_TAG_END {
                function_call = serde_json::from_str::<serde_json::Value>(&middle.as_str())
                    .unwrap_or_else(|_| serde_json::Value::Null);
                function_call = function_call
                    .as_array().unwrap().iter()
                    .map(|item| {
                        serde_json::json!({
                            "id": Uuid::new_v4().to_string(),
                            "function": item,
                            "type": "function",
                            "index": 0,
                        })
                    }).collect();
                finish_reason = serde_json::Value::String("tool_calls".to_string());
                finished |= true;
            }
        }

        self.buffer = middle;
        self.finished = finished;

        let json_delta: serde_json::Value;
        if self.is_function_call {
            json_delta = serde_json::json!({
                "role": self.role.clone(),
                "tool_calls": function_call,
            });
        } else {
            json_delta = serde_json::json!({
                "role": self.role.clone(),
                "content": content.clone(),
            });
        }

        let ans = serde_json::json!({
            "choices": [
                {
                    "index": 0,
                    "delta": json_delta,
                    "finish_reason": finish_reason,
                }
            ]
        });

        Ok((ans, finished))
    }
}

fn normalize_buffer(
    buffer: &str,
) -> String {
    return buffer.to_string().replace("\r", "");
}

fn split_buffer(
    buffer: &str,
    delimiter: &String,
) -> (String, String, String) {
    // NOTE: this function tries to split given String into 3 parts:
    // 1. prefix of buffer where is no delimiter, or it's prefix
    // 2. delimiter or it's prefix
    // 3. remained buffer's suffix
    let text = normalize_buffer(buffer);
    let parts: Vec<&str> = text.split(delimiter).collect();
    if parts.len() > 1 {
        return (parts[0].to_string(), delimiter.clone(), parts[1..parts.len()].join(delimiter));
    }
    let mut prefix = parts[0].to_string();
    let mut middle = String::new();
    for idx in 1..delimiter.len() {
        if parts[0].ends_with(&delimiter[..idx]) {
            prefix = parts[0][..parts[0].len() - idx].to_string();
            middle = delimiter[..idx].to_string();
        }
    }
    return (prefix, middle, String::new());
}

fn split_buffer_min_prefix(
    buffer: &str,
    delimiters: &Vec<String>,
    full_match: bool,
) -> (String, String) {
    // NOTE: this function finds shortest prefix of buffer without delimiter symbols.
    // if you set full_match to true, it will return result only for full matches
    let text = normalize_buffer(buffer);
    let mut result = (text.clone(), String::new(), String::new());
    for delimiter in delimiters {
        let t = split_buffer(&text, delimiter);
        if full_match && t.1.as_str() != delimiter.as_str() {
            continue;
        }
        if t.0.len() < result.0.len() {
            result = t;
        }
    }
    return (result.0, result.1);
}
