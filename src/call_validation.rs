use serde::{Deserialize, Deserializer, Serialize};
use std::collections::HashMap;
use std::hash::Hash;
use std::sync::{Arc, RwLock, RwLockWriteGuard};
use axum::http::StatusCode;
use indexmap::IndexMap;
use regex::Regex;
use ropey::Rope;
use tokenizers::Tokenizer;

use crate::custom_error::ScratchError;
use crate::scratchpads::chat_utils_limit_history::calculate_image_tokens_openai;


#[derive(Debug, Serialize, Deserialize, Clone, Default)]
pub struct CursorPosition {
    pub file: String,
    pub line: i32,
    pub character: i32,
}

#[derive(Debug, Serialize, Deserialize, Clone, Default)]
pub struct CodeCompletionInputs {
    pub sources: HashMap<String, String>,
    pub cursor: CursorPosition,
    pub multiline: bool,
}

#[derive(Debug, Serialize, Deserialize, Clone, Default)]
pub struct SamplingParameters {
    #[serde(default)]
    pub max_new_tokens: usize,
    pub temperature: Option<f32>,
    pub top_p: Option<f32>,
    #[serde(default)]
    pub stop: Vec<String>,
    pub n: Option<usize>,
}

#[derive(Debug, Deserialize, Clone)]
pub struct CodeCompletionPost {
    pub inputs: CodeCompletionInputs,
    #[serde(default)]
    pub parameters: SamplingParameters,
    #[serde(default)]
    pub model: String,
    #[serde(default)]
    pub scratchpad: String,
    #[serde(default)]
    pub stream: bool,
    #[serde(default)]
    pub no_cache: bool,
    #[serde(default)]
    pub use_ast: bool,
    #[allow(dead_code)]
    #[serde(default)]
    pub use_vecdb: bool,
    #[serde(default)]
    pub rag_tokens_n: usize,
}

pub fn code_completion_post_validate(code_completion_post: CodeCompletionPost) -> axum::response::Result<(), ScratchError> {
    let pos = code_completion_post.inputs.cursor.clone();
    let Some(source) = code_completion_post.inputs.sources.get(&code_completion_post.inputs.cursor.file) else {
        return Err(ScratchError::new(StatusCode::BAD_REQUEST, "invalid post".to_string()))
    };
    let text = Rope::from_str(&*source);
    let line_number = pos.line as usize;
    if line_number >= text.len_lines() {
        return Err(ScratchError::new(StatusCode::BAD_REQUEST, "invalid post".to_string()))
    }
    let line = text.line(line_number);
    let col = pos.character as usize;
    if col > line.len_chars() {
        return Err(ScratchError::new(StatusCode::BAD_REQUEST, "invalid post".to_string()))
    }
    Ok(())
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct ContextFile {
    pub file_name: String,
    pub file_content: String,
    pub line1: usize,   // starts from 1, zero means non-valid
    pub line2: usize,   // starts from 1
    #[serde(default, skip_serializing)]
    pub symbols: Vec<String>,
    #[serde(default = "default_gradient_type_value", skip_serializing)]
    pub gradient_type: i32,
    #[serde(default, skip_serializing)]
    pub usefulness: f32,  // higher is better
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub enum ContextEnum {
    ContextFile(ContextFile),
    ChatMessage(ChatMessage),
}

fn default_gradient_type_value() -> i32 {
    -1
}

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq)]
pub enum ChatMultimodalElement {
    MultimodalTextElement(MultimodalTextElement),
    MultiModalImageURLElement(MultimodalImageURLElement),
}

impl Default for ChatMultimodalElement {
    fn default() -> Self {
        ChatMultimodalElement::MultimodalTextElement(MultimodalTextElement {
            content_type: "text".to_string(),
            text: String::new(),
        })
    }
}

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Default)]
pub struct MultimodalTextElement {
    #[serde(rename = "type")]
    pub content_type: String,
    pub text: String,
}

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Default)]
pub struct MultimodalImageURLElement {
    #[serde(rename = "type")]
    pub content_type: String,
    pub image_url: MultimodalImageURLElementImageURL,
}

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Default)]
pub struct MultimodalImageURLElementImageURL {
    pub url: String,
    #[serde(default = "default_detail")]
    pub detail: String,
}

fn default_detail() -> String {
    "high".to_string()
}

// todo: images via links are yet not implemented: unclear how to calculate tokens
fn parse_image_b64_from_image_url(image_url: &str) -> Option<String> {
    let re = Regex::new(r"data:image/(png|jpeg|jpg|webp|gif);base64,([A-Za-z0-9+/=]+)").unwrap();
    re.captures(image_url).and_then(|captures| {
        captures.get(2).map(|m| m.as_str().to_string())
    })
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

pub fn chat_content_from_value(value: serde_json::Value) -> Result<ChatContent, String> {
    fn validate_multimodal_element(element: &ChatMultimodalElement) -> Result<(), String> {
        match element {
            ChatMultimodalElement::MultimodalTextElement(el) => {
                if el.content_type!= "text" {
                    return Err("Invalid multimodal element: type must be `text`".to_string());
                }
            },
            ChatMultimodalElement::MultiModalImageURLElement(el) => {
                if el.content_type != "image_url" {
                    return Err("Invalid multimodal element: type must be `image_url`".to_string());
                }
                if parse_image_b64_from_image_url(&el.image_url.url).is_none() {
                    return Err("Invalid image URL in MultimodalImageURLElement: must pass regexp `data:image/(png|jpeg|jpg|webp|gif);base64,([A-Za-z0-9+/=]+)`".to_string());
                }
            }
        };
        Ok(())
    }
    
    match value {
        serde_json::Value::String(s) => Ok(ChatContent::SimpleText(s)),
        serde_json::Value::Array(array) => {
            let elements: Vec<ChatMultimodalElement> = serde_json::from_value(serde_json::Value::Array(array))
                .map_err(|e| e.to_string())?;
            for e in elements.iter() {
                validate_multimodal_element(e)?;
            }
            if elements.len() == 1 {
                if let ChatMultimodalElement::MultimodalTextElement(el) = &elements[0] {
                    return Ok(ChatContent::SimpleText(el.text.clone()));
                }
            }
            Ok(ChatContent::Multimodal(elements))
        },
        _ => Err("deserialize_chat_content() can't parse content".to_string()),
    }
}

fn deserialize_chat_content<'de, D>(deserializer: D) -> Result<ChatContent, D::Error>
where
    D: Deserializer<'de>,
{
    let value: serde_json::Value = Deserialize::deserialize(deserializer)?;
    chat_content_from_value(value).map_err(serde::de::Error::custom)
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

impl ChatContent {
    pub fn content_text_only(&self) -> String {
        match self {
            ChatContent::SimpleText(text) => text.clone(),
            ChatContent::Multimodal(elements) => {
                elements
                    .iter()
                    .filter_map(|element| {
                        match element {
                            ChatMultimodalElement::MultimodalTextElement(el) => Some(el.text.clone()),
                            _ => None,
                        }
                    })
                    .collect::<Vec<String>>()
                    .join("\n\n")
            }
        }
    }

    pub fn size_estimate(&self) -> usize {
        match self {
            ChatContent::SimpleText(text) => text.len(),
            _ => unreachable!()
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
                        ChatMultimodalElement::MultimodalTextElement(el) => count_tokens_simple_text(&tokenizer_lock, el.text.as_str())?,
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
pub struct SubchatParameters {
    pub subchat_model: String,
    pub subchat_n_ctx: usize,
    #[serde(default)]
    pub subchat_tokens_for_rag: usize,
    #[serde(default)]
    pub subchat_temperature: Option<f32>,
    #[serde(default)]
    pub subchat_max_new_tokens: usize,
}

#[derive(Debug, Deserialize, Clone)]
pub struct ChatPost {
    pub messages: Vec<ChatMessage>,
    #[serde(default)]
    pub parameters: SamplingParameters,
    #[serde(default)]
    pub model: String,
    #[serde(default)]
    pub scratchpad: String,
    pub stream: Option<bool>,
    pub temperature: Option<f32>,
    #[serde(default)]
    pub max_tokens: usize,
    #[serde(default)]
    pub n: Option<usize>,
    #[serde(default)]
    pub tools: Option<Vec<serde_json::Value>>,
    #[serde(default)]
    pub tool_choice: Option<String>,
    #[serde(default)]
    pub only_deterministic_messages: bool,  // means don't sample from the model
    #[serde(default)]
    pub subchat_tool_parameters: IndexMap<String, SubchatParameters>, // tool_name: {model, allowed_context, temperature}
    #[serde(default="PostprocessSettings::new")]
    pub postprocess_parameters: PostprocessSettings,
    #[allow(dead_code)]
    #[serde(default)]
    pub chat_id: String,
}

fn default_true() -> bool {
    true
}

#[derive(Serialize, Deserialize, Clone, Hash, Debug, Eq, PartialEq, Default, Ord, PartialOrd)]
pub struct DiffChunk {
    pub file_name: String,
    pub file_action: String, // edit, rename, add, remove
    pub line1: usize,
    pub line2: usize,
    pub lines_remove: String,
    pub lines_add: String,
    #[serde(default)]
    pub file_name_rename: Option<String>,
    #[serde(default = "default_true", skip_serializing)]
    pub is_file: bool,
}

#[derive(Debug, Deserialize, Clone)]
#[serde(default)]
pub struct PostprocessSettings {
    pub useful_background: f32,          // first, fill usefulness of all lines with this
    pub useful_symbol_default: f32,      // when a symbol present, set usefulness higher
    // search results fill usefulness as it passed from outside
    pub downgrade_parent_coef: f32,      // goto parent from search results and mark it useful, with this coef
    pub downgrade_body_coef: f32,        // multiply body usefulness by this, so it's less useful than the declaration
    pub comments_propagate_up_coef: f32, // mark comments above a symbol as useful, with this coef
    pub close_small_gaps: bool,
    pub take_floor: f32,                 // take/dont value
    pub max_files_n: usize,              // don't produce more than n files in output
}

impl Default for PostprocessSettings {
    fn default() -> Self {
        Self::new()
    }
}

impl PostprocessSettings {
    pub fn new() -> Self {
        PostprocessSettings {
            downgrade_body_coef: 0.8,
            downgrade_parent_coef: 0.6,
            useful_background: 5.0,
            useful_symbol_default: 10.0,
            close_small_gaps: true,
            comments_propagate_up_coef: 0.99,
            take_floor: 0.0,
            max_files_n: 0,
        }
    }
}


#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use crate::call_validation::{CodeCompletionInputs, CursorPosition, SamplingParameters};
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

    #[test]
    fn test_valid_post1() {
        let post = CodeCompletionPost {
            inputs: CodeCompletionInputs {
                sources: HashMap::from_iter([("hello.py".to_string(), "def hello_world():".to_string())]),
                cursor: CursorPosition {
                    file: "hello.py".to_string(),
                    line: 0,
                    character: 18,
                },
                multiline: true,
            },
            parameters: SamplingParameters {
                max_new_tokens: 20,
                temperature: Some(0.1),
                top_p: None,
                stop: vec![],
                n: None
            },
            model: "".to_string(),
            scratchpad: "".to_string(),
            stream: false,
            no_cache: false,
            use_ast: true,
            use_vecdb: true,
            rag_tokens_n: 0,
        };
        assert!(code_completion_post_validate(post).is_ok());
    }

    #[test]
    fn test_valid_post2() {
        let post = CodeCompletionPost {
            inputs: CodeCompletionInputs {
                sources: HashMap::from_iter([("hello.py".to_string(), "你好世界Ωßåß🤖".to_string())]),
                cursor: CursorPosition {
                    file: "hello.py".to_string(),
                    line: 0,
                    character: 10,
                },
                multiline: true,
            },
            parameters: SamplingParameters {
                max_new_tokens: 20,
                temperature: Some(0.1),
                top_p: None,
                stop: vec![],
                n: None,
            },
            model: "".to_string(),
            scratchpad: "".to_string(),
            stream: false,
            no_cache: false,
            use_ast: true,
            use_vecdb: true,
            rag_tokens_n: 0,
        };
        assert!(code_completion_post_validate(post).is_ok());
    }

    #[test]
    fn test_invalid_post_incorrect_line() {
        let post = CodeCompletionPost {
            inputs: CodeCompletionInputs {
                sources: HashMap::from_iter([("hello.py".to_string(), "def hello_world():".to_string())]),
                cursor: CursorPosition {
                    file: "hello.py".to_string(),
                    line: 2,
                    character: 18,
                },
                multiline: true,
            },
            parameters: SamplingParameters {
                max_new_tokens: 20,
                temperature: Some(0.1),
                top_p: None,
                stop: vec![],
                n: None,
            },
            model: "".to_string(),
            scratchpad: "".to_string(),
            stream: false,
            no_cache: false,
            use_ast: true,
            use_vecdb: true,
            rag_tokens_n: 0,
        };
        assert!(code_completion_post_validate(post).is_err());
    }

    #[test]
    fn test_invalid_post_incorrect_col() {
        let post = CodeCompletionPost {
            inputs: CodeCompletionInputs {
                sources: HashMap::from_iter([("hello.py".to_string(), "def hello_world():".to_string())]),
                cursor: CursorPosition {
                    file: "hello.py".to_string(),
                    line: 0,
                    character: 80,
                },
                multiline: true,
            },
            parameters: SamplingParameters {
                max_new_tokens: 20,
                temperature: Some(0.1),
                top_p: None,
                stop: vec![],
                n: None,
            },
            model: "".to_string(),
            scratchpad: "".to_string(),
            stream: false,
            no_cache: false,
            use_ast: true,
            use_vecdb: true,
            rag_tokens_n: 0,
        };
        assert!(code_completion_post_validate(post).is_err());
    }
}
