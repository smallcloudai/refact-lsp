use std::collections::{HashMap, HashSet};
use std::path::{Component, PathBuf};
use std::sync::Arc;
use std::time::Instant;
use tokio::sync::RwLock as ARwLock;
use tracing::info;

use crate::global_context::GlobalContext;

pub async fn paths_from_anywhere(global_context: Arc<ARwLock<GlobalContext>>) -> Vec<PathBuf> {
    let file_paths_from_memory = global_context.read().await.documents_state.memory_document_map.keys().map(|x|x.clone()).collect::<Vec<_>>();
    let paths_from_workspace: Vec<PathBuf> = global_context.read().await.documents_state.workspace_files.lock().unwrap().clone();
    let paths_from_jsonl: Vec<PathBuf> = global_context.read().await.documents_state.jsonl_files.lock().unwrap().clone();
    let paths_from_anywhere = file_paths_from_memory.into_iter().chain(paths_from_workspace.into_iter().chain(paths_from_jsonl.into_iter()));
    paths_from_anywhere.collect::<Vec<PathBuf>>()
}

fn make_cache(paths: &Vec<PathBuf>, workspace_folders: &Vec<PathBuf>) -> (
    HashMap<String, HashSet<String>>, Vec<String>, usize
) {
    let mut cache_correction = HashMap::<String, HashSet<String>>::new();
    let mut cnt = 0;

    for path in paths {
        let path_str = path.to_str().unwrap_or_default().to_string();

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

    // Find the shortest unique suffix for each path, that is at least the path from workspace root
    let cache_shortened = paths.iter().map(|path| {
        let workspace_components_len = workspace_folders.iter()
            .filter_map(|workspace_dir| {
                if path.starts_with(workspace_dir) {
                    Some(workspace_dir.components().count())
                } else {
                    None
                }
            })
            .max()
            .unwrap_or(0);

        let path_is_dir = path.to_string_lossy().ends_with(std::path::MAIN_SEPARATOR);
        let mut current_suffix = PathBuf::new();
        for component in path.components().rev() {
            if !current_suffix.as_os_str().is_empty() || path_is_dir {
                current_suffix = PathBuf::from(component.as_os_str()).join(&current_suffix);
            } else {
                current_suffix = PathBuf::from(component.as_os_str());
            }
            let suffix = current_suffix.to_string_lossy().into_owned();
            if cache_correction.get(suffix.as_str()).map_or(0, |v| v.len()) == 1 &&
                current_suffix.components().count() + workspace_components_len >= path.components().count() {
                cnt += 1;
                return suffix;
            }
        }
        cnt += 1;
        path.to_string_lossy().into_owned()
    }).collect();

    (cache_correction, cache_shortened, cnt)
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

pub async fn files_cache_rebuild_as_needed(global_context: Arc<ARwLock<GlobalContext>>) -> (Arc<HashMap<String, HashSet<String>>>, Arc<Vec<String>>) {
    let (cache_dirty_arc, mut cache_correction_arc, mut cache_shortened_arc) = {
        let cx = global_context.read().await;
        (
            cx.documents_state.cache_dirty.clone(),
            cx.documents_state.cache_correction.clone(),
            cx.documents_state.cache_shortened.clone(),
        )
    };

    let mut cache_dirty_ref = cache_dirty_arc.lock().await;
    if *cache_dirty_ref {
        info!("rebuilding files cache...");
        // filter only get_project_dirs?
        let start_time = Instant::now();
        let paths_from_anywhere = paths_from_anywhere(global_context.clone()).await;
        let workspace_folders = get_project_dirs(global_context.clone()).await;
        let (cache_correction, cache_shortened, cnt) = make_cache(&paths_from_anywhere, &workspace_folders);

        info!("rebuild completed in {:.3}s, {} URLs => cache_correction.len is now {}", start_time.elapsed().as_secs_f64(), cnt, cache_correction.len());
        cache_correction_arc = Arc::new(cache_correction);
        cache_shortened_arc = Arc::new(cache_shortened);
        {
            let mut cx = global_context.write().await;
            cx.documents_state.cache_correction = cache_correction_arc.clone();
            cx.documents_state.cache_shortened = cache_shortened_arc.clone();
        }
        *cache_dirty_ref = false;
    }

    return (cache_correction_arc, cache_shortened_arc);
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
where I: IntoIterator<Item = String> {
    let correction_candidate_filename = PathBuf::from(&correction_candidate)
        .file_name()
        .map(|name| name.to_string_lossy().to_string())
        .unwrap_or(correction_candidate.clone());

    let mut top_n_records = Vec::with_capacity(top_n);
    for path in candidates {
        let filename = PathBuf::from(&path).file_name().unwrap_or_default().to_string_lossy().to_string();

        let filename_dist = normalized_distance(&correction_candidate_filename, &filename);
        let path_dist = normalized_distance(&correction_candidate, &path);

        top_n_records.push((path, filename_dist * 2.5 + path_dist));
        if top_n_records.len() >= top_n {
            top_n_records.sort_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal));
            top_n_records.pop();
        }
    }
    top_n_records.sort_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal));
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
        let mut dirs = HashSet::<String>::new();

        for p in cache_fuzzy_set.iter() {
            let mut current_path = PathBuf::from(&p);
            while let Some(parent) = current_path.parent() {
                dirs.insert(parent.to_string_lossy().to_string());
                current_path = parent.to_path_buf();
            }
        }

        info!("fuzzy search {:?}, dirs.len={}", correction_candidate, dirs.len());
        return fuzzy_search(correction_candidate, dirs.iter().cloned(), top_n);
    }
    vec![]
}

pub async fn get_project_dirs(gcx: Arc<ARwLock<GlobalContext>>) -> Vec<PathBuf> {
    let gcx_locked = gcx.write().await;
    let workspace_folders = gcx_locked.documents_state.workspace_folders.lock().unwrap();
    workspace_folders.iter().cloned().collect::<Vec<_>>()
}

pub async fn shortify_paths(gcx: Arc<ARwLock<GlobalContext>>, paths: Vec<String>) -> Vec<String> {
    let (_, shortened_paths) = files_cache_rebuild_as_needed(gcx.clone()).await;

    // Temporary slow way, will change by a trie
    paths.iter().map(|path| {
        for shortened in (*shortened_paths).iter() {
            if path.ends_with(shortened) {
                return shortened.clone();
            }
        }
        path.clone()
    }).collect()
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
    use crate::files_in_workspace::retrieve_files_in_workspace_folders;

    async fn get_candidates_from_workspace_files() -> Vec<String> {
        let proj_folders = vec![PathBuf::from(".").canonicalize().unwrap()];
        let proj_folder = &proj_folders[0];

        let workspace_files = retrieve_files_in_workspace_folders(proj_folders.clone()).await;

        workspace_files
            .iter()
            .filter_map(|path| {
                let relative_path = path.strip_prefix(proj_folder)
                    .unwrap_or(path)
                    .to_string_lossy()
                    .to_string();

                    Some(relative_path)
            })
            .collect()
    }

    #[tokio::test]
    async fn test_fuzzy_search_finds_frog_py() {
        // Arrange
        let correction_candidate = "frog.p".to_string();
        let top_n = 2;

        let candidates = get_candidates_from_workspace_files().await;

        // Act
        let result = fuzzy_search(&correction_candidate, candidates, top_n);

        // Assert
        let expected_result = vec!["/home/user/workspace/tests/emergency_frog_situation/frog.py".to_string()];

        assert_eq!(result, expected_result, "It should find the proper frog.py, found {:?} instead", result);
    }

    #[tokio::test]
    async fn test_fuzzy_search_path_helps_finding_file() {
        // Arrange
        let correction_candidate = "tests/emergency_frog_situation/w".to_string();
        let top_n = 2;

        let candidates = get_candidates_from_workspace_files().await;

        // Act
        let result = fuzzy_search(&correction_candidate, candidates, top_n);

        // Assert
        let expected_result = vec!["/home/user/workspace/tests/emergency_frog_situation/work_day.py".to_string()];

        assert_eq!(result, expected_result, "It should find the proper file (work_day.py), found {:?} instead", result);
    }

    #[tokio::test]
    async fn test_fuzzy_search_filename_weights_more_than_path() {
        // Arrange
        let correction_candidate = "my_file.ext".to_string();
        let top_n = 3;

        let candidates = vec![
            "my_library/implementation/my_file.ext".to_string(),
            "my_library/my_file.ext".to_string(),
            "another_file.ext".to_string(),
        ];

        // Act
        let result = fuzzy_search(&correction_candidate, candidates, top_n);

        // Assert
        let expected_result = vec![
            "/home/user/workspace/my_library/my_file.ext".to_string(),
            "/home/user/workspace/my_library/implementation/my_file.ext".to_string(),
        ];
        
        let mut sorted_result = result.clone();
        let mut sorted_expected = expected_result.clone();
        
        sorted_result.sort();
        sorted_expected.sort();

        assert_eq!(sorted_result, sorted_expected, "The result should contain the expected paths in any order, found {:?} instead", result);
    }

    #[test]
    fn test_make_cache() {
        // Arrange
        let paths = vec![
            PathBuf::from("/home/user/repo1/dir/file.ext"),
            PathBuf::from("/home/user/repo2/dir/file.ext"),
            PathBuf::from("/home/user/repo1/this_file.ext"),
            PathBuf::from("/home/user/repo2/dir/this_file.ext"),
            PathBuf::from("/home/user/repo2/dir2/"),
        ];

        let workspace_folders = vec![
            PathBuf::from("/home/user/repo1"),
            PathBuf::from("/home/user/repo2"),
        ];

        // Act
        let(_, cache_shortened_result, cnt) = make_cache(&paths, &workspace_folders);

        // Assert
        let expected_result = vec![
            "repo1/dir/file.ext".to_string(),
            "repo2/dir/file.ext".to_string(),
            "repo1/this_file.ext".to_string(),
            "dir/this_file.ext".to_string(),
            "dir2/".to_string(),
        ];

        assert_eq!(cnt, 5, "The cache should contain 5 paths");
        assert_eq!(cache_shortened_result, expected_result, "The result should contain the expected paths, instead it found");
    }
}