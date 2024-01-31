use std::collections::{HashMap, HashSet};
use chrono::{NaiveDateTime, Utc};
use tracing::info;

#[derive(Debug, serde::Deserialize, serde::Serialize)]
pub struct RHData {
    pub id: i64,
    pub tenant_name: String,
    pub ts_reported: i64,
    pub ip: String,
    pub enduser_client_version: String,
    pub completions_cnt: i64,
    pub file_extension: String,
    pub human_characters: i64,
    pub model: String,
    pub robot_characters: i64,
    pub teletype: String,
    pub ts_start: i64,
    pub ts_end: i64,
}

#[derive(Debug, Clone)]
struct RHTableStatsByLang {
    lang: String,
    refact: i64,
    human: i64,
    total: i64,
    refact_impact: f32,
    completions: i64,
}

fn robot_human_ratio(robot: i64, human: i64) -> f32 {
    if human == 0 {
        return 1.0;
    }
    if robot == 0 {
        return 0.0;
    }
    // in older versions of refact LSP negative values of human metric existed
    if robot + human == 0 {
        return 0.0;
    }
    return robot as f32 / (robot + human) as f32;
}

async fn table_stats_by_lang(records: &Vec<RHData>) -> Vec<RHTableStatsByLang> {
    let mut lang2stats: HashMap<String, RHTableStatsByLang> = HashMap::new();

    for r in records.iter() {
        let lang = r.file_extension.clone();
        let stats = lang2stats.entry(lang.clone()).or_insert(RHTableStatsByLang {
            lang: lang.clone(),
            refact: 0,
            human: 0,
            total: 0,
            refact_impact: 0.0,
            completions: 0,
        });
        stats.refact += r.robot_characters;
        stats.total += r.robot_characters + r.human_characters;
        stats.human += r.human_characters;
        stats.completions += r.completions_cnt;
    }

    for (_, stats) in lang2stats.iter_mut() {
        stats.refact_impact = robot_human_ratio(stats.refact, stats.human);
    }

    let mut lang_stats_records: Vec<RHTableStatsByLang> = lang2stats.iter().map(|(_, v)| v.clone()).collect();
    lang_stats_records.sort_by(|a, b| b.total.cmp(&a.total));
    lang_stats_records
}

pub async fn records2df(records: &mut Vec<RHData>) {
    records.sort_by(|a, b| a.ts_end.cmp(&b.ts_end));
    let records_lang = table_stats_by_lang(records).await;
    info!("records_lang: {:?}", records_lang);

    // let dt_end_vec: Vec<i64> = records.iter().map(|x| x.ts_end).collect();


    // let dt_end_to_fmt: HashSet<(i64, String)> = dt_end_vec
    //     .iter().map(|x|
    //         (x, chrono::DateTime::<Utc>::from_timestamp(x.clone(), 0).unwrap().format("%b %d").to_string()))
    //     .collect();
}