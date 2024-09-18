use std::sync::Arc;
use std::collections::HashMap;
use tokio::sync::RwLock as ARwLock;
use tokio::fs::{File, read_to_string, write};
use tokio::io::AsyncWriteExt;
use sha2::{Sha256, Digest};
use serde_yaml;
use std::path::Path;
use tracing::{info, warn};
use crate::global_context::GlobalContext;
use crate::yaml_configs::yaml_configs_compiled_in::{COMPILED_IN_INITIAL_BYOK, COMPILED_IN_INITIAL_INTEGRATIONS, COMPILED_IN_INITIAL_PRIVACY_YAML, COMPILED_IN_INITIAL_USER_YAML};


const DEFAULT_CHECKSUM_FILE: &str = ".yaml_configs_checksums.yaml";


pub async fn yaml_configs_try_create_all(gcx: Arc<ARwLock<GlobalContext>>) {
    info!("verifying yaml configs...");
    let files = vec![
        ("bring-your-own-key.yaml", COMPILED_IN_INITIAL_BYOK),
        ("customization.yaml", COMPILED_IN_INITIAL_USER_YAML),
        ("privacy.yaml", COMPILED_IN_INITIAL_PRIVACY_YAML),
        ("integrations.yaml", COMPILED_IN_INITIAL_INTEGRATIONS),
    ];
    for (file_name, content) in files {
        if let Err(e) = yaml_file_exists_or_create(gcx.clone(), file_name, content).await {
            warn!("{}", e);
        }
    }
}


async fn yaml_file_exists_or_create(gcx: Arc<ARwLock<GlobalContext>>, config_name: &str, compiled_in: &str) -> Result<(), String> {
    let cache_dir = gcx.read().await.cache_dir.clone();
    let config_path = cache_dir.join(config_name);

    let checksum_from_yaml = read_checksums(&cache_dir).await?
        .get(config_name).cloned().unwrap_or("".to_string());
    let checksum_complied_in = calculate_checksum(compiled_in);

    if config_path.is_file() {
        let existing_content = read_to_string(&config_path).await
            .map_err(|e| format!("failed to read {}: {}", config_name, e))?;
        if existing_content == compiled_in {
            return Ok(());
        }
        let checksum_existing = calculate_checksum(&existing_content);
        if checksum_existing == checksum_from_yaml {
            info!("\n * * * detected that {} is a default config from a previous version of this binary, no changes made by human, overwrite * * *\n", config_path.display());
        } else {
            return Ok(());
        }
    }

    let mut f = File::create(&config_path).await
        .map_err(|e| format!("failed to create {}: {}", config_name, e))?;
    f.write_all(compiled_in.as_bytes()).await
        .map_err(|e| format!("failed to write into {}: {}", config_name, e))?;
    info!("created {}", config_path.display());
    update_checksum(&cache_dir, config_name.to_string(), &checksum_complied_in).await?;
    Ok(())
}

fn calculate_checksum(content: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(content);
    format!("{:x}", hasher.finalize())
}

async fn read_checksums(cache_dir: &Path) -> Result<HashMap<String, String>, String> {
    let checksum_path = cache_dir.join(DEFAULT_CHECKSUM_FILE);
    if checksum_path.exists() {
        let content = tokio::fs::read_to_string(&checksum_path).await
            .map_err(|e| format!("failed to read {}: {}", DEFAULT_CHECKSUM_FILE, e))?;
        let checksums: HashMap<String, String> = serde_yaml::from_str(&content)
            .map_err(|e| format!("failed to parse {}: {}", DEFAULT_CHECKSUM_FILE, e))?;
        Ok(checksums)
    } else {
        Ok(HashMap::new())
    }
}

async fn update_checksum(cache_dir: &Path, config_name: String, checksum: &str) -> Result<(), String> {
    let checksum_path = cache_dir.join(DEFAULT_CHECKSUM_FILE);
    let mut checksums = read_checksums(&cache_dir).await?;
    checksums.insert(config_name.to_string(), checksum.to_string());
    let content = serde_yaml::to_string(&checksums).unwrap();
    write(&checksum_path, content).await
        .map_err(|e| format!("failed to write {}: {}", DEFAULT_CHECKSUM_FILE, e))?;
    Ok(())
}
