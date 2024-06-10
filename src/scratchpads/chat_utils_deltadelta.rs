use log::info;
use uuid::Uuid;

#[derive(Debug)]
pub struct DeltaDeltaChatStreamer {
    // This class helps chat implementations to stop at two-token phrases (at most) when streaming,
    // by delaying output by 1 token.
    // (the problem is the naive approach would have already sent the first token to the user, instead of stopping)
    pub buffer: String,
    pub finished: bool,
    pub stop_list: Vec<String>,
    pub role: String,
}

impl DeltaDeltaChatStreamer {
    pub fn new() -> Self {
        Self {
            buffer: String::new(),
            finished: false,
            stop_list: Vec::new(),
            role: String::new(),
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
            let (content, functioncall) = parse_functioncall(&content);
            let mut message_json = serde_json::json!({
                "role": self.role.clone(),
                "content": content,
            });
            if functioncall != serde_json::Value::Null {
                message_json["tool_calls"] = functioncall
                    .as_array()
                    .unwrap()
                    .iter()
                    .enumerate()
                    .map(|(index, item)| {
                        serde_json::json!({
                            "id": Uuid::new_v4().to_string(),
                            "function": item,
                            "type": "function"
                        })
                    })
                    .collect();
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

        let (content, middle) =
            split_buffer_min_prefix(&self.buffer, &self.stop_list, delta.is_empty());
        let finished = stopped | self.stop_list.contains(&middle);
        let mut finish_reason: serde_json::Value =
            if !delta.is_empty() { serde_json::Value::Null }
            else { serde_json::Value::String("length".to_string()) };
        if finished {
            finish_reason = serde_json::Value::String("stop".to_string());
        }
        self.buffer = middle;
        self.finished = finished;

        let ans = serde_json::json!({
            "choices": [
                {
                    "index": 0,
                    "delta": {
                        "role": self.role.clone(),
                        "content": content.clone(),
                    },
                    "finish_reason": finish_reason,
                }
            ]
        });

        Ok((ans, finished))
    }
}

fn split_buffer(
    buffer: &str,
    delimiter: &String,
) -> (String, String, String) {
    // NOTE: this function tries to split given String into 3 parts:
    // 1. prefix of buffer where is no delimiter, or it's prefix
    // 2. delimiter or it's prefix
    // 3. remained buffer's suffix
    let text = buffer.to_string().replace("\r", "");
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
    let mut result = (String::from(buffer), String::new(), String::new());
    for delimiter in delimiters {
        let t = split_buffer(buffer, delimiter);
        if full_match && t.1.as_str() != delimiter.as_str() {
            continue;
        }
        if t.0.len() < result.0.len() {
            result = t;
        }
    }
    return (result.0, result.1);
}

fn parse_functioncall(
    content: &str,
) -> (&str, serde_json::Value) {
    let start_tag = "<functioncall>";
    let end_tag = "</functioncall>";

    if let Some(start_idx) = content.find(start_tag) {
        if let Some(end_idx) = content.find(end_tag) {
            let context = &content[..start_idx];
            let functioncall = &content[start_idx + start_tag.len()..end_idx];
            if let Ok(parsed_json) = serde_json::from_str(functioncall) {
                return (context, parsed_json);
            } else {
                return (context, serde_json::Value::Null);
            }
        }
    }

    return (content, serde_json::Value::Null);
}
