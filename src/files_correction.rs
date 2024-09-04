use std::collections::{HashMap, HashSet};
use std::path::{Component, PathBuf};
use std::sync::Arc;
use std::time::Instant;
use crate::global_context::GlobalContext;
use itertools::Itertools;
use tokio::sync::RwLock as ARwLock;
use strsim::normalized_damerau_levenshtein;
use tracing::info;


pub async fn paths_from_anywhere(global_context: Arc<ARwLock<GlobalContext>>) -> Vec<PathBuf> {
    let file_paths_from_memory = global_context.read().await.documents_state.memory_document_map.keys().map(|x|x.clone()).collect::<Vec<_>>();
    let paths_from_workspace: Vec<PathBuf> = global_context.read().await.documents_state.workspace_files.lock().unwrap().clone();
    let paths_from_jsonl: Vec<PathBuf> = global_context.read().await.documents_state.jsonl_files.lock().unwrap().clone();
    let paths_from_anywhere = file_paths_from_memory.into_iter().chain(paths_from_workspace.into_iter().chain(paths_from_jsonl.into_iter()));
    paths_from_anywhere.collect::<Vec<PathBuf>>()
}

fn get_last_component(p: &String) -> Option<String> {
    PathBuf::from(p).components().last().map(|comp| comp.as_os_str().to_string_lossy().to_string())
}

fn get_parent(p: &String) -> Option<String> {
    PathBuf::from(p).parent().map(PathBuf::from).map(|x|x.to_string_lossy().to_string())
}

fn _make_cache(
    paths_from_anywhere: Vec<PathBuf>
) -> (
    HashMap<String, HashSet<String>>,
    HashMap<String, String>,
    HashMap<String, HashSet<String>>,
    HashMap<String, String>,
    usize
) {
    let (mut cache_correction_files, mut cache_fuzzy_files) = (HashMap::new(), HashMap::new());
    let (mut cache_correction_dirs, mut cache_fuzzy_dirs) = (HashMap::new(), HashMap::new());
    let mut cnt = 0;

    for path in paths_from_anywhere {
        let path_str = path.to_str().unwrap_or_default().to_string();
        let file_name = path.file_name().unwrap_or_default().to_string_lossy().to_string();
        cache_fuzzy_files.insert(file_name.clone(), path_str.clone());
        if let Some(parent) = get_parent(&path_str) {
            if let Some(last_component) = get_last_component(&parent) {
                cache_fuzzy_dirs.insert(last_component, parent);
            }
        }
        cnt += 1;

        cache_correction_files.entry(path_str.clone()).or_insert_with(HashSet::new).insert(path_str.clone());
        // chop off directory names one by one
        let mut index = 0;
        while let Some(slashpos) = path_str[index .. ].find(|c| c == '/' || c == '\\') {
            let absolute_slashpos = index + slashpos;
            index = absolute_slashpos + 1;
            let slashpos_to_end = &path_str[index .. ];
            if !slashpos_to_end.is_empty() {
                cache_correction_files.entry(slashpos_to_end.to_string()).or_insert_with(HashSet::new).insert(path_str.clone());
            }
        }
    }
    for (k, v) in cache_correction_files.iter() {
        if let Some(k_parent) = get_parent(k) {
            let v_parents = v.iter().filter_map(|x| get_parent(x)).collect::<Vec<_>>();
            if !v_parents.is_empty() {
                cache_correction_dirs.entry(k_parent.clone()).or_insert_with(HashSet::new).extend(v_parents);
            }
        }
    }

    (
        cache_correction_files,
        cache_fuzzy_files,
        cache_correction_dirs,
        cache_fuzzy_dirs, cnt
    )
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

pub async fn files_cache_rebuild_as_needed(
    global_context: Arc<ARwLock<GlobalContext>>
) -> (
    Arc<HashMap<String, HashSet<String>>>,
    Arc<HashMap<String, String>>,
    Arc<HashMap<String, HashSet<String>>>,
    Arc<HashMap<String, String>>
) {
    let (
        cache_dirty_arc,
        mut cache_correction_files_arc,
        mut cache_fuzzy_files_arc,
        mut cache_correction_dirs_arc,
        mut cache_fuzzy_dirs_arc,
    ) = {
        let cx = global_context.read().await;
        (
            cx.documents_state.cache_dirty.clone(),
            cx.documents_state.cache_correction_files.clone(),
            cx.documents_state.cache_fuzzy_files.clone(),
            cx.documents_state.cache_correction_dirs.clone(),
            cx.documents_state.cache_fuzzy_dirs.clone(),
        )
    };

    let mut cache_dirty_ref = cache_dirty_arc.lock().await;
    if *cache_dirty_ref {
        info!("rebuilding files cache...");
        // filter only get_project_dirs?
        let start_time = Instant::now();
        let paths_from_anywhere = paths_from_anywhere(global_context.clone()).await;
        let (
            cache_correction_files,
            cache_fuzzy_files,
            cache_correction_dirs,
            cache_fuzzy_dirs,
            cnt
        ) = _make_cache(paths_from_anywhere);

        info!("rebuild completed in {}s, {} URLs => cache_correction_files.len is now {}", start_time.elapsed().as_secs(), cnt, cache_correction_files.len());

        cache_correction_files_arc = Arc::new(cache_correction_files);
        cache_fuzzy_files_arc = Arc::new(cache_fuzzy_files);
        cache_correction_dirs_arc = Arc::new(cache_correction_dirs);
        cache_fuzzy_dirs_arc = Arc::new(cache_fuzzy_dirs);

        {
            let mut cx = global_context.write().await;
            cx.documents_state.cache_correction_files = cache_correction_files_arc.clone();
            cx.documents_state.cache_fuzzy_files = cache_fuzzy_files_arc.clone();
            cx.documents_state.cache_correction_dirs = cache_correction_dirs_arc.clone();
            cx.documents_state.cache_fuzzy_dirs = cache_fuzzy_dirs_arc.clone();
        }
        *cache_dirty_ref = false;
    }

    (
        cache_correction_files_arc,
        cache_fuzzy_files_arc,
        cache_correction_dirs_arc,
        cache_fuzzy_dirs_arc
    )
}

fn fuzzy_search(
    cache_correction_files_arc: Arc<HashMap<String, HashSet<String>>>,
    cache_fuzzy_files_arc: Arc<HashMap<String, String>>,
    cache_correction_dirs_arc: Arc<HashMap<String, HashSet<String>>>,
    correction_candidate: &String,
    top_n: usize,
) -> Vec<String> {
    let mut top_n_records = Vec::with_capacity(top_n);

    // prefixes -- ways correction_candidate.parent() can be completed (plural)
    // if prefix is found, correction candidate will be shortened to its last component
    let (prefixes, correction_candidate) = PathBuf::from(correction_candidate).parent()
        .map(|p| p.to_string_lossy().to_string())
        .and_then(|p| cache_correction_dirs_arc.get(&p).cloned())
        .and_then(|p| get_last_component(correction_candidate).map(|last_component| (p.into_iter().collect::<Vec<_>>(), last_component)))
        .unwrap_or((vec!["".to_string()], correction_candidate.clone()));

    // pre-filtering cache_fuzzy_files_arc with items that start with prefixes
    for (c_name, c_full_path) in cache_fuzzy_files_arc.iter().filter(|x|prefixes.iter().any(|p| x.1.starts_with(p))) {
        let dist = normalized_damerau_levenshtein(&correction_candidate, c_name);
        top_n_records.push((c_full_path.clone(), dist));
        if top_n_records.len() >= top_n {
            top_n_records.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap());
            top_n_records.pop();
        }
    }
    let mut sorted_paths  = vec![];
    for path in top_n_records.iter().sorted_by(|a, b|a.1.partial_cmp(&b.1).unwrap()).rev().map(|(path, _)| path) {
        if let Some(fixed) = (*cache_correction_files_arc).get(path) {
            sorted_paths.extend(fixed.into_iter().cloned());
        } else {
            sorted_paths.push(path.clone());
        }
    }
    sorted_paths
}

pub async fn correct_to_nearest_filename(
    global_context: Arc<ARwLock<GlobalContext>>,
    correction_candidate: &String,
    fuzzy: bool,
    top_n: usize,
) -> Vec<String> {
    let (cache_correction_files_arc, cache_fuzzy_files_arc, cache_correction_dirs_arc, _) = files_cache_rebuild_as_needed(global_context.clone()).await;
    // it's dangerous to use cache_correction_files_arc without a mutex, but should be fine as long as it's read-only
    // (another thread never writes to the map itself, it can only replace the arc with a different map)

    if let Some(fixed) = (*cache_correction_files_arc).get(&correction_candidate.clone()) {
        // info!("found {:?} in cache_correction_files, returning [{:?}]", correction_candidate, fixed);
        return fixed.into_iter().cloned().collect::<Vec<String>>();
    } else {
        info!("not found {} in cache_correction_files", correction_candidate);
    }

    if fuzzy {
        info!("fuzzy search {:?}, cache_fuzzy_files_arc.len={}", correction_candidate, cache_fuzzy_files_arc.len());
        return fuzzy_search(cache_correction_files_arc, cache_fuzzy_files_arc, cache_correction_dirs_arc, correction_candidate, top_n);
    }

    return vec![];
}

pub async fn correct_to_nearest_dir_path(
    gcx: Arc<ARwLock<GlobalContext>>,
    correction_candidate: &String,
    fuzzy: bool,
    top_n: usize,
) -> Vec<String> {
    let (.., cache_correction_dirs_arc, cache_fuzzy_dirs) = files_cache_rebuild_as_needed(gcx.clone()).await;
    if let Some(res) = cache_correction_dirs_arc.get(correction_candidate).map(|x|x.iter().cloned().collect::<Vec<_>>()) {
        return res;
    }
    return match fuzzy {
        true => fuzzy_search(cache_correction_dirs_arc.clone(), cache_fuzzy_dirs, cache_correction_dirs_arc, correction_candidate, top_n),
        false => vec![]
    };
}

pub async fn get_project_dirs(gcx: Arc<ARwLock<GlobalContext>>) -> Vec<PathBuf> {
    let gcx_locked = gcx.write().await;
    let workspace_folders = gcx_locked.documents_state.workspace_folders.lock().unwrap();
    workspace_folders.iter().cloned().collect::<Vec<_>>()
}

pub async fn shortify_paths(gcx: Arc<ARwLock<GlobalContext>>, paths: Vec<String>) -> Vec<String> {
    let (cache_correction_files_arc, ..) = files_cache_rebuild_as_needed(gcx.clone()).await;
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
                if let Some(candidates) = cache_correction_files_arc.get(p_no_base) {
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

