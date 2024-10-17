use serde::{Deserialize, Deserializer, Serialize};
use std::sync::{Arc, RwLock, RwLockReadGuard};
use tokenizers::Tokenizer;
use crate::scratchpads::scratchpad_utils::{calculate_image_tokens_openai, count_tokens_simple_text, parse_image_b64_from_image_url_openai};


#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Default)]
pub struct MultimodalElement {
    pub m_type: String, // text or image/* 
    pub m_content: String,
    pub m_encoding: String,
    pub m_from: String,
}

impl MultimodalElement {
    pub fn from_openai_image(openai_image: MultimodalElementImageOpenAI) -> Result<Self, String> {
        let (image_type, image_encoding, image_content) = parse_image_b64_from_image_url_openai(&openai_image.image_url.url)
            .ok_or(format!("Failed to parse image URL: {}", openai_image.image_url.url))?;
        Ok(MultimodalElement {
            m_type: image_type.to_string(),
            m_content: image_content,
            m_encoding: image_encoding,
            m_from: "openai_image".to_string()
        })
    }
    
    pub fn from_openai_text(openai_text: MultimodalElementTextOpenAI) -> Self {
        MultimodalElement {
            m_type: "text".to_string(),
            m_content: openai_text.text,
            m_from: "openai_text".to_string(),
            ..Default::default()
        }
    }
    
    pub fn to_orig(&self) -> ChatMultimodalElement {
        match self.m_from.as_str() {
            "openai_image" => self.to_openai_image(),
            "openai_text" => self.to_openai_text(),
            _ => unreachable!()
        }
    }
    
    fn to_openai_image(&self) -> ChatMultimodalElement {
        let image_url = format!("data:{};{},{}", self.m_type, self.m_encoding, self.m_content);
        ChatMultimodalElement::MultimodalElementImageOpenAI(MultimodalElementImageOpenAI {
            content_type: "image_url".to_string(),
            image_url: MultimodalElementImageOpenAIImageURL {
                url: image_url.clone(),
                detail: "high".to_string(),
            }
        })
    }
    
    fn to_openai_text(&self) -> ChatMultimodalElement {
        ChatMultimodalElement::MultimodalElementTextOpenAI(MultimodalElementTextOpenAI {
            content_type: "text".to_string(),
            text: self.m_content.clone(),
        })
    }
    
    pub fn count_tokens(&self, tokenizer: Option<&RwLockReadGuard<Tokenizer>>) -> Result<i32, String> {
        if self.m_type == "text" {
            if let Some(tokenizer) = tokenizer {
                count_tokens_simple_text(&tokenizer, &self.m_content)
            } else {
                return Err("count_tokens() received no tokenizer".to_string());
            }
        } else if self.m_type.starts_with("image") {
            match self.m_from.as_str() {
                "openai_image" => calculate_image_tokens_openai(&self.m_content, "high"),
                _ => unreachable!(), 
            } 
        } else {
            unreachable!()
        } 
    } 
    
    pub fn m_from_prefix(&self) -> String {
        self.m_from.split("_").next().expect("'_' in m_from is not found").to_string()
    }
}
    
    // pub fn change_m_type_from_image_to_text(&mut self) {
    //     if self.m_type == 
    // }

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Default)]
pub struct MultimodalElementTextOpenAI {
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
pub struct MultimodalElementImageOpenAIImageURL {
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
    MultimodalElement(MultimodalElement), // default internal structure
    // transform structures below into MultimodalElement
    MultimodalElementTextOpenAI(MultimodalElementTextOpenAI),
    MultimodalElementImageOpenAI(MultimodalElementImageOpenAI),
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
            ChatContent::Multimodal(elements) => elements.iter()
                .filter_map(|element| {
                    if let ChatMultimodalElement::MultimodalElement(el) = element {
                        if el.m_type == "text" {
                            return Some(el.m_content.clone());
                        }
                    }
                    None
                })
                .collect::<Vec<_>>()
                .join("\n\n"),
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
        let tokenizer_lock = tokenizer.read().unwrap();
        match self {
            ChatContent::SimpleText(text) => count_tokens_simple_text(&tokenizer_lock, text),
            ChatContent::Multimodal(elements) => elements.iter()
                .map(|e| match e {
                    ChatMultimodalElement::MultimodalElement(e) => e.count_tokens(Some(&tokenizer_lock)),
                    _ => unreachable!(),
                })
                .collect::<Result<Vec<_>, _>>()
                .map(|counts| counts.iter().sum()),
        }
    }
    
    pub fn to_orig_format(&self) -> ChatContent {
        match self {
            ChatContent::SimpleText(text) => ChatContent::SimpleText(text.clone()),
            ChatContent::Multimodal(elements) => {
                let orig_elements = elements.iter()
                    .map(|element| {
                        match element {
                            ChatMultimodalElement::MultimodalElement(el) => {
                                el.to_orig()
                            },
                            _ => unreachable!(),
                        }
                    })
                    .collect();
                ChatContent::Multimodal(orig_elements)
            }
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
            ChatMultimodalElement::MultimodalElementTextOpenAI(el) => {
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
            _ => unreachable!(),
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

                match element {
                    ChatMultimodalElement::MultimodalElement(el) => {
                        elements.push(el);
                    }
                    ChatMultimodalElement::MultimodalElementTextOpenAI(el) => {
                        elements.push(
                            MultimodalElement::from_openai_text(el)
                        );
                    }
                    ChatMultimodalElement::MultimodalElementImageOpenAI(el) => {
                        elements.push(
                            MultimodalElement::from_openai_image(el)?
                        );
                    }
                }
            }

            if elements.len() == 1 {
                if elements[0].m_type == "text" {
                    return Ok(ChatContent::SimpleText(elements[0].m_content.clone()));
                }
            }
            Ok(ChatContent::Multimodal(
                elements.into_iter().map(|el|ChatMultimodalElement::MultimodalElement(el)).collect::<Vec<_>>()
            ))
        },
        _ => Err("deserialize_chat_content() can't parse content".to_string()),
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

    pub fn to_orig_format(&self) -> ChatMessage {
        ChatMessage {
            role: self.role.clone(),
            content: self.content.to_orig_format(),
            tool_calls: self.tool_calls.clone(),
            tool_call_id: self.tool_call_id.clone(),
            usage: None,
        }
    }
}
