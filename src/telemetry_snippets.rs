use tracing::{error, info};
use std::sync::Arc;
use tokio::sync::RwLock as ARwLock;
use std::sync::RwLock as StdRwLock;

use crate::call_validation;
use serde::Deserialize;
use serde::Serialize;
use serde_json::json;
use crate::global_context;
use crate::completion_cache;
use crate::telemetry_storage;
use crate::call_validation::CodeCompletionPost;
use similar::{ChangeTag, TextDiff};


// How it works:
// 1. Rust returns {"snippet_telemetry_id":101,"choices":[{"code_completion":"\n    return \"Hello World!\"\n"}] ...}
// ?. IDE detects accept, sends /v1/completion-accepted with {"snippet_telemetry_id":101}
// 3. LSP looks at file changes (LSP can be replaced with reaction to a next completion?)
// 4. Changes are translated to "after_walkaway_remaining50to95" etc

const SNIP_FINISHED_AFTER : i64 = 240;
const SNIP_TIMEOUT_AFTER : i64 = 60;


#[derive(Debug, Clone)]
pub struct SaveSnippet {
    pub storage_arc: Arc<StdRwLock<telemetry_storage::Storage>>,
    pub post: CodeCompletionPost,
}

impl SaveSnippet {
    pub fn new(
        storage_arc: Arc<StdRwLock<telemetry_storage::Storage>>,
        post: &CodeCompletionPost
    ) -> Self {
        SaveSnippet {
            storage_arc,
            post: post.clone(),
        }
    }
}


#[derive(Debug, Serialize, Deserialize, Clone, Default)]
pub struct SnippetTelemetry {
    pub snippet_telemetry_id: u64,
    pub inputs: call_validation::CodeCompletionInputs,
    pub grey_text: String,
    pub accepted: bool,

    pub corrected_text_30s: String,
    pub corrected_text_90s: String,
    pub corrected_text_180s: String,
    pub remaining_percent_30s: f64,
    pub remaining_percent_90s: f64,
    pub remaining_percent_180s: f64,

    // pub remaining_percent_walkaway: f64,
    // pub walkaway_ms: u64,
    pub created_ts: i64,
    pub accepted_ts: i64,
}

pub fn snippet_register(
    ss: &SaveSnippet,
    grey_text: String,
) -> u64 {
    let mut storage_locked = ss.storage_arc.write().unwrap();
    let snippet_telemetry_id = storage_locked.tele_snippet_next_id;
    let snip = SnippetTelemetry {
        snippet_telemetry_id,
        inputs: ss.post.inputs.clone(),
        grey_text: grey_text.clone(),
        accepted: false,
        corrected_text_30s: "".to_string(),
        corrected_text_90s: "".to_string(),
        corrected_text_180s: "".to_string(),
        remaining_percent_30s: -1.,
        remaining_percent_90s: -1.,
        remaining_percent_180s: -1.,
        created_ts: chrono::Local::now().timestamp(),
        accepted_ts: 0,
    };
    storage_locked.tele_snippet_next_id += 1;
    storage_locked.tele_snippets.push(snip);
    snippet_telemetry_id
}

pub fn snippet_register_from_data4cache(
    ss: &SaveSnippet,
    data4cache: &mut completion_cache::CompletionSaveToCache,
) {
    // Convenience function: snippet_telemetry_id should be returned inside a cached answer as well, so there's
    // typically a combination of the two
    if data4cache.completion0_finish_reason.is_empty() {
        return;
    }
    data4cache.completion0_snippet_telemetry_id = Some(snippet_register(&ss, data4cache.completion0_text.clone()));
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct SnippetAccepted {
    pub snippet_telemetry_id: u64,
}

pub async fn snippet_accepted(
    gcx: Arc<ARwLock<global_context::GlobalContext>>,
    snippet_telemetry_id: u64,
) -> bool {
    let tele_storage_arc = gcx.read().await.telemetry.clone();
    let mut storage_locked = tele_storage_arc.write().unwrap();
    let snip = storage_locked.tele_snippets.iter_mut().find(|s| s.snippet_telemetry_id == snippet_telemetry_id);
    if let Some(snip) = snip {
        snip.accepted = true;
        snip.accepted_ts = chrono::Local::now().timestamp();
        return true;
    }
    return false;
}

pub async fn sources_changed(
    gcx: Arc<ARwLock<global_context::GlobalContext>>,
    uri: &String,
    text: &String,
) {
    info!("sources_changed: uri: {:?}, text: {:?}", uri, text);
    let tele_storage = gcx.read().await.telemetry.clone();
    let mut storage_locked = tele_storage.write().unwrap();
    for snip in &mut storage_locked.tele_snippets {
        if !snip.accepted {
            continue;
        }
        if !uri.ends_with(&snip.inputs.cursor.file) {
            continue;
        }
        let orig_text = snip.inputs.sources.get(&snip.inputs.cursor.file);
        if !orig_text.is_some() {
            continue;
        }
        let time_from_accepted = chrono::Local::now().timestamp() - snip.accepted_ts;

        if time_from_accepted < 30 ||
            (time_from_accepted >= 30 && time_from_accepted < 90 && snip.remaining_percent_30s >= 0.) ||
            (time_from_accepted >= 90 && time_from_accepted < 180 && snip.remaining_percent_90s >= 0.) ||
            (time_from_accepted >= 180 && snip.remaining_percent_180s >= 0.){
            continue;
        }
        info!("snip id {}; time_from_accepted {}", snip.snippet_telemetry_id, time_from_accepted);
        let snip_unchanged_percentage = unchanged_percentage(orig_text.unwrap(), text, &snip.grey_text);
        info!("snip_unchanged_percentage: {}", snip_unchanged_percentage);
        if time_from_accepted >= 30 && time_from_accepted < 90 {
            snip.corrected_text_30s = text.clone();
            snip.remaining_percent_30s = snip_unchanged_percentage;
        }
        else if time_from_accepted >= 90 && time_from_accepted < 180 {
            snip.corrected_text_90s = text.clone();
            snip.remaining_percent_90s = snip_unchanged_percentage;
        }
        else if time_from_accepted >= 180 {
            snip.corrected_text_180s = text.clone();
            snip.remaining_percent_180s = snip_unchanged_percentage;
        }
    }
}

fn get_add_del_from_texts(
    text_a: &String,
    text_b: &String,
) -> (String, String) {
    let diff = TextDiff::from_lines(text_a, text_b);
    let mut added = "".to_string();
    let mut removed = "".to_string();
    for change in diff.iter_all_changes() {
        match change.tag() {
            ChangeTag::Delete => {
                removed += change.value();
            }
            ChangeTag::Insert => {
                added += change.value();
            }
            ChangeTag::Equal => {
            }
        }
    }
    (added, removed)
}

fn unchanged_percentage(
    text_a: &String,
    text_b: &String,
    grey_text_a: &String,
) -> f64 {
    if text_b.contains(grey_text_a) {
        // info!("text_b contains grey_text_a");
        return 1.;
    }
    let (texts_ab_added, _) = get_add_del_from_texts(text_a, text_b);
    let (_, removed) = get_add_del_from_texts(&texts_ab_added, grey_text_a);

    if removed.is_empty() {
        // info!("removed is empty");
        return 1.;
    }

    fn common_syms_string(a: &String, b: &String) -> f64 {
        let diff = TextDiff::from_chars(a, b);
        let mut common = 0;
        for change in diff.iter_all_changes() {
            match change.tag() {
                ChangeTag::Delete => {
                }
                ChangeTag::Insert => {
                }
                ChangeTag::Equal => {
                    common += 1;
                }
            }
        }
        common as f64
    }

    if !grey_text_a.contains("\n") {
        let common = common_syms_string(&removed, grey_text_a);
        let unchanged_percentage = common / grey_text_a.len() as f64;
        unchanged_percentage
    } else {
        let mut common = 0.;
        for line in grey_text_a.lines() {
            // info!("checking the line: {:?}", line);
            if text_b.contains(line.trim()) {
                common += line.len() as f64;
                // info!("text_b contains line {}: +{}", line, line.len());
                continue;
            }
            let common_line = common_syms_string(&removed, &line.to_string());
            // info!("common_line {}: +{}", line, common_line);
            common += common_line;
        }
        common / grey_text_a.len() as f64
    }
}

async fn send_finished_snippets(gcx: Arc<ARwLock<global_context::GlobalContext>>) {
    let tele_storage;
    let now = chrono::Local::now().timestamp();
    let enduser_client_version;
    let api_key: String;
    let caps;
    let mothership_enabled: bool;
    let mut telemetry_corrected_snippets_dest = String::new();
    {
        let cx = gcx.read().await;
        enduser_client_version = cx.cmdline.enduser_client_version.clone();
        tele_storage = cx.telemetry.clone();
        api_key = cx.cmdline.api_key.clone();
        caps = cx.caps.clone();
        mothership_enabled = cx.cmdline.snippet_telemetry;
    }
    if let Some(caps) = &caps {
        telemetry_corrected_snippets_dest = caps.read().unwrap().telemetry_corrected_snippets_dest.clone();
    }

    let mut snips_send: Vec<SnippetTelemetry> = vec![];
    {
        let mut to_remove: Vec<usize> = vec![];
        let mut storage_locked = tele_storage.write().unwrap();
        for (idx, snip) in &mut storage_locked.tele_snippets.iter().enumerate() {
            if snip.accepted && snip.accepted_ts != 0 {
                if now - snip.accepted_ts >= SNIP_FINISHED_AFTER {
                    to_remove.push(idx);
                    snips_send.push(snip.clone());
                }
                continue;
            }
            if !snip.accepted && now - snip.created_ts >= SNIP_TIMEOUT_AFTER {
                to_remove.push(idx);
                continue;
            }
        }
        for idx in to_remove.iter().rev() {
            storage_locked.tele_snippets.remove(*idx);
        }
    }

    if !mothership_enabled {
        info!("telemetry snippets sending not enabled, skip");
        return;
    }
    if telemetry_corrected_snippets_dest.is_empty() {
        info!("telemetry_corrected_snippets_dest is empty, skip");
        return;
    }
    if snips_send.is_empty() {
        info!("no snippets to send, skip");
        return;
    }
    info!("sending {} snippets", snips_send.len());

    for snip in snips_send {
        let json_dict = serde_json::to_value(snip).unwrap();
        info!("sending snippet: {:?}", json_dict);
        let big_json_snip = json!({
            "records": [json_dict],
            "ts_start": now,
            "ts_end": chrono::Local::now().timestamp(),
            "teletype": "snippets",
            "enduser_client_version": enduser_client_version,
        });
        let resp_maybe = telemetry_storage::send_telemetry_data(
            big_json_snip.to_string(),
            &telemetry_corrected_snippets_dest,
            &api_key
        ).await;
        if resp_maybe.is_err() {
            error!("snippet send failed: {}", resp_maybe.err().unwrap());
            error!("too bad snippet is lost now");
            continue;
        }
    }
}

pub async fn tele_snip_background_task(
    global_context: Arc<ARwLock<global_context::GlobalContext>>,
) -> () {
    loop {
        tokio::time::sleep(tokio::time::Duration::from_secs(30)).await;
        info!("tele_snip_background_task");
        send_finished_snippets(global_context.clone()).await;
    }
}
