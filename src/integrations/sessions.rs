use std::{any::Any, sync::Arc};
use futures::future::join_all;
use tokio::sync::RwLock as ARwLock;

use crate::global_context::GlobalContext;

pub trait IntegrationSession: Any + Send + Sync
{
    fn as_any_mut(&mut self) -> &mut dyn Any;

    fn is_expired(&self) -> bool;
}

pub fn get_session_hashmap_key(integration_name: &str, base_key: &str) -> String {
    format!("{} ⚡ {}", integration_name, base_key)
}

async fn remove_expired_sessions(gcx: Arc<ARwLock<GlobalContext>>) {
    let sessions = {
        let gcx_locked = gcx.read().await;
        gcx_locked.integration_sessions.iter()
            .map(|(key, session)| (key.to_string(), session.clone()))
            .collect::<Vec<_>>()
    };
    
    let expired_keys = join_all(
        sessions.into_iter().map(|(key, session)| async move {
            let session_locked = session.lock().await;
            if session_locked.is_expired() {
                Some(key)
            } else {
                None
            }
        })
    ).await.into_iter().filter_map(|key| key).collect::<Vec<_>>();

    let mut gcx_locked = gcx.write().await;
    for key in expired_keys {
        gcx_locked.integration_sessions.remove(&key);
    }
}

pub async fn remove_expired_sessions_background_task(
    gcx: Arc<ARwLock<GlobalContext>>,
) {
    loop {
        tokio::time::sleep(tokio::time::Duration::from_secs(60)).await;
        remove_expired_sessions(gcx.clone()).await;
    }
}