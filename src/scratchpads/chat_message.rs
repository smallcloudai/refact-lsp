use serde::{Deserialize, Deserializer, Serialize};
use std::sync::{Arc, RwLock, RwLockWriteGuard};
use tokenizers::Tokenizer;

use crate::scratchpads::scratchpad_utils::{calculate_image_tokens_openai, parse_image_b64_from_image_url};


#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Default)]
pub struct MultimodalElementText {
    #[serde(rename = "type")]
    pub content_type: String,
    pub text: String,
}

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Default)]
pub struct MultimodalElementImage {
    #[serde(rename = "type")]
    pub content_type: String,
    pub image_url: MultimodalElementImageImageURL,
}

impl MultimodalElementImage {
    pub fn new(url: String) -> Self {
        let image_url = MultimodalElementImageImageURL {
            url: url.clone(),
            detail: default_detail().to_string(),
        };
        MultimodalElementImage {
            content_type: "image_url".to_string(),
            image_url
        }
    }
}

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Default)]
pub struct MultimodalElementImageImageURL {
    pub url: String,
    #[serde(default = "default_detail")]
    pub detail: String,
}

fn default_detail() -> String {
    "high".to_string()
}

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq)]
#[serde(untagged)] // tries to deserialize each enum variant in order
pub enum ChatMultimodalElement {
    MultimodalElementText(MultimodalElementText),
    MultiModalImageURLElement(MultimodalElementImage),
}

impl Default for ChatMultimodalElement {
    fn default() -> Self {
        ChatMultimodalElement::MultimodalElementText(MultimodalElementText {
            content_type: "text".to_string(),
            text: String::new(),
        })
    }
}

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq)]
#[serde(untagged)]
pub enum ChatContent {
    SimpleText(String),
    Multimodal(Vec<ChatMultimodalElement>),
}

impl Default for ChatContent {
    fn default() -> Self {
        ChatContent::SimpleText(String::new())
    }
}

impl ChatContent {
    pub fn content_text_only(&self) -> String {
        match self {
            ChatContent::SimpleText(text) => text.clone(),
            ChatContent::Multimodal(elements) => {
                elements
                    .iter()
                    .filter_map(|element| {
                        match element {
                            ChatMultimodalElement::MultimodalElementText(el) => Some(el.text.clone()),
                            _ => None,
                        }
                    })
                    .collect::<Vec<String>>()
                    .join("\n\n")
            }
        }
    }

    pub fn size_estimate(&self, tokenizer: Arc<RwLock<Tokenizer>>) -> usize {
        match self {
            ChatContent::SimpleText(text) => text.len(),
            ChatContent::Multimodal(_elements) => { 
                let tcnt = self.count_tokens(tokenizer).unwrap_or(0);
                (tcnt as f32 * 2.618) as usize
            }
        }
    }

    pub fn count_tokens(&self, tokenizer: Arc<RwLock<Tokenizer>>) -> Result<i32, String> {
        fn count_tokens_simple_text(tokenizer_lock: &RwLockWriteGuard<Tokenizer>, text: &str) -> Result<i32, String> {
            tokenizer_lock.encode(text, false)
                .map(|tokens|tokens.len() as i32)
                .map_err(|e|format!("Tokenizing error: {e}"))
        }
        let tokenizer_lock = tokenizer.write().unwrap();
        match self {
            ChatContent::SimpleText(text) => count_tokens_simple_text(&tokenizer_lock, text),
            ChatContent::Multimodal(elements) => {
                let mut tcnt = 0;
                for e in elements {
                    tcnt += match e {
                        ChatMultimodalElement::MultimodalElementText(el) => count_tokens_simple_text(&tokenizer_lock, el.text.as_str())?,
                        ChatMultimodalElement::MultiModalImageURLElement(el) => {
                            if let Some(image_base64) = parse_image_b64_from_image_url(el.image_url.url.as_str()) {
                                calculate_image_tokens_openai(&image_base64, &el.image_url.detail)?
                            } else {
                                0
                            }
                        }
                    };
                }
                Ok(tcnt)
            }
        }
    }
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct ChatToolFunction {
    pub arguments: String,
    pub name: String,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct ChatToolCall {
    pub id: String,
    pub function: ChatToolFunction,
    #[serde(rename = "type")]
    pub tool_type: String,
}

#[derive(Debug, Serialize, Deserialize, Clone, Default)]
pub struct ChatUsage {
    pub prompt_tokens: usize,
    pub completion_tokens: usize,
    pub total_tokens: usize,   // TODO: remove (can produce self-contradictory data when prompt+completion != total)
}

#[derive(Debug, Serialize, Deserialize, Clone, Default)]
pub struct ChatMessage {
    pub role: String,
    #[serde(default, deserialize_with="deserialize_chat_content")]
    pub content: ChatContent,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_calls: Option<Vec<ChatToolCall>>,
    #[serde(default)]
    pub tool_call_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub usage: Option<ChatUsage>,
}

impl ChatMessage {
    pub fn new(role: String, content: String) -> Self {
        ChatMessage {
            role,
            content: ChatContent::SimpleText(content),
            ..Default::default()
        }
    }

    pub fn drop_usage(&self) -> ChatMessage {
        ChatMessage {
            role: self.role.clone(),
            content: self.content.clone(),
            tool_calls: self.tool_calls.clone(),
            tool_call_id: self.tool_call_id.clone(),
            usage: None,
        }
    }
}

fn deserialize_chat_content<'de, D>(deserializer: D) -> Result<ChatContent, D::Error>
where
    D: Deserializer<'de>,
{
    let value: serde_json::Value = Deserialize::deserialize(deserializer)?;
    chat_content_from_value(value).map_err(serde::de::Error::custom)
}

pub fn chat_content_from_value(value: serde_json::Value) -> Result<ChatContent, String> {
    fn validate_multimodal_element(element: &ChatMultimodalElement) -> Result<(), String> {
        match element {
            ChatMultimodalElement::MultimodalElementText(el) => {
                if el.content_type != "text" {
                    return Err("Invalid multimodal element: type must be `text`".to_string());
                }
            },
            ChatMultimodalElement::MultiModalImageURLElement(el) => {
                if el.content_type != "image_url" {
                    return Err("Invalid multimodal element: type must be `image_url`".to_string());
                }
                if parse_image_b64_from_image_url(&el.image_url.url).is_none() {
                    return Err("Invalid image URL in MultimodalElementImage: must pass regexp `data:image/(png|jpeg|jpg|webp|gif);base64,([A-Za-z0-9+/=]+)`".to_string());
                }
            }
        };
        Ok(())
    }

    match value {
        serde_json::Value::Null => Ok(ChatContent::SimpleText(String::new())),
        serde_json::Value::String(s) => Ok(ChatContent::SimpleText(s)),
        serde_json::Value::Array(array) => {
            let mut elements = vec![];
            for (idx, item) in array.into_iter().enumerate() {
                let element: ChatMultimodalElement = serde_json::from_value(item)
                    .map_err(|e| format!("Error deserializing element at index {}: {}", idx, e))?;
                validate_multimodal_element(&element)
                    .map_err(|e| format!("Validation error for element at index {}: {}", idx, e))?;
                elements.push(element);
            }
            if elements.len() == 1 {
                if let ChatMultimodalElement::MultimodalElementText(el) = &elements[0] {
                    return Ok(ChatContent::SimpleText(el.text.clone()));
                }
            }
            Ok(ChatContent::Multimodal(elements))
        },
        _ => Err("deserialize_chat_content() can't parse content".to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_image_b64_from_image_url() {
        let image_url = "data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAUA";
        let expected_base64 = "iVBORw0KGgoAAAANSUhEUgAAAAUA";
        assert_eq!(parse_image_b64_from_image_url(image_url), Some(expected_base64.to_string()));

        let invalid_image_url = "data:image/png;base64,";
        assert_eq!(parse_image_b64_from_image_url(invalid_image_url), None);

        let non_matching_url = "https://example.com/image.png";
        assert_eq!(parse_image_b64_from_image_url(non_matching_url), None);
    }
}
