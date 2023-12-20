use std::collections::HashMap;
use tracing::{error, info};
use std::sync::{Arc, RwLockWriteGuard};
use std::sync::RwLock as StdRwLock;
use std::path::PathBuf;
use serde::{Deserialize, Serialize};
use serde_json::json;
use regex::Regex;

use tokio::sync::RwLock as ARwLock;

use crate::global_context;
use crate::telemetry::utils;
use crate::telemetry::telemetry_structs;
use crate::telemetry::telemetry_structs::{SnippetTracker, TeleRobotHumanAccum};


const ROBOT_HUMAN_FILE_STATS_UPDATE_EVERY: i64 = 15;


fn get_robot_characters(snip: &SnippetTracker) -> i64 {
    let re = Regex::new(r"\s+").unwrap();
    let robot_characters = re.replace_all(&snip.grey_text, "").len() as i64;
    // info!("increase_counters_from_finished_snippet: ID: {}; robot_characters: {}", snip.snippet_telemetry_id, robot_characters);
    robot_characters
}

fn get_human_characters(
    baseline_text: &String,
    text: &String,
    robot_characters_acc_baseline: i64
) -> i64 {
    let re = Regex::new(r"\s+").unwrap();
    let (added_characters, _) = utils::get_add_del_from_texts(baseline_text, text);
    // info!("added_characters: {}", added_characters);
    let human_characters = 0.max(re.replace_all(&added_characters, "").len() as i64 - robot_characters_acc_baseline);
    // info!("human_characters: {}", human_characters);
    human_characters
}

pub fn increase_counters_from_finished_snippet(
    tele_robot_human: &mut Vec<TeleRobotHumanAccum>,
    uri: &String,
    text: &String,
    snip: &SnippetTracker,
    init_file_text: &String,
) {
    let now = chrono::Local::now().timestamp();

    if let Some(rec) = tele_robot_human.iter_mut().find(|stat| stat.uri.eq(uri)) {
        if rec.used_snip_ids.contains(&snip.snippet_telemetry_id) {
            return;
        }
        let robot_characters = get_robot_characters(snip);
        rec.robot_characters_acc_baseline += robot_characters;
        let human_characters = get_human_characters(&rec.baseline_text, text, rec.robot_characters_acc_baseline);
        rec.used_snip_ids.push(snip.snippet_telemetry_id);
        if rec.baseline_updated_ts + ROBOT_HUMAN_FILE_STATS_UPDATE_EVERY < now {
            // New baseline, increase counters
            rec.baseline_updated_ts = now;
            rec.human_characters += human_characters;
            rec.robot_characters += rec.robot_characters_acc_baseline;
            rec.robot_characters_acc_baseline = 0;
            rec.baseline_text = text.clone();
        }
        // info!("increasing for {}, human+{}, robot+{}", snip.snippet_telemetry_id, human_characters, robot_characters);
    } else {
        // info!("increase_counters_from_finished_snippet: new uri {}", uri);
        let robot_characters = get_robot_characters(snip);
        let human_characters = get_human_characters(&init_file_text, text, robot_characters);
        tele_robot_human.push(TeleRobotHumanAccum::new(
            uri.clone(),
            snip.model.clone(),
            init_file_text.clone(),
            get_robot_characters(snip),
            human_characters,
            vec![snip.snippet_telemetry_id],
        ));
    }
}

fn compress_robot_human(
    storage_locked: &mut RwLockWriteGuard<telemetry_structs::Storage>
) -> Vec<TeleRobotHuman> {
    let mut unique_combinations: HashMap<(String, String), Vec<TeleRobotHumanAccum>> = HashMap::new();

    let tele_robot_human = storage_locked.tele_robot_human.clone();

    for accum in tele_robot_human {
        let key = (accum.file_extension.clone(), accum.model.clone());
        unique_combinations.entry(key).or_default().push(accum);
    }
    let mut compressed_vec= vec![];
    for (key, entries) in unique_combinations {
        // info!("compress_robot_human: compressing {} entries for key {:?}", entries.len(), key);
        let mut record = TeleRobotHuman::new(
            key.0.clone(),
            key.1.clone()
        );
        for entry in entries {
            let progress_file_text_mb= storage_locked.progress_file_texts.iter().find(|s| s.uri == entry.uri);
            if let Some(progress_file_text) = progress_file_text_mb {
                let human_characters = get_human_characters(
                    &entry.baseline_text,
                    &progress_file_text.file_text,
                    entry.robot_characters_acc_baseline
                );
                // info!("entry_baseline_text: {}", entry.baseline_text);
                // info!("progress_file_text: {}", progress_file_text.file_text);
                // info!("compress_robot_human: human_characters: {}", human_characters);
                record.human_characters += human_characters;
            }
            record.human_characters += entry.human_characters;
            record.robot_characters += entry.robot_characters + entry.robot_characters_acc_baseline;
            record.completions_cnt += entry.used_snip_ids.len() as i64;
        }
        compressed_vec.push(record);
    }
    compressed_vec
}

pub async fn tele_robot_human_compress_to_file(
    cx: Arc<ARwLock<global_context::GlobalContext>>,
) {
    let now = chrono::Local::now();
    let cache_dir: PathBuf;
    let storage: Arc<StdRwLock<telemetry_structs::Storage>>;
    let enduser_client_version;
    let mut records = json!([]);
    {
        let cx_locked = cx.read().await;
        storage = cx_locked.telemetry.clone();
        cache_dir = cx_locked.cache_dir.clone();
        enduser_client_version = cx_locked.cmdline.enduser_client_version.clone();

        let mut storage_locked = storage.write().unwrap();
        for rec in compress_robot_human(&mut storage_locked) {
            let json_dict = serde_json::to_value(rec).unwrap();
            records.as_array_mut().unwrap().push(json_dict);
        }
        storage_locked.tele_robot_human.clear();
    }
    if records.as_array().unwrap().is_empty() {
        // info!("no robot_human telemetry to save");
        return;
    }
    let (dir, _) = utils::telemetry_storage_dirs(&cache_dir).await;

    let fn_rh = dir.join(format!("{}-rh.json", now.format("%Y%m%d-%H%M%S")));
    let big_json_rh = json!({
        "records": records,
        "ts_start": now.timestamp(),
        "ts_end": now.timestamp(),
        "teletype": "robot_human",
        "enduser_client_version": enduser_client_version,
    });
    // info!("robot_human telemetry save \"{}\"", fn_rh.to_str().unwrap());
    let io_result = utils::file_save(fn_rh.clone(), big_json_rh).await;
    if io_result.is_err() {
        error!("error: {}", io_result.err().unwrap());
    }
}

#[derive(Debug, Serialize, Deserialize, Clone, Default)]
struct TeleRobotHuman {
    file_extension: String,
    model: String,

    human_characters: i64,
    robot_characters: i64,
    completions_cnt: i64,
}

impl TeleRobotHuman {
    fn new(
        file_extension: String, model: String
    ) -> Self {
        Self {
            file_extension,
            model,

            human_characters: 0,
            robot_characters: 0,
            completions_cnt: 0
        }
    }
}