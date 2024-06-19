use std::sync::Arc;
use crate::global_context::GlobalContext;
use tokio::sync::{RwLock as ARwLock};
use std::fs;
use std::io::BufReader;
use std::path::PathBuf;
use log::{error, info, warn};
use crate::at_tools::att_doc_sources::DocOrigin;
use crate::files_in_workspace::Document;

pub async fn enqueue_all_files_from_workspace_folders(gcx: Arc<ARwLock<GlobalContext>>) {
    let Ok(paths) = fs::read_dir("./.refact/docs") else {
        warn!("No ./.refact/docs directory");
        return;
    };

    for path in paths {
        let Ok(path) = path else {
            continue;
        };


        let mut path = path.path();
        path.push("origin.json");
        let Ok(file) = fs::File::open(path.clone()) else {
            continue;
        };

        let reader = BufReader::new(file);
        let Some(doc_origin): Option<DocOrigin> = serde_json::from_reader(reader).ok() else {
            error!("Unable to parse {}", path.display());
            continue;
        };

        for file_path in doc_origin.pages.values() {
            let gcx = gcx.write().await;
            let mut files = gcx.documents_state.documentation_files.lock().await;
            let vec_db_module = {
                *gcx.documents_state.cache_dirty.lock().await = true;
                gcx.vec_db.clone()
            };
            if !files.contains(&file_path) {
                files.push(file_path.clone());
            }
            let document = Document::new(&PathBuf::from(file_path.as_str()));
            match *vec_db_module.lock().await {
                Some(ref mut db) => db.vectorizer_enqueue_files(&vec![document], false).await,
                None => {}
            };
        }
    }
}