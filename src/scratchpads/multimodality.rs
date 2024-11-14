use serde::{Deserialize, Deserializer, Serialize};
use std::sync::{Arc, RwLock, RwLockReadGuard};
use serde_json::{json, Value};
use tokenizers::Tokenizer;
use crate::call_validation::{ChatContent, ChatMessage, ChatToolCall, ChatToolFunction};
use crate::scratchpads::scratchpad_utils::{calculate_image_tokens_openai, count_tokens as count_tokens_simple_text, image_reader_from_b64string, parse_image_b64_from_image_url_openai};


#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Default)]
pub struct MultimodalElement {
    pub m_type: String, // "text", "image/png" etc
    pub m_content: String,
}

impl MultimodalElement {
    pub fn new(m_type: String, m_content: String) -> Result<Self, String> {
        if !(m_type == "text") && !m_type.starts_with("image/") {
            return Err(format!("MultimodalElement::new() received invalid type: {}", m_type));
        }
        if m_type.starts_with("image/") {
            let _ = image_reader_from_b64string(&m_content)
                .map_err(|e| format!("MultimodalElement::new() failed to parse m_content: {}", e));
        }
        Ok(MultimodalElement { m_type, m_content })
    }

    pub fn is_text(&self) -> bool {
        self.m_type == "text"
    }

    pub fn is_image(&self) -> bool {
        self.m_type.starts_with("image/")
    }

    pub fn from_openai_image(openai_image: MultimodalElementImageOpenAI) -> Result<Self, String> {
        let (image_type, _, image_content) = parse_image_b64_from_image_url_openai(&openai_image.image_url.url)
            .ok_or(format!("Failed to parse image URL: {}", openai_image.image_url.url))?;
        MultimodalElement::new(image_type, image_content)
    }

    pub fn from_text(openai_text: MultimodalElementText) -> Result<Self, String> {
        MultimodalElement::new("text".to_string(), openai_text.text)
    }

    pub fn from_anthropic_tool_use(el: MultimodalElementToolUseAnthropic) -> ChatMessage {
        ChatMessage {
            role: "assistant".to_string(),
            content: ChatContent::SimpleText("".to_string()),
            tool_calls: Some(vec![ChatToolCall {
                id: el.id.clone(),
                tool_type: "function".to_string(),
                function: ChatToolFunction {
                    arguments: el.input.to_string(),
                    name: el.name.clone(),
                }
            }
            ]),
            tool_call_id: "".to_string(),
            usage: None,
        }
    }

    pub fn from_anthropic_tool_result(el: MultimodalElementToolResultAnthropic) -> ChatMessage {
        ChatMessage {
            role: "tool".to_string(),
            content: ChatContent::SimpleText("".to_string()),
            tool_calls: None,
            tool_call_id: el.tool_use_id.clone(),
            usage: None,
        }
    }
    
    pub fn to_orig(&self, style: &str) -> ChatMultimodalElement {
        match style {
            "openai" => {
                if self.is_text() {
                    self.to_text()
                } else if self.is_image() {
                    self.to_openai_image()
                } else {
                    unreachable!()
                }
            },
            "anthropic" => {
                if self.is_text() {
                    self.to_text()
                } else if self.is_image() {
                    todo!()
                } else {
                    unreachable!()
                }
            }
            _ => unreachable!()
        }
    }

    fn to_openai_image(&self) -> ChatMultimodalElement {
        let image_url = format!("data:{};base64,{}", self.m_type, self.m_content);
        ChatMultimodalElement::MultimodalElementImageOpenAI(MultimodalElementImageOpenAI {
            content_type: "image_url".to_string(),
            image_url: MultimodalElementImageOpenAIImageURL {
                url: image_url.clone(),
                detail: "high".to_string(),
            }
        })
    }

    fn to_text(&self) -> ChatMultimodalElement {
        ChatMultimodalElement::MultimodalElementText(MultimodalElementText {
            content_type: "text".to_string(),
            text: self.m_content.clone(),
        })
    }

    pub fn count_tokens(&self, tokenizer: Option<&RwLockReadGuard<Tokenizer>>, style: &str) -> Result<i32, String> {
        if self.is_text() {
            if let Some(tokenizer) = tokenizer {
                Ok(count_tokens_simple_text(&tokenizer, &self.m_content) as i32)
            } else {
                Err("count_tokens() received no tokenizer".to_string())
            }
        } else if self.is_image() {
            match style {
                "openai" => {
                    calculate_image_tokens_openai(&self.m_content, "high")
                },
                _ => unreachable!(),
            }
        } else {
            unreachable!()
        }
    }
}

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Default)]
pub struct MultimodalElementText {
    #[serde(rename = "type")]
    pub content_type: String,
    pub text: String,
}

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Default)]
pub struct MultimodalElementImageOpenAI {
    #[serde(rename = "type")]
    pub content_type: String,
    pub image_url: MultimodalElementImageOpenAIImageURL,
}

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Default)]
pub struct MultimodalElementToolResultAnthropic {
    #[serde(rename = "type")]
    pub content_type: String, // type="tool_result"
    pub tool_use_id: String,
    pub content: ChatContentRaw,
}

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Default)]
pub struct MultimodalElementToolUseAnthropic {
    #[serde(rename = "type")]
    pub content_type: String, // type="tool_use"
    pub id: String,
    pub name: String,
    pub input: Value,
}

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Default)]
pub struct MultimodalElementImageOpenAIImageURL {
    pub url: String,
    #[serde(default = "default_detail")]
    pub detail: String,
}

fn default_detail() -> String {
    "high".to_string()
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum AnthropicInputElement {
    MultimodalElementText(MultimodalElementText),
    MultimodalElementToolUseAnthropic(MultimodalElementToolUseAnthropic),
}

pub fn split_anthropic_input_elements(els: Vec<AnthropicInputElement>) -> (Vec<MultimodalElementText>, Vec<MultimodalElementToolUseAnthropic>) {
    let mut text_elements = Vec::new();
    let mut tool_use_elements = Vec::new();

    for el in els {
        match el {
            AnthropicInputElement::MultimodalElementText(text_el) => text_elements.push(text_el),
            AnthropicInputElement::MultimodalElementToolUseAnthropic(tool_use_el) => tool_use_elements.push(tool_use_el),
        }
    }

    (text_elements, tool_use_elements)
}

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq)]
#[serde(untagged)] // tries to deserialize each enum variant in order
pub enum ChatMultimodalElement {
    MultimodalElementText(MultimodalElementText),
    MultimodalElementImageOpenAI(MultimodalElementImageOpenAI),
    MultimodalElementToolUseAnthropic(MultimodalElementToolUseAnthropic),
    MultimodalElementToolResultAnthropic(MultimodalElementToolResultAnthropic),
    MultimodalElement(MultimodalElement),
}

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq)]
#[serde(untagged)]
pub enum ChatContentRaw {
    SimpleText(String),
    Multimodal(Vec<ChatMultimodalElement>),
}

impl Default for ChatContentRaw {
    fn default() -> Self {
        ChatContentRaw::SimpleText(String::new())
    }
}

impl ChatContentRaw {
    pub fn to_internal_format(&self) -> Result<(ChatContent, Vec<ChatMessage>), String> {
        match self {
            ChatContentRaw::SimpleText(text) => Ok((ChatContent::SimpleText(text.clone()), vec![])),
            ChatContentRaw::Multimodal(elements) => {
                let mut internal_elements = Vec::new();
                let mut chat_messages: Vec<ChatMessage> = vec![];

                for el in elements {
                    match el {
                        ChatMultimodalElement::MultimodalElementText(text_el) => {
                            let element = MultimodalElement::from_text(text_el.clone())?;
                            internal_elements.push(element);
                        },
                        ChatMultimodalElement::MultimodalElementImageOpenAI(image_el) => {
                            let element = MultimodalElement::from_openai_image(image_el.clone())?;
                            internal_elements.push(element);
                        },
                        ChatMultimodalElement::MultimodalElementToolUseAnthropic(el) => {
                            let message = MultimodalElement::from_anthropic_tool_use(el.clone());
                            chat_messages.push(message);
                        },
                        ChatMultimodalElement::MultimodalElementToolResultAnthropic(el) => {
                            let message = MultimodalElement::from_anthropic_tool_result(el.clone());
                            chat_messages.push(message);
                        },

                        ChatMultimodalElement::MultimodalElement(el) => {
                            internal_elements.push(el.clone());
                        },
                    }
                }

                Ok((ChatContent::Multimodal(internal_elements), chat_messages))
            }
        }
    }
}

impl ChatContent {
    pub fn content_text_only(&self) -> String {
        match self {
            ChatContent::SimpleText(text) => text.clone(),
            ChatContent::Multimodal(elements) => elements.iter()
                .filter(|el|el.m_type == "text")
                .map(|el|el.m_content.clone())
                .collect::<Vec<_>>()
                .join("\n\n"),
        }
    }

    pub fn size_estimate(&self, tokenizer: Arc<RwLock<Tokenizer>>, style: &str) -> usize {
        match self {
            ChatContent::SimpleText(text) => text.len(),
            ChatContent::Multimodal(_elements) => {
                let tcnt = self.count_tokens(tokenizer, style).unwrap_or(0);
                (tcnt as f32 * 2.618) as usize
            },
        }
    }

    pub fn count_tokens(&self, tokenizer: Arc<RwLock<Tokenizer>>, style: &str) -> Result<i32, String> {
        let tokenizer_lock = tokenizer.read().unwrap();
        match self {
            ChatContent::SimpleText(text) => Ok(count_tokens_simple_text(&tokenizer_lock, text) as i32),
            ChatContent::Multimodal(elements) => elements.iter()
                .map(|e|e.count_tokens(Some(&tokenizer_lock), style))
                .collect::<Result<Vec<_>, _>>()
                .map(|counts| counts.iter().sum()),
        }
    }

    pub fn into_raw(&self, style: &str) -> ChatContentRaw {
        match self {
            ChatContent::SimpleText(text) => ChatContentRaw::SimpleText(text.clone()),
            ChatContent::Multimodal(elements) => {
                let orig_elements = elements.iter()
                    .map(|el| el.to_orig(style))
                    .collect::<Vec<_>>();
                ChatContentRaw::Multimodal(orig_elements)
            }
        }
    }
}

pub fn chat_content_raw_from_value(value: Value) -> Result<ChatContentRaw, String> {
    fn validate_multimodal_element(element: &ChatMultimodalElement) -> Result<(), String> {
        match element {
            ChatMultimodalElement::MultimodalElementText(el) => {
                if el.content_type != "text" {
                    return Err("Invalid multimodal element: type must be `text`".to_string());
                }
            },
            ChatMultimodalElement::MultimodalElementImageOpenAI(el) => {
                if el.content_type != "image_url" {
                    return Err("Invalid multimodal element: type must be `image_url`".to_string());
                }
                if parse_image_b64_from_image_url_openai(&el.image_url.url).is_none() {
                    return Err("Invalid image URL in MultimodalElementImageOpenAI: must pass regexp `data:image/(png|jpeg|jpg|webp|gif);base64,([A-Za-z0-9+/=]+)`".to_string());
                }
            },
            ChatMultimodalElement::MultimodalElementToolUseAnthropic(_el) => {},
            ChatMultimodalElement::MultimodalElementToolResultAnthropic(_el) => {},
            ChatMultimodalElement::MultimodalElement(_el) => {},
        };
        Ok(())
    }

    match value {
        Value::Null => Ok(ChatContentRaw::SimpleText(String::new())),
        Value::String(s) => Ok(ChatContentRaw::SimpleText(s)),
        Value::Array(array) => {
            let mut elements = vec![];
            for (idx, item) in array.into_iter().enumerate() {
                let element: ChatMultimodalElement = serde_json::from_value(item.clone())
                    .map_err(|e| format!("Error deserializing element at index {}:\n{:#?}\n\nError: {}", idx, item, e))?;
                validate_multimodal_element(&element)
                    .map_err(|e| format!("Validation error for element at index {}: {}", idx, e))?;
                elements.push(element);
            }

            Ok(ChatContentRaw::Multimodal(elements))
        },
        _ => Err("deserialize_chat_content() can't parse content".to_string()),
    }
}

impl ChatMessage {
    pub fn new(role: String, content: String) -> Self {
        ChatMessage {
            role,
            content: ChatContent::SimpleText(content),
            ..Default::default()
        }
    }

    pub fn into_value(&self, style: &str) -> Value {
        let mut dict = serde_json::Map::new();
        let chat_content_raw = self.content.into_raw(style);

        dict.insert("role".to_string(), Value::String(self.role.clone()));
        dict.insert("content".to_string(), json!(chat_content_raw));

        match style {
            "openai" => {
                dict.insert("tool_calls".to_string(), json!(self.tool_calls.clone()));
                dict.insert("tool_call_id".to_string(), Value::String(self.tool_call_id.clone()));
            },
            "anthropic" => {
                if self.role == "tool" {
                    let content = vec![json!({
                        "type": "tool_result",
                        "tool_use_id": self.tool_call_id.clone(),
                        "content": self.content.clone().into_raw(style),
                    })];
                    dict.insert("role".to_string(), Value::String("user".to_string()));
                    dict.insert("content".to_string(), Value::Array(content));
                }

                if self.role == "assistant" && self.tool_calls.is_some() {
                    let tool_calls = self.tool_calls.clone().unwrap_or_default();
                    let content = tool_calls.iter().map(|call| {
                        let input_map: serde_json::Map<String, Value> = serde_json::from_str(&call.function.arguments)
                            .unwrap_or_else(|_| serde_json::Map::new());
                        json!({
                            "type": "tool_use",
                            "id": call.id.clone(),
                            "name": call.function.name.clone(),
                            "input": input_map,
                        })
                    }).collect::<Vec<_>>();
                    dict.insert("content".to_string(), Value::Array(content));
                }
            },
            _ => unreachable!(),
        }

        Value::Object(dict)
    }

    pub fn from_anthropic_input(els: Vec<AnthropicInputElement>, role: &str) -> Self {
        let (text_elements, tool_use_elements) = split_anthropic_input_elements(els);
        let content = text_elements.iter().map(|x|x.text.clone()).collect::<Vec<_>>().join("\n\n");

        if !tool_use_elements.is_empty() {
            ChatMessage {
                role: role.to_string(),
                content: ChatContent::SimpleText(content),
                tool_calls: Some(tool_use_elements.iter().map(|m| ChatToolCall {
                    id: m.id.clone(),
                    function: ChatToolFunction {
                        arguments: m.input.to_string(),
                        name: m.name.clone()
                    },
                    tool_type: "function".to_string(),
                }).collect::<Vec<_>>()),
                tool_call_id: "".to_string(),
                usage: None,
            }
        } else {
            ChatMessage::new(role.to_string(), content)
        }
    }
}

#[derive(Debug, Serialize, Clone, PartialEq, Default)]
pub struct ChatMessages(pub Vec<ChatMessage>);

impl<'de> Deserialize<'de> for ChatMessages {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value: Value = Deserialize::deserialize(deserializer)?;
        let mut messages = Vec::new();

        if let Value::Array(array) = value {
            for item in array {
                let role = item.get("role")
                    .and_then(|s| s.as_str())
                    .ok_or_else(|| serde::de::Error::missing_field("role"))?
                    .to_string();

                let (content, chat_messages) = match item.get("content") {
                    Some(content_value) => {
                        let content_raw: ChatContentRaw = chat_content_raw_from_value(content_value.clone())
                            .map_err(|e| serde::de::Error::custom(e))?;
                        content_raw.to_internal_format()
                            .map_err(|e| serde::de::Error::custom(e))?
                    },
                    None => (ChatContent::SimpleText(String::new()), vec![]),
                };

                let tool_calls: Option<Vec<ChatToolCall>> = item.get("tool_calls")
                    .and_then(|v| v.as_array())
                    .map(|v| v.iter().map(|v| serde_json::from_value(v.clone()).map_err(serde::de::Error::custom)).collect::<Result<Vec<_>, _>>())
                    .transpose()?;
                let tool_call_id: Option<String> = item.get("tool_call_id")
                    .and_then(|s| s.as_str()).map(|s| s.to_string());

                messages.push(ChatMessage {
                    role,
                    content,
                    tool_calls,
                    tool_call_id: tool_call_id.unwrap_or_default(),
                    ..Default::default()
                });
                messages.extend(chat_messages);
            }
        } else {
            return Err(serde::de::Error::custom("Expected an array of chat messages"));
        }

        Ok(ChatMessages(messages))
    }
}
