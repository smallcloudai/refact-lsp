use std::collections::{HashMap, HashSet};
use std::path::{Component, PathBuf};
use std::sync::Arc;
use std::time::Instant;
use crate::global_context::GlobalContext;
use tokio::sync::RwLock as ARwLock;
use tracing::info;

pub async fn paths_from_anywhere(global_context: Arc<ARwLock<GlobalContext>>) -> Vec<PathBuf> {
    let file_paths_from_memory = global_context.read().await.documents_state.memory_document_map.keys().map(|x|x.clone()).collect::<Vec<_>>();
    let paths_from_workspace: Vec<PathBuf> = global_context.read().await.documents_state.workspace_files.lock().unwrap().clone();
    let paths_from_jsonl: Vec<PathBuf> = global_context.read().await.documents_state.jsonl_files.lock().unwrap().clone();
    let paths_from_anywhere = file_paths_from_memory.into_iter().chain(paths_from_workspace.into_iter().chain(paths_from_jsonl.into_iter()));
    paths_from_anywhere.collect::<Vec<PathBuf>>()
}

fn make_cache<I>(paths_iter: I, workspace_folders: &Vec<PathBuf>) -> (
    HashMap<String, HashSet<String>>, Vec<(String, String)>, usize
) where I: IntoIterator<Item = PathBuf> {
    let mut cache_correction = HashMap::<String, HashSet<String>>::new();
    let mut cache_fuzzy_set = HashSet::<(String, String)>::new();
    let mut cnt = 0;

    for path in paths_iter {
        let path_str = path.to_str().unwrap_or_default().to_string();
        let filename = path.file_name().unwrap().to_string_lossy().to_string();
        
        // get path in workspace, stripping off everything before workspace root
        let workspace_path = workspace_folders.iter()
            .filter_map(|workspace_folder| {
                let workspace_folder_str = workspace_folder.to_str().unwrap_or_default();
                if path_str.starts_with(workspace_folder_str) {
                    return Some(path_str.strip_prefix(workspace_folder_str).unwrap_or(&path_str).to_string());
                }
                None
            })
            .min_by_key(|s| s.len())
            .unwrap_or(path_str.clone());
        
        cache_fuzzy_set.insert((filename.clone(), workspace_path.clone()));
        cnt += 1;

        cache_correction.entry(path_str.clone()).or_insert_with(HashSet::new).insert(path_str.clone());
        // chop off directory names one by one
        let mut index = 0;
        while let Some(slashpos) = path_str[index .. ].find(|c| c == '/' || c == '\\') {
            let absolute_slashpos = index + slashpos;
            index = absolute_slashpos + 1;
            let slashpos_to_end = &path_str[index .. ];
            if !slashpos_to_end.is_empty() {
                cache_correction.entry(slashpos_to_end.to_string()).or_insert_with(HashSet::new).insert(path_str.clone());
            }
        }
    }

    (cache_correction, cache_fuzzy_set.into_iter().collect::<Vec<_>>(), cnt)
}

pub async fn get_files_in_dir(
    global_context: Arc<ARwLock<GlobalContext>>,
    dir: &PathBuf,
) -> Vec<PathBuf> {
    let paths = paths_from_anywhere(global_context.clone()).await;
    paths.into_iter()
        .filter(|path| path.parent() == Some(dir))
        .collect()
}

pub async fn files_cache_rebuild_as_needed(global_context: Arc<ARwLock<GlobalContext>>) -> (Arc<HashMap<String, HashSet<String>>>, Arc<Vec<(String, String)>>) {
    let (cache_dirty_arc, mut cache_correction_arc, mut cache_fuzzy_arc) = {
        let cx = global_context.read().await;
        (
            cx.documents_state.cache_dirty.clone(),
            cx.documents_state.cache_correction.clone(),
            cx.documents_state.cache_fuzzy.clone(),
        )
    };

    let mut cache_dirty_ref = cache_dirty_arc.lock().await;
    if *cache_dirty_ref {
        info!("rebuilding files cache...");
        // filter only get_project_dirs?
        let start_time = Instant::now();
        let paths_from_anywhere = paths_from_anywhere(global_context.clone()).await;
        let workspace_folders = get_project_dirs(global_context.clone()).await;
        let (cache_correction, cache_fuzzy, cnt) = make_cache(paths_from_anywhere, &workspace_folders);

        info!("rebuild completed in {:.3}s, {} URLs => cache_correction.len is now {}", start_time.elapsed().as_secs_f64(), cnt, cache_correction.len());
        cache_correction_arc = Arc::new(cache_correction);
        cache_fuzzy_arc = Arc::new(cache_fuzzy);
        {
            let mut cx = global_context.write().await;
            cx.documents_state.cache_correction = cache_correction_arc.clone();
            cx.documents_state.cache_fuzzy = cache_fuzzy_arc.clone();
        }
        *cache_dirty_ref = false;
    }

    return (cache_correction_arc, cache_fuzzy_arc);
}

fn normalized_distance(a: &str, b: &str) -> f64 {
    let max_length = std::cmp::max(a.len(), b.len()) as f64;
    if max_length == 0.0 {
        return 0.0;
    }
    sift4::simple(a, b) as f64 / max_length
}

fn fuzzy_search<I>(
    correction_candidate: &String,
    candidates: I,
    top_n: usize,
) -> Vec<String>
where I: IntoIterator<Item = (String, String)> {
    let mut top_n_records = Vec::with_capacity(top_n);
    for p in candidates {
        let correction_candidate_filename = PathBuf::from(&correction_candidate)
            .file_name()
            .unwrap()
            .to_string_lossy()
            .to_string();

        let filename_dist = normalized_distance(&correction_candidate_filename, &p.0);
        let path_dist = normalized_distance(&correction_candidate, &p.1);

        top_n_records.push((p.1.clone(), filename_dist * 2.5 + path_dist));
        if top_n_records.len() >= top_n {
            top_n_records.sort_by(|a, b| a.1.partial_cmp(&b.1).unwrap());
            top_n_records.pop();
        }
    }
    top_n_records.sort_by(|a, b| a.1.partial_cmp(&b.1).unwrap());
    top_n_records.into_iter().map(|x| x.0.clone()).collect()
}

pub async fn correct_to_nearest_filename(
    global_context: Arc<ARwLock<GlobalContext>>,
    correction_candidate: &String,
    fuzzy: bool,
    top_n: usize,
) -> Vec<String> {
    let (cache_correction_arc, cache_fuzzy_arc) = files_cache_rebuild_as_needed(global_context.clone()).await;
    // it's dangerous to use cache_correction_arc without a mutex, but should be fine as long as it's read-only
    // (another thread never writes to the map itself, it can only replace the arc with a different map)

    if let Some(fixed) = (*cache_correction_arc).get(&correction_candidate.clone()) {
        // info!("found {:?} in cache_correction, returning [{:?}]", correction_candidate, fixed);
        return fixed.into_iter().cloned().collect::<Vec<String>>();
    } else {
        info!("not found {} in cache_correction", correction_candidate);
    }

    if fuzzy {
        info!("fuzzy search {:?}, cache_fuzzy_arc.len={}", correction_candidate, cache_fuzzy_arc.len());
        return fuzzy_search(correction_candidate, cache_fuzzy_arc.iter().cloned(), top_n);
    }

    return vec![];
}

pub async fn correct_to_nearest_dir_path(
    gcx: Arc<ARwLock<GlobalContext>>,
    correction_candidate: &String,
    fuzzy: bool,
    top_n: usize,
) -> Vec<String> {
    fn get_parent(p: &String) -> Option<String> {
        PathBuf::from(p).parent().map(PathBuf::from).map(|x|x.to_string_lossy().to_string())
    }

    let (cache_correction_arc, cache_fuzzy_set) = files_cache_rebuild_as_needed(gcx.clone()).await;
    let mut paths_correction_map = HashMap::new();
    for (k, v) in cache_correction_arc.iter() {
        match get_parent(k) {
            Some(k_parent) => {
                let v_parents = v.iter().filter_map(|x| get_parent(x)).collect::<Vec<_>>();
                if v_parents.is_empty() {
                    continue;
                }
                paths_correction_map.entry(k_parent.clone()).or_insert_with(HashSet::new).extend(v_parents);
            },
            None => {}
        }
    }
    if let Some(res) = paths_correction_map.get(correction_candidate).map(|x|x.iter().cloned().collect::<Vec<_>>()) {
        return res;
    }

    if fuzzy {
        return fuzzy_search(correction_candidate, cache_fuzzy_set.iter().cloned(), top_n);
    }
    vec![]
}

pub async fn get_project_dirs(gcx: Arc<ARwLock<GlobalContext>>) -> Vec<PathBuf> {
    let gcx_locked = gcx.write().await;
    let workspace_folders = gcx_locked.documents_state.workspace_folders.lock().unwrap();
    workspace_folders.iter().cloned().collect::<Vec<_>>()
}

pub async fn shortify_paths(gcx: Arc<ARwLock<GlobalContext>>, paths: Vec<String>) -> Vec<String> {
    let (cache_correction_arc, _) = files_cache_rebuild_as_needed(gcx.clone()).await;
    let project_dirs = get_project_dirs(gcx.clone()).await;
    let p_paths_str: Vec<String> = project_dirs.iter()
        .map(|x| x.to_string_lossy().into_owned())
        .collect();

    let mut results = Vec::with_capacity(paths.len());
    for p in paths {
        let matching_proj = p_paths_str.iter().find(|proj| p.starts_with(*proj));
        if let Some(proj) = matching_proj {
            let p_no_base = p.strip_prefix(proj).unwrap_or(&p).trim_start_matches('/');
            if !p_no_base.is_empty() {
                if let Some(candidates) = cache_correction_arc.get(p_no_base) {
                    if candidates.len() == 1 {
                        results.push(p_no_base.to_string());
                        continue;
                    }
                }
            }
        }
        // If we reach here, we couldn't shorten the path unambiguously
        results.push(p);
    }
    results
}

fn absolute(path: &std::path::Path) -> std::io::Result<PathBuf> {
    let mut components = path.strip_prefix(".").unwrap_or(path).components();
    let path_os = path.as_os_str().as_encoded_bytes();
    let mut normalized = if path.is_absolute() {
        if path_os.starts_with(b"//") && !path_os.starts_with(b"///") {
            components.next();
            PathBuf::from("//")
        } else {
            PathBuf::new()
        }
    } else {
        std::env::current_dir()?
    };
    normalized.extend(components);
    if path_os.ends_with(b"/") {
        normalized.push("");
    }
    Ok(normalized)
}

pub fn canonical_path(s: &String) -> PathBuf {
    let mut res = match PathBuf::from(s).canonicalize() {
        Ok(x) => x,
        Err(_) => {
            let a = absolute(std::path::Path::new(s)).unwrap_or(PathBuf::from(s));
            // warn!("canonical_path: {:?} doesn't work: {}\n using absolute path instead {}", s, e, a.display());
            a
        }
    };
    let components: Vec<String> = res
        .components()
        .map(|x| match x {
            Component::Normal(c) => c.to_string_lossy().to_string(),
            Component::Prefix(c) => {
                let lowercase_prefix = c.as_os_str().to_string_lossy().to_string().to_lowercase();
                lowercase_prefix
            },
            _ => x.as_os_str().to_string_lossy().to_string(),
        })
        .collect();
    res = components.iter().fold(PathBuf::new(), |mut acc, x| {
        acc.push(x);
        acc
    });
    // info!("canonical_path:\n{:?}\n{:?}", s, res);
    res
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::files_in_workspace::_retrieve_files_in_workspace_folders;

    async fn get_candidates_from_workspace_files() -> Vec<(String, String)> {
        let proj_folders = vec![PathBuf::from(".").canonicalize().unwrap()];
        let proj_folder = &proj_folders[0];

        let workspace_files = _retrieve_files_in_workspace_folders(proj_folders.clone()).await;

        workspace_files
            .iter()
            .filter_map(|path| {
                let filename = path.file_name()?.to_string_lossy().to_string();

                let relative_path = path.strip_prefix(proj_folder)
                    .unwrap_or(path) // If strip_prefix fails, use the full path
                    .to_string_lossy()
                    .to_string();

                Some((filename, relative_path))
            })
            .collect()
    }

    #[tokio::test]
    async fn test_fuzzy_search_finds_frog_py() {
        let correction_candidate = "frog.p".to_string();
        let top_n = 2;

        let candidates = get_candidates_from_workspace_files().await;

        let result = fuzzy_search(&correction_candidate, candidates, top_n);

        let expected_result = vec!["tests/emergency_frog_situation/frog.py".to_string()];

        assert_eq!(result, expected_result, "It should find the proper frog.py, found {:?} instead", result);
    }

    #[tokio::test]
    async fn test_fuzzy_search_path_helps_finding_file() {
        let correction_candidate = "tests/emergency_frog_situation/w".to_string();
        let top_n = 2;

        let candidates = get_candidates_from_workspace_files().await;

        let result = fuzzy_search(&correction_candidate, candidates, top_n);

        let expected_result = vec!["tests/emergency_frog_situation/work_day.py".to_string()];

        assert_eq!(result, expected_result, "It should find the proper file (work_day.py), found {:?} instead", result);
    }
}