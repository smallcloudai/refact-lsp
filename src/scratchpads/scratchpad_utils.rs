use std::io::Cursor;
use image::ImageReader;
use regex::Regex;
use serde_json::Value;
use tokenizers::Tokenizer;

use crate::postprocessing::pp_context_files::RESERVE_FOR_QUESTION_AND_FOLLOWUP;
use crate::scratchpads::chat_message::{ChatContent, MultimodalElementImageOpenAI};


pub struct HasRagResults {
    pub was_sent: bool,
    pub in_json: Vec<Value>,
}

impl HasRagResults {
    pub fn new() -> Self {
        HasRagResults {
            was_sent: false,
            in_json: vec![],
        }
    }
}

impl HasRagResults {
    pub fn push_in_json(&mut self, value: Value) {
        self.in_json.push(value);
    }

    pub fn response_streaming(&mut self) -> Result<Vec<Value>, String> {
        if self.was_sent == true || self.in_json.is_empty() {
            return Ok(vec![]);
        }
        self.was_sent = true;
        Ok(self.in_json.clone())
    }
}

pub fn count_tokens(
    tokenizer: &Tokenizer,
    content: &ChatContent,
) -> usize {
    // XXX count image size
    count_tokens_text_only(tokenizer, content.content_text_only().as_str())
}

pub fn count_tokens_text_only(
    tokenizer: &Tokenizer,
    text: &str,
) -> usize {
    match tokenizer.encode(text, false) {
        Ok(tokens) => tokens.len(),
        Err(_) => 0,
    }
}

pub fn parse_image_b64_from_image_url(image_url: &str) -> Option<String> {
    let re = Regex::new(r"data:image/(png|jpeg|jpg|webp|gif);base64,([A-Za-z0-9+/=]+)").unwrap();
    re.captures(image_url).and_then(|captures| {
        captures.get(2).map(|m| m.as_str().to_string())
    })
}

pub fn multimodal_image_count_tokens(el: &MultimodalElementImageOpenAI) -> usize {
    parse_image_b64_from_image_url(el.image_url.url.as_str())
        .and_then(|image_b64| calculate_image_tokens_openai(&image_b64, &el.image_url.detail).ok())
        .unwrap_or(0) as usize
}

pub fn max_tokens_for_rag_chat(n_ctx: usize, maxgen: usize) -> usize {
    (n_ctx/2).saturating_sub(maxgen).saturating_sub(RESERVE_FOR_QUESTION_AND_FOLLOWUP)
}

fn calculate_image_tokens_by_dimensions_openai(mut width: u32, mut height: u32) -> i32 {
    // as per https://platform.openai.com/docs/guides/vision
    const SMALL_CHUNK_SIZE: u32 = 512;
    const COST_PER_SMALL_CHUNK: i32 = 170;
    const BIG_CHUNK_SIZE: u32 = 2048;
    const CONST_COST: i32 = 85;

    let shrink_factor = (width.max(height) as f64) / (BIG_CHUNK_SIZE as f64);
    if shrink_factor > 1.0 {
        width = (width as f64 / shrink_factor) as u32;
        height = (height as f64 / shrink_factor) as u32;
    }

    let width_chunks = (width as f64 / SMALL_CHUNK_SIZE as f64).ceil() as u32;
    let height_chunks = (height as f64 / SMALL_CHUNK_SIZE as f64).ceil() as u32;
    let small_chunks_needed = width_chunks * height_chunks;

    small_chunks_needed as i32 * COST_PER_SMALL_CHUNK + CONST_COST
}

// for detail = high. all images w detail = low cost 85 tokens (independent of image size)
pub fn calculate_image_tokens_openai(image_string: &String, detail: &String) -> Result<i32, String> {
    #[allow(deprecated)]
    let image_bytes = base64::decode(image_string).map_err(|_| "base64 decode failed".to_string())?;
    let cursor = Cursor::new(image_bytes);
    let reader = ImageReader::new(cursor).with_guessed_format().map_err(|e| e.to_string())?;
    let (width, height) = reader.into_dimensions().map_err(|_| "Failed to get dimensions".to_string())?;

    match detail.as_str() {
        "high" => Ok(calculate_image_tokens_by_dimensions_openai(width, height)),
        "low" => Ok(85),
        _ => Err("detail must be one of high or low".to_string()),
    }
}

// cargo test scratchpads::scratchpad_utils
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_calculate_image_tokens_by_dimensions_openai() {
        let width = 1024;
        let height = 1024;
        let expected_tokens = 765;
        let tokens = calculate_image_tokens_by_dimensions_openai(width, height);
        assert_eq!(tokens, expected_tokens, "Expected {} tokens, but got {}", expected_tokens, tokens);
    }
}
