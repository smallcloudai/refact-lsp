use std::collections::HashSet;
use crate::global_context::GlobalContext;
use crate::vecdb::vdb_highlev::{memories_add, memories_block_until_vectorized, memories_select_all};
use serde_json::Value;
use std::sync::Arc;
use tokio::sync::RwLock as ARwLock;
use tracing::info;
use chrono::{NaiveDateTime, Utc};

static URL: &str = "https://www.smallcloud.ai/v1/trajectory-get-all";
static TRAJECTORIES_STATUS_FILENAME: &str = "trajectories_last_update";
static TRAJECTORIES_UPDATE_EACH_N_DAYS: i64 = 7;


async fn save_last_download_time(gcx: Arc<ARwLock<GlobalContext>>) -> Result<(), String> {
    let cache_dir = gcx.read().await.cache_dir.clone();
    let now = Utc::now().naive_utc();
    let now_str = now.format("%Y-%m-%d %H:%M:%S").to_string();
    let file_path = cache_dir.join(TRAJECTORIES_STATUS_FILENAME);
    tokio::fs::write(file_path, now_str).await.map_err(|x| x.to_string())
}

async fn is_time_to_download_trajectories(gcx: Arc<ARwLock<GlobalContext>>) -> Result<bool, String> {
    let cache_dir = gcx.read().await.cache_dir.clone();
    let file_path = cache_dir.join(TRAJECTORIES_STATUS_FILENAME);
    let last_download_time = match tokio::fs::read_to_string(file_path).await {
        Ok(time_str) => {
            NaiveDateTime::parse_from_str(&time_str, "%Y-%m-%d %H:%M:%S")
                .map_err(|x| x.to_string())?
        }
        Err(_) => {
            return Ok(true);
        }
    };
    let now = Utc::now().naive_utc();
    let duration_since_last_download = now.signed_duration_since(last_download_time);
    Ok(duration_since_last_download.num_days() >= TRAJECTORIES_UPDATE_EACH_N_DAYS)
}

pub async fn try_to_download_trajectories(gcx: Arc<ARwLock<GlobalContext>>) -> Result<(), String> {
    if !is_time_to_download_trajectories(gcx.clone()).await? {
        return Ok(());
    }
    
    let (vec_db, api_key) = {
        let gcx_locked = gcx.read().await;
        (
            gcx_locked.vec_db.clone(),
            gcx_locked.cmdline.api_key.clone(),
        )
    };
    if vec_db.lock().await.is_none() {
        info!("VecDb is not initialized");
        return Ok(());
    }
    memories_block_until_vectorized(vec_db.clone(), 20_000).await?;

    info!("starting to download trajectories...");
    let client = reqwest::Client::new();
    let response = client
        .get(URL)
        .header("Authorization", format!("Bearer {}", api_key))
        .send()
        .await
        .map_err(|err| err.to_string())?;
    let response_json: Value = response.json().await.map_err(|err| err.to_string())?;

    if response_json["retcode"] != "OK" {
        info!("failed to download trajectories: {:?}", response_json);
        return Ok(());
    }

    let trajectories = response_json["data"].as_array().unwrap();
    let existing_trajectories = memories_select_all(vec_db.clone())
        .await?
        .iter()
        .map(|x| x.memid.clone())
        .collect::<HashSet<String>>();
    for trajectory in trajectories {
        let m_memid = trajectory["memid"].as_str().ok_or("Failed to get memid")?;
        if existing_trajectories.contains(m_memid) {
            info!("trajectory {} already exists in the vecdb", m_memid);            
            continue;
        }
        
        let m_type = trajectory["kind"].as_str().unwrap_or("unknown");
        let m_goal = trajectory["goal"].as_str().unwrap_or("unknown");
        let m_project = trajectory["framework"].as_str().unwrap_or("unknown");
        let m_payload = trajectory["payload"].as_str().unwrap_or("");
        if m_payload.is_empty() {
            info!("empty or no payload for the trajectory: {}, skipping it", m_memid);
            continue;            
        }
        match memories_add(
            vec_db.clone(),
            m_type,
            m_goal,
            m_project,
            m_payload,
            Some(m_memid.to_string()),
        ).await {
            Ok(memid) => info!("Memory added with ID: {}", memid),
            Err(err) => info!("Failed to add memory: {}", err),
        }
        info!(
            "downloaded trajectory: memid={}, type={}, goal={}, project={}, payload={}",
            m_memid,
            m_type,
            m_goal,
            m_project,
            crate::nicer_logs::first_n_chars(&m_payload.to_string(), 100)
        );
    }

    info!("finished downloading trajectories");
    save_last_download_time(gcx.clone()).await?;
    Ok(())
}
