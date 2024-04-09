use async_trait::async_trait;
use regex::Regex;
use serde_json::json;
use tokio::sync::Mutex as AMutex;
use tracing::info;
use std::sync::Arc;

use crate::at_commands::at_commands::{AtCommand, AtCommandsContext, AtParam};
use crate::at_commands::utils::split_file_into_chunks_from_line_inside;
use crate::files_in_workspace::get_file_text_from_memory_or_disk;
use crate::call_validation::{ChatMessage, ContextFile};


pub struct AtFile {
    pub name: String,
    pub params: Vec<Arc<AMutex<dyn AtParam>>>,
}

impl AtFile {
    pub fn new() -> Self {
        AtFile {
            name: "@file".to_string(),
            params: vec![
                Arc::new(AMutex::new(AtParamFilePath::new()))
            ],
        }
    }
}

#[derive(Debug, PartialEq)]
pub enum RangeKind {
    GradToCursorTwosided,
    GradToCursorPrefix,
    GradToCursorSuffix,
    Range,
}

#[derive(Debug, PartialEq)]
pub struct ColonLinesRange {
    pub kind: RangeKind,
    pub line1: usize,
    pub line2: usize,
}

pub fn range_print(range: &ColonLinesRange) -> String {
    match range.kind {
        RangeKind::GradToCursorTwosided => format!("{}", range.line1),
        RangeKind::GradToCursorPrefix => format!("-{}", range.line2),
        RangeKind::GradToCursorSuffix => format!("{}-", range.line1),
        RangeKind::Range => format!("{}-{}", range.line1, range.line2),
    }
}

pub fn colon_lines_range_from_arg(value: &mut String) -> Option<ColonLinesRange> {
    let re_hyphen = Regex::new(r":(\d+)?-(\d+)?$").unwrap();
    if let Some(captures) = re_hyphen.captures(value.clone().as_str()) {
        *value = re_hyphen.replace(value, "").to_string();
        return match (captures.get(1), captures.get(2)) {
            (Some(line1), Some(line2)) => {
                let line1 = line1.as_str().parse::<usize>().unwrap_or(0);
                let line2 = line2.as_str().parse::<usize>().unwrap_or(0);
                Some(ColonLinesRange { kind: RangeKind::Range, line1, line2 })
            },
            (Some(line1), None) => {
                let line1 = line1.as_str().parse::<usize>().unwrap_or(0);
                Some(ColonLinesRange { kind: RangeKind::GradToCursorSuffix, line1, line2: 0 })
            },
            (None, Some(line2)) => {
                let line2 = line2.as_str().parse::<usize>().unwrap_or(0);
                Some(ColonLinesRange { kind: RangeKind::GradToCursorPrefix, line1: 0, line2 })
            },
            _ => None,
        }
    }
    let re_one_number = Regex::new(r":(\d+)$").unwrap();
    if let Some(captures) = re_one_number.captures(value.clone().as_str()) {
        *value = re_one_number.replace(value, "").to_string();
        if let Some(line1) = captures.get(1) {
            let line = line1.as_str().parse::<usize>().unwrap_or(0);
            return Some(ColonLinesRange { kind: RangeKind::GradToCursorTwosided, line1: line, line2: 0 });
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_file_range() {
        {
            let mut value = String::from(":10-20");
            let result = colon_lines_range_from_arg(&mut value);
            assert_eq!(result, Some(ColonLinesRange { kind: RangeKind::Range, line1: 10, line2: 20 }));
        }
        {
            let mut value = String::from(":5-");
            let result = colon_lines_range_from_arg(&mut value);
            assert_eq!(result, Some(ColonLinesRange { kind: RangeKind::GradToCursorSuffix, line1: 5, line2: 0 }));
        }
        {
            let mut value = String::from(":-15");
            let result = colon_lines_range_from_arg(&mut value);
            assert_eq!(result, Some(ColonLinesRange { kind: RangeKind::GradToCursorPrefix, line1: 0, line2: 15 }));
        }
        {
            let mut value = String::from(":25");
            let result = colon_lines_range_from_arg(&mut value);
            assert_eq!(result, Some(ColonLinesRange { kind: RangeKind::GradToCursorTwosided, line1: 25, line2: 0 }));
        }
        {
            let mut value = String::from("invalid");
            let result = colon_lines_range_from_arg(&mut value);
            assert_eq!(result, None);

        }
    }
}

fn chunks_into_context_file(
    result_above: Vec<((usize, usize), String)>,
    results_below: Vec<((usize, usize), String)>,
    file_name: &String,
) -> Vec<ContextFile> {
    let max_val = result_above.len().max(results_below.len());
    let mut usefulness_vec = vec![];
    for idx in 0..max_val + 1 {
        usefulness_vec.push(100.0 * (idx as f32 / max_val as f32));
    }
    let reversed_vec: Vec<f32> = usefulness_vec.iter().cloned().rev().collect();
    let usefulness_above: Vec<f32> = reversed_vec[..result_above.len()].to_vec().iter().cloned().rev().collect();

    let mut vector_of_context_file: Vec<ContextFile> = vec![];
    for (idx, ((line1, line2), text_above)) in result_above.iter().enumerate() {
        vector_of_context_file.push({
            ContextFile {
                file_name: file_name.clone(),
                file_content: text_above.clone(),
                line1: *line1,
                line2: *line2,
                symbol: "".to_string(),
                usefulness: *usefulness_above.get(idx).unwrap_or(&100.),
            }
        })
    }

    let usefulness_below = reversed_vec[1..].to_vec();
    for (idx, ((line1, line2), text_below)) in results_below.iter().enumerate() {
        vector_of_context_file.push({
            ContextFile {
                file_name: file_name.clone(),
                file_content: text_below.clone(),
                line1: *line1,
                line2: *line2,
                symbol: "".to_string(),
                usefulness: *usefulness_below.get(idx).unwrap_or(&0.0),
            }
        })
    }
    vector_of_context_file
}

fn put_colon_back_to_arg(value: &mut String, colon: &Option<ColonLinesRange>) {
    if let Some(colon) = colon {
        value.push_str(":");
        value.push_str(range_print(colon).as_str());
    }
}

async fn parameter_repair_candidates(
    value: &String,
    context: &AtCommandsContext,
    top_n: usize
) -> Vec<String>
{
    let mut correction_candidate = value.clone();
    let colon_mb = colon_lines_range_from_arg(&mut correction_candidate);

    let fuzzy = true;
    let result: Vec<String> = crate::files_in_workspace::correct_to_nearest_filename(
        context.global_context.clone(),
        &correction_candidate,
        fuzzy,
        top_n,
    ).await;

    return result.iter().map(|x| {
        let mut x = x.clone();
        put_colon_back_to_arg(&mut x, &colon_mb);
        x
    }).collect();
}

#[derive(Debug)]
pub struct AtParamFilePath {
    pub name: String,
}

impl AtParamFilePath {
    pub fn new() -> Self {
        Self {
            name: "file_path".to_string()
        }
    }
}

#[async_trait]
impl AtParam for AtParamFilePath {
    fn name(&self) -> &String {
        &self.name
    }

    async fn is_value_valid(&self, value: &String, context: &AtCommandsContext) -> bool {
        let mut value = value.clone();
        colon_lines_range_from_arg(&mut value);
        let (cache_correction_arc, _cache_fuzzy_arc) = crate::files_in_workspace::files_cache_rebuild_as_needed(context.global_context.clone()).await;
        // it's dangerous to use cache_correction_arc without a mutex, but should be fine as long as it's read-only
        // (another thread never writes to the map itself, it can only replace the arc with a different map)
        if (*cache_correction_arc).contains_key(&value) {
            info!("@file found {:?} in cache_correction", value);
            return true;
        }
        info!("@file not found {:?} in cache_correction", value);
        false
    }

    async fn complete(&self, value: &String, context: &AtCommandsContext, top_n: usize) -> Vec<String> {
        return parameter_repair_candidates(value, context, top_n).await;
    }
}

#[async_trait]
impl AtCommand for AtFile {
    fn name(&self) -> &String {
        &self.name
    }
    fn params(&self) -> &Vec<Arc<AMutex<dyn AtParam>>> {
        &self.params
    }

    async fn can_execute(&self, args: &Vec<String>, _context: &AtCommandsContext) -> bool {
        args.len() == 1
        // let param = self.params.get(0).unwrap();
        // if let Some(arg) = args.get(0) {
        //     let mut arg_clone = arg.clone();
        //     colon_lines_range_from_arg(&mut arg_clone);
        //     if param.lock().await.is_value_valid(&arg_clone, context).await { // FIXME: is_value_valid is @file-specific, move here
        //         return true;
        //     }
        // }
        // false
    }

    async fn execute(&self, _query: &String, args: &Vec<String>, top_n: usize, context: &AtCommandsContext) -> Result<Vec<ContextFile>, String> {
        let can_execute = self.can_execute(args, context).await;
        if !can_execute {
            return Err("incorrect arguments".to_string());
        }
        let correctable_file_path = args[0].clone();
        let candidates = parameter_repair_candidates(&correctable_file_path, context, top_n).await;
        if candidates.len() == 0 {
            info!("parameter {:?} is uncorrectable :/", &correctable_file_path);
            return Err(format!("parameter {:?} is uncorrectable :/", &correctable_file_path));
        }
        let mut file_path = candidates[0].clone();

        let mut split_into_chunks = false;
        let mut cursor = 0;
        let mut line1 = 0;
        let mut line2 = 0;

        let colon = match colon_lines_range_from_arg(&mut file_path) {
            Some(x) => {
                info!("@file range: {:?}", x);
                if x.kind == RangeKind::GradToCursorTwosided {
                    split_into_chunks = true;
                    cursor = x.line1;
                }
                if x.kind == RangeKind::GradToCursorPrefix {
                    split_into_chunks = true;
                    cursor = x.line2;
                }
                if x.kind == RangeKind::GradToCursorSuffix {
                    split_into_chunks = true;
                    cursor = x.line1;
                }
                if x.kind == RangeKind::Range {
                    line1 = x.line1;
                    line2 = x.line2;
                }
                x
            },
            None => {
                split_into_chunks = true;
                cursor = 0;
                ColonLinesRange { kind: RangeKind::GradToCursorSuffix, line1: 0, line2: 0 }  // not used if split_into_chunks is true
            }
        };
        info!("@file {:?} execute range {:?}", file_path, colon);

        let cpath = crate::files_in_workspace::canonical_path(&file_path);
        let mut file_text = get_file_text_from_memory_or_disk(context.global_context.clone(), &cpath).await?;
        let mut file_lines: Vec<String> = file_text.lines().map(String::from).collect();
        let lines_cnt = file_lines.len();

        if split_into_chunks {
            cursor = cursor.max(0).min(lines_cnt);
            let (mut res_above, mut res_below) = split_file_into_chunks_from_line_inside(cursor, &mut file_lines, 20);
            info!("split_into_chunks cursor: {} <= {}", cursor, lines_cnt);
            if colon.kind == RangeKind::GradToCursorPrefix {
                res_below.clear();
            }
            if colon.kind == RangeKind::GradToCursorSuffix {
                res_above.clear();
            }
            for ((line1, line2), _text) in res_above.iter() {
                info!("above: {}-{}", line1, line2);
            }
            for ((line1, line2), _text) in res_below.iter() {
                info!("below: {}-{}", line1, line2);
            }
            return Ok(chunks_into_context_file(res_above, res_below, &file_path))
        }

        if line1 == 0 || line2 == 0 {
            return Err(format!("{} incorrect range: {}-{}", file_path, colon.line1, colon.line2));
        }
        line1 = (line1 - 1).max(0).min(lines_cnt);
        line2 = line2.max(0).min(lines_cnt);
        let lines: Vec<&str> = file_text.lines().collect();
        file_text = lines[line1 .. line2].join("\n");

        let mut vector_of_context_file: Vec<ContextFile> = vec![];
        vector_of_context_file.push(ContextFile {
            file_name: file_path.clone(),
            file_content: file_text,
            line1: line1 + 1,
            line2: line2,
            symbol: "".to_string(),
            usefulness: 100.0,
        });
        Ok(vector_of_context_file) 
    }
}

