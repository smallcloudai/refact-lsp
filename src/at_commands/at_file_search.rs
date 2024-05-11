use std::ffi::OsStr;
use std::path::PathBuf;
use async_trait::async_trait;
use tokio::sync::Mutex as AMutex;
use tracing::info;
use std::sync::Arc;

use crate::at_commands::at_commands::{AtCommand, AtCommandsContext, AtParam};
use crate::at_commands::at_file::{AtParamFilePath, parameter_repair_candidates};
use crate::at_commands::at_workspace::vdb_search_results2context_file;
use crate::call_validation::ContextFile;
use crate::vecdb::structs::VecdbSearch;


pub struct AtFileSearch {
    pub name: String,
    pub params: Vec<Arc<AMutex<dyn AtParam>>>,
}

impl AtFileSearch {
    pub fn new() -> Self {
        AtFileSearch {
            name: "@file-search".to_string(),
            params: vec![
                Arc::new(AMutex::new(AtParamFilePath::new()))
            ],
        }
    }
}

fn text_on_clip(results: &Vec<ContextFile>) -> String {
    let file_paths = results.iter().map(|x| x.file_name.clone()).collect::<Vec<_>>();
    return if let Some(path0) = file_paths.get(0) {
        let path = PathBuf::from(path0);
        let file_name = path.file_name().unwrap_or(OsStr::new(path0)).to_string_lossy();
        format!("(according to {})", file_name)
    } else {
        "".to_string()
    }
}

#[async_trait]
impl AtCommand for AtFileSearch {
    fn name(&self) -> &String {
        &self.name
    }
    fn params(&self) -> &Vec<Arc<AMutex<dyn AtParam>>> {
        &self.params
    }
    async fn execute(&self, query: &String, args: &Vec<String>, top_n: usize, context: &AtCommandsContext) -> Result<(Vec<ContextFile>, String), String> {
        // info!("given query: \n{:?}", query);
        let correctable_file_path = args[0].clone();
        let candidates = parameter_repair_candidates(&correctable_file_path, context, top_n).await;
        if candidates.len() == 0 {
            info!("parameter {:?} is uncorrectable :/", &correctable_file_path);
            return Err(format!("parameter {:?} is uncorrectable :/", &correctable_file_path));
        }
        let file_path = candidates[0].clone();
        let filter = format!("(file_path = \"{}\")", file_path);
        
        let search_results = match *context.global_context.read().await.vec_db.lock().await {
            Some(ref db) => {
                let mut db_query = args.join(" ");
                if db_query.is_empty() {
                    db_query = query.clone();
                }
                let search_result = db.vecdb_search(db_query, top_n, Some(filter)).await?;
                let results = search_result.results.clone();
                Ok(vdb_search_results2context_file(&results))
            }
            None => Err("vecdb is not available".to_string())
        }?;
        Ok((search_results.clone(), text_on_clip(&search_results)))
    }
    fn depends_on(&self) -> Vec<String> {
        vec!["vecdb".to_string()]
    }
}
