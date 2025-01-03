use std::collections::{HashMap, HashSet};
use std::fs;
use std::hash::Hash;
use std::path::PathBuf;
use std::sync::{Arc, Weak, Mutex as StdMutex};
use std::time::Instant;
use notify::{Config, Event, EventKind, RecommendedWatcher, RecursiveMode, Watcher};
use notify::event::{CreateKind, DataChange, ModifyKind, RemoveKind};
use ropey::Rope;
use tokio::sync::{RwLock as ARwLock, Mutex as AMutex};
use walkdir::WalkDir;
use which::which;
use tracing::info;

use crate::git::git_ls_files;
use crate::global_context::GlobalContext;
use crate::telemetry;
use crate::file_filter::{is_this_inside_blacklisted_dir, is_valid_file, BLACKLISTED_DIRS, SOURCE_FILE_EXTENSIONS};
use crate::ast::ast_indexer_thread::ast_indexer_enqueue_files;
use crate::privacy::{check_file_privacy, load_privacy_if_needed, PrivacySettings, FilePrivacyLevel};


#[derive(Debug, Eq, Hash, PartialEq, Clone)]
pub struct Document {
    pub doc_path: PathBuf,
    pub doc_text: Option<Rope>,
}

pub async fn get_file_text_from_memory_or_disk(global_context: Arc<ARwLock<GlobalContext>>, file_path: &PathBuf) -> Result<String, String>
{
    check_file_privacy(load_privacy_if_needed(global_context.clone()).await, &file_path, &FilePrivacyLevel::AllowToSendAnywhere)?;

    if let Some(doc) = global_context.read().await.documents_state.memory_document_map.get(file_path) {
        let doc = doc.read().await;
        if doc.doc_text.is_some() {
            return Ok(doc.doc_text.as_ref().unwrap().to_string());
        }
    }
    read_file_from_disk_without_privacy_check(&file_path)
        .await.map(|x|x.to_string())
        .map_err(|e|format!("Not found in memory, not found on disk: {}", e))
}

impl Document {
    pub fn new(doc_path: &PathBuf) -> Self {
        Self { doc_path: doc_path.clone(),  doc_text: None }
    }

    #[cfg(feature="vecdb")]
    pub async fn update_text_from_disk(&mut self, gcx: Arc<ARwLock<GlobalContext>>) -> Result<(), String> {
        match read_file_from_disk(load_privacy_if_needed(gcx.clone()).await, &self.doc_path).await {
            Ok(res) => {
                self.doc_text = Some(res);
                return Ok(());
            },
            Err(e) => {
                return Err(e)
            }
        }
    }

    pub async fn get_text_or_read_from_disk(&mut self, gcx: Arc<ARwLock<GlobalContext>>) -> Result<String, String> {
        if self.doc_text.is_some() {
            return Ok(self.doc_text.as_ref().unwrap().to_string());
        }
        read_file_from_disk(load_privacy_if_needed(gcx.clone()).await, &self.doc_path).await.map(|x|x.to_string())
    }

    pub fn update_text(&mut self, text: &String) {
        self.doc_text = Some(Rope::from_str(text));
    }

    #[cfg(feature="vecdb")]
    pub fn text_as_string(&self) -> Result<String, String> {
        if let Some(r) = &self.doc_text {
            return Ok(r.to_string());
        }
        return Err(format!("no text loaded in {}", self.doc_path.display()));
    }

    pub fn does_text_look_good(&self) -> Result<(), String> {
        // Some simple tests to find if the text is suitable to parse (not generated or compressed code)
        assert!(self.doc_text.is_some());
        let r = self.doc_text.as_ref().unwrap();

        let total_chars = r.chars().count();
        let total_lines = r.lines().count();
        let avg_line_length = total_chars / total_lines;
        if avg_line_length > 150 {
            return Err("generated, avg line length > 150".to_string());
        }

        // example: hl.min.js
        let total_spaces = r.chars().filter(|x| x.is_whitespace()).count();
        let spaces_percentage = total_spaces as f32 / total_chars as f32;
        if total_lines >= 5 && spaces_percentage <= 0.05 {
            return Err(format!("generated or compressed, {:.1}% spaces < 5%", 100.0*spaces_percentage));
        }

        Ok(())
    }
}

pub struct DocumentsState {
    pub workspace_folders: Arc<StdMutex<Vec<PathBuf>>>,
    pub workspace_files: Arc<StdMutex<Vec<PathBuf>>>,
    pub active_file_path: Option<PathBuf>,
    pub jsonl_files: Arc<StdMutex<Vec<PathBuf>>>,
    // document_map on windows: c%3A/Users/user\Documents/file.ext
    // query on windows: C:/Users/user/Documents/file.ext
    pub memory_document_map: HashMap<PathBuf, Arc<ARwLock<Document>>>,   // if a file is open in IDE, and it's outside workspace dirs, it will be in this map and not in workspace_files
    pub cache_dirty: Arc<AMutex<f64>>,
    pub cache_correction: Arc<HashMap<String, HashSet<String>>>,  // map dir3/file.ext -> to /dir1/dir2/dir3/file.ext
    pub cache_shortened: Arc<HashSet<String>>,
    pub fs_watcher: Arc<ARwLock<RecommendedWatcher>>,
}

async fn mem_overwrite_or_create_document(
    global_context: Arc<ARwLock<GlobalContext>>,
    document: Document
) -> (Arc<ARwLock<Document>>, Arc<AMutex<f64>>, bool) {
    let mut cx = global_context.write().await;
    let doc_map = &mut cx.documents_state.memory_document_map;
    if let Some(existing_doc) = doc_map.get_mut(&document.doc_path) {
        *existing_doc.write().await = document;
        (existing_doc.clone(), cx.documents_state.cache_dirty.clone(), false)
    } else {
        let path = document.doc_path.clone();
        let darc = Arc::new(ARwLock::new(document));
        doc_map.insert(path, darc.clone());
        (darc, cx.documents_state.cache_dirty.clone(), true)
    }
}

impl DocumentsState {
    pub async fn new(
        workspace_dirs: Vec<PathBuf>,
    ) -> Self {
        let watcher = RecommendedWatcher::new(|_|{}, Default::default()).unwrap();
        Self {
            workspace_folders: Arc::new(StdMutex::new(workspace_dirs)),
            workspace_files: Arc::new(StdMutex::new(Vec::new())),
            active_file_path: None,
            jsonl_files: Arc::new(StdMutex::new(Vec::new())),
            memory_document_map: HashMap::new(),
            cache_dirty: Arc::new(AMutex::<f64>::new(0.0)),
            cache_correction: Arc::new(HashMap::<String, HashSet<String>>::new()),
            cache_shortened: Arc::new(HashSet::<String>::new()),
            fs_watcher: Arc::new(ARwLock::new(watcher)),
        }
    }

    pub fn init_watcher(&mut self, gcx_weak: Weak<ARwLock<GlobalContext>>) {
        let rt = tokio::runtime::Handle::current();
        let event_callback = move |res| {
            rt.block_on(async {
                if let Ok(event) = res {
                    file_watcher_event(event, gcx_weak.clone()).await;
                }
            });
        };
        let mut watcher = RecommendedWatcher::new(event_callback, Config::default()).unwrap();
        for folder in self.workspace_folders.lock().unwrap().iter() {
            info!("ADD WATCHER (1): {}", folder.display());
            let _ = watcher.watch(folder, RecursiveMode::Recursive);  // actually that might not exist because you can load a project into IDE and the folder is deleted for whatever reason
        }
        self.fs_watcher = Arc::new(ARwLock::new(watcher));
    }
}

async fn read_file_from_disk_without_privacy_check(
    path: &PathBuf,
) -> Result<Rope, String> {
    tokio::fs::read_to_string(path).await
        .map(|x|Rope::from_str(&x))
        .map_err(|e|
            format!("failed to read file {}: {}", crate::nicer_logs::last_n_chars(&path.display().to_string(), 30), e)
        )
}

pub async fn read_file_from_disk(
    privacy_settings: Arc<PrivacySettings>,
    path: &PathBuf,
) -> Result<Rope, String> {
    check_file_privacy(privacy_settings, path, &FilePrivacyLevel::AllowToSendAnywhere)?;
    read_file_from_disk_without_privacy_check(path).await
}

async fn _run_command(cmd: &str, args: &[&str], path: &PathBuf, filter_out_status: bool) -> Option<Vec<PathBuf>> {
    info!("{} EXEC {} {}", path.display(), cmd, args.join(" "));
    let output = async_process::Command::new(cmd)
        .args(args)
        .current_dir(path)
        .output()
        .await
        .ok()?;

    if !output.status.success() {
        return None;
    }

    String::from_utf8(output.stdout.clone())
        .ok()
        .map(|s| s.lines().map(|line| {
            let trimmed = line.trim();
            if filter_out_status && trimmed.len() > 1 {
                path.join(&trimmed[1..].trim())
            } else {
                path.join(line)
            }
        }).collect())
}

async fn ls_files_under_version_control(path: &PathBuf) -> Option<Vec<PathBuf>> {
    if path.join(".git").exists() {
        git_ls_files(path)
    } else if path.join(".hg").exists() && which("hg").is_ok() {
        // Mercurial repository
        _run_command("hg", &["status", "--added", "--modified", "--clean", "--unknown", "--no-status"], path, false).await
    } else if path.join(".svn").exists() && which("svn").is_ok() {
        // SVN repository
        let files_under_vc = _run_command("svn", &["list", "-R"], path, false).await;
        let files_changed = _run_command("svn", &["status"], path, true).await;
        Some(files_under_vc.unwrap_or_default().into_iter().chain(files_changed.unwrap_or_default().into_iter()).collect())
    } else {
        None
    }
}

pub fn ls_files(path: &PathBuf, recursive: bool) -> Result<Vec<PathBuf>, String> {
    if !path.is_dir() {
        return Err(format!("path '{}' is not a directory", path.display()));
    }

    let mut paths = vec![];
    let mut dirs_to_visit = vec![path.clone()];

    while let Some(dir) = dirs_to_visit.pop() {
        let ls_maybe = fs::read_dir(&dir);
        if ls_maybe.is_err() {
            info!("failed to read directory {}: {}", dir.display(), ls_maybe.unwrap_err());
            continue;
        }
        let ls: fs::ReadDir = ls_maybe.unwrap();
        let entries_maybe = ls.collect::<Result<Vec<_>, _>>();
        if entries_maybe.is_err() {
            info!("failed to read directory {}: {}", dir.display(), entries_maybe.unwrap_err());
            continue;
        }
        let mut entries = entries_maybe.unwrap();
        entries.sort_by_key(|entry| entry.file_name());
        for entry in entries {
            let path = entry.path();
            if recursive && path.is_dir() && !(
                path.file_name().unwrap_or_default().to_str().unwrap_or_default().starts_with(".") ||
                BLACKLISTED_DIRS.contains(&path.file_name().unwrap_or_default().to_str().unwrap_or_default())
            ) {
                dirs_to_visit.push(path);
            } else if path.is_file() {
                paths.push(path);
            }
        }
    }
    Ok(paths)
}

pub async fn detect_vcs_for_a_file_path(file_path: &PathBuf) -> Option<(PathBuf, &'static str)> {
    let mut dir = file_path.clone();
    if dir.is_file() {
        dir.pop();
    }
    loop {
        if dir.join(".git").is_dir() {
            return Some((dir.clone(), "git"));
        } else if dir.join(".svn").is_dir() {
            return Some((dir.clone(), "svn"));
        } else if dir.join(".hg").is_dir() {
            return Some((dir.clone(), "hg"));
        }
        if !dir.pop() {
            break;
        }
    }
    None
}

// Slow version of version control detection:
// async fn is_git_repo(directory: &PathBuf) -> bool {
//     Command::new("git")
//         .arg("rev-parse")
//         .arg("--is-inside-work-tree")
//         .current_dir(directory)
//         .output()
//         .await
//         .map(|output| output.status.success())
//         .unwrap_or(false)
// }
// async fn is_svn_repo(directory: &PathBuf) -> bool {
//     Command::new("svn")
//         .arg("info")
//         .current_dir(directory)
//         .output()
//         .await
//         .map(|output| output.status.success())
//         .unwrap_or(false)
// }
// async fn is_hg_repo(directory: &PathBuf) -> bool {
//     Command::new("hg")
//         .arg("root")
//         .current_dir(directory)
//         .output()
//         .await
//         .map(|output| output.status.success())
//         .unwrap_or(false)
// }

async fn ls_files_under_version_control_recursive(
    path: PathBuf,
    allow_files_in_hidden_folders: bool,
    ignore_size_thresholds: bool
) -> Vec<PathBuf> {
    let mut paths: Vec<PathBuf> = vec![];
    let mut candidates: Vec<PathBuf> = vec![path];
    let mut rejected_reasons: HashMap<String, usize> = HashMap::new();
    let mut blacklisted_dirs_cnt: usize = 0;
    while !candidates.is_empty() {
        let local_path = candidates.pop().unwrap();
        if local_path.is_file() {
            let maybe_valid = is_valid_file(
                &local_path, allow_files_in_hidden_folders, ignore_size_thresholds);
            match maybe_valid {
                Ok(_) => {
                    paths.push(local_path.clone());
                }
                Err(e) => {
                    rejected_reasons.entry(e.to_string()).and_modify(|x| *x += 1).or_insert(1);
                    continue;
                }
            }
        }
        if local_path.is_dir() {
            if BLACKLISTED_DIRS.contains(&local_path.file_name().unwrap().to_str().unwrap()) {
                blacklisted_dirs_cnt += 1;
                continue;
            }
            let maybe_files = ls_files_under_version_control(&local_path).await;
            if let Some(v) = maybe_files {
                for x in v.iter() {
                    let maybe_valid = is_valid_file(
                        x, allow_files_in_hidden_folders, ignore_size_thresholds);
                    match maybe_valid {
                        Ok(_) => {
                            paths.push(x.clone());
                        }
                        Err(e) => {
                            rejected_reasons.entry(e.to_string()).and_modify(|x| *x += 1).or_insert(1);
                        }
                    }
                }
            } else {
                let local_paths: Vec<PathBuf> = WalkDir::new(local_path.clone()).max_depth(1)
                    .into_iter()
                    .filter_map(|e| e.ok())
                    .map(|e| e.path().to_path_buf())
                    .filter(|e| e != &local_path)
                    .collect();
                candidates.extend(local_paths);
            }
        }
    }
    info!("rejected files reasons:");
    for (reason, count) in &rejected_reasons {
        info!("    {:>6} {}", count, reason);
    }
    if rejected_reasons.is_empty() {
        info!("    no bad files at all");
    }
    info!("also the loop bumped into {} blacklisted dirs", blacklisted_dirs_cnt);
    paths
}

pub async fn retrieve_files_in_workspace_folders(
    proj_folders: Vec<PathBuf>,
    allow_files_in_hidden_folders: bool,
    ignore_size_thresholds: bool,
) -> Vec<PathBuf> {
    let mut all_files: Vec<PathBuf> = Vec::new();
    for proj_folder in proj_folders {
        let files = ls_files_under_version_control_recursive(
            proj_folder.clone(), allow_files_in_hidden_folders, ignore_size_thresholds).await;
        all_files.extend(files);
    }
    all_files
}

pub fn is_path_to_enqueue_valid(path: &PathBuf) -> Result<(), String> {
    let extension = path.extension().unwrap_or_default();
    if !SOURCE_FILE_EXTENSIONS.contains(&extension.to_str().unwrap_or_default()) {
        return Err(format!("Unsupported file extension {:?}", extension).into());
    }
    Ok(())
}

async fn enqueue_some_docs(
    gcx: Arc<ARwLock<GlobalContext>>,
    docs: &Vec<Document>,
    force: bool,
) {
    info!("detected {} modified/added/removed files", docs.len());
    for d in docs.iter().take(5) {
        info!("    {}", crate::nicer_logs::last_n_chars(&d.doc_path.display().to_string(), 30));
    }
    if docs.len() > 5 {
        info!("    ...");
    }
    let (vec_db_module, ast_service) = {
        let cx = gcx.write().await;
        (cx.vec_db.clone(), cx.ast_service.clone())
    };
    #[cfg(feature="vecdb")]
    if let Some(ref mut db) = *vec_db_module.lock().await {
        db.vectorizer_enqueue_files(&docs, force).await;
    }
    #[cfg(not(feature="vecdb"))]
    let _ = vec_db_module;
    if let Some(ast) = &ast_service {
        let cpaths: Vec<String> = docs.iter().map(|doc| doc.doc_path.to_string_lossy().to_string()).collect();
        ast_indexer_enqueue_files(ast.clone(), cpaths, force).await;
    }
    let (cache_correction_arc, _) = crate::files_correction::files_cache_rebuild_as_needed(gcx.clone()).await;
    let mut moar_files: Vec<PathBuf> = Vec::new();
    for doc in docs {
        let doc_path_str = doc.doc_path.to_string_lossy().to_string();
        if !cache_correction_arc.contains_key(&doc_path_str) {
            moar_files.push(doc.doc_path.clone());
        }
    }
    if moar_files.len() > 0 {
        info!("this made file cache dirty");
        let dirty_arc = {
            let gcx_locked = gcx.write().await;
            gcx_locked.documents_state.workspace_files.lock().unwrap().extend(moar_files);
            gcx_locked.documents_state.cache_dirty.clone()
        };
        let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs_f64();
        *dirty_arc.lock().await = now + 1.0;  // next rebuild will be one second later, to prevent rapid-fire rebuilds from file events
    }
}

pub async fn enqueue_all_files_from_workspace_folders(
    gcx: Arc<ARwLock<GlobalContext>>,
    force: bool,
    vecdb_only: bool,
) -> i32 {
    let folders: Vec<PathBuf> = gcx.read().await.documents_state.workspace_folders.lock().unwrap().clone();

    info!("enqueue_all_files_from_workspace_folders started files search with {} folders", folders.len());
    let paths = retrieve_files_in_workspace_folders(
        folders, false, false).await;
    info!("enqueue_all_files_from_workspace_folders found {} files => workspace_files", paths.len());

    let mut documents: Vec<Document> = vec![];
    for d in paths.iter() {
        documents.push(Document { doc_path: d.clone(), doc_text: None });
    }

    let (vec_db_module, ast_service, previous_list) = {
        let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs_f64();
        let cx = gcx.write().await;
        *cx.documents_state.cache_dirty.lock().await = now;
        let mut workspace_files = cx.documents_state.workspace_files.lock().unwrap();
        let mut old_workspace_files = Vec::new();
        std::mem::swap(&mut *workspace_files, &mut old_workspace_files);
        workspace_files.extend(paths);
        (cx.vec_db.clone(), cx.ast_service.clone(), old_workspace_files)
    };

    #[cfg(feature="vecdb")]
    if let Some(ref mut db) = *vec_db_module.lock().await {
        // TODO: enqueue both lists, ones that don't open should be removed from vecdb
        db.vectorizer_enqueue_files(&documents, force).await;
    }
    #[cfg(not(feature="vecdb"))]
    let _ = vec_db_module;
    if let Some(ast) = ast_service {
        if !vecdb_only {
            let cpaths1: Vec<String> = documents.iter().map(|doc| doc.doc_path.to_string_lossy().to_string()).collect();
            let cpaths2: Vec<String> = previous_list.iter().map(|p| p.to_string_lossy().to_string()).collect();
            ast_indexer_enqueue_files(ast.clone(), cpaths1, force).await;
            ast_indexer_enqueue_files(ast.clone(), cpaths2, force).await;
        }
    }
    documents.len() as i32
}

pub async fn on_workspaces_init(gcx: Arc<ARwLock<GlobalContext>>) -> i32
{
    // Called from lsp and lsp_like
    // Not called from main.rs as part of initialization
    {
        let gcx_weak = Arc::downgrade(&gcx);
        let mut gcx_lock = gcx.write().await;
        gcx_lock.documents_state.init_watcher(gcx_weak);
    }
    enqueue_all_files_from_workspace_folders(gcx.clone(), false, false).await
}

pub async fn on_did_open(
    gcx: Arc<ARwLock<GlobalContext>>,
    cpath: &PathBuf,
    text: &String,
    _language_id: &String,
) {
    let mut doc = Document::new(cpath);
    doc.update_text(text);
    info!("on_did_open {}", crate::nicer_logs::last_n_chars(&cpath.display().to_string(), 30));
    let (_doc_arc, dirty_arc, mark_dirty) = mem_overwrite_or_create_document(gcx.clone(), doc).await;
    if mark_dirty {
        let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs_f64();
        *dirty_arc.lock().await = now;
    }
    gcx.write().await.documents_state.active_file_path = Some(cpath.clone());
}

pub async fn on_did_close(
    gcx: Arc<ARwLock<GlobalContext>>,
    cpath: &PathBuf,
) {
    info!("on_did_close {}", crate::nicer_logs::last_n_chars(&cpath.display().to_string(), 30));
    {
        let mut cx = gcx.write().await;
        if cx.documents_state.memory_document_map.remove(cpath).is_none() {
            tracing::error!("on_did_close: failed to remove from memory_document_map {:?}", cpath.display());
        }
    }
}

pub async fn on_did_change(
    gcx: Arc<ARwLock<GlobalContext>>,
    path: &PathBuf,
    text: &String,
) {
    let t0 = Instant::now();
    let (doc_arc, dirty_arc, mark_dirty) = {
        let mut doc = Document::new(path);
        doc.update_text(text);
        let (doc_arc, dirty_arc, set_mark_dirty) = mem_overwrite_or_create_document(gcx.clone(), doc).await;
        (doc_arc, dirty_arc, set_mark_dirty)
    };

    if mark_dirty {
        let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs_f64();
        *dirty_arc.lock().await = now;
    }

    gcx.write().await.documents_state.active_file_path = Some(path.clone());

    let mut go_ahead = true;
    {
        let is_it_good = is_valid_file(path, false, false);
        if is_it_good.is_err() {
            info!("{:?} ignoring changes: {}", path, is_it_good.err().unwrap());
            go_ahead = false;
        }
    }

    let doc = Document { doc_path: doc_arc.read().await.doc_path.clone(), doc_text: None };
    if go_ahead {
        enqueue_some_docs(gcx.clone(), &vec![doc], false).await;
    }

    telemetry::snippets_collection::sources_changed(
        gcx.clone(),
        &path.to_string_lossy().to_string(),
        text,
    ).await;

    info!("on_did_change {}, total time {:.3}s", crate::nicer_logs::last_n_chars(&path.to_string_lossy().to_string(), 30), t0.elapsed().as_secs_f32());
}

pub async fn on_did_delete(gcx: Arc<ARwLock<GlobalContext>>, path: &PathBuf)
{
    info!("on_did_delete {}", crate::nicer_logs::last_n_chars(&path.to_string_lossy().to_string(), 30));

    let (vec_db_module, ast_service, dirty_arc) = {
        let mut cx = gcx.write().await;
        cx.documents_state.memory_document_map.remove(path);
        (cx.vec_db.clone(), cx.ast_service.clone(), cx.documents_state.cache_dirty.clone())
    };

    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs_f64();
    (*dirty_arc.lock().await) = now;

    #[cfg(feature="vecdb")]
    match *vec_db_module.lock().await {
        Some(ref mut db) => db.remove_file(path).await,
        None => {}
    }
    #[cfg(not(feature="vecdb"))]
    let _ = vec_db_module;
    if let Some(ast) = &ast_service {
        let cpath = path.to_string_lossy().to_string();
        ast_indexer_enqueue_files(ast.clone(), vec![cpath], false).await;
    }
}

pub async fn add_folder(gcx: Arc<ARwLock<GlobalContext>>, path: &PathBuf)
{
    {
        let documents_state = &mut gcx.write().await.documents_state;
        documents_state.workspace_folders.lock().unwrap().push(path.clone());
        info!("ADD WATCHER (2): {}", path.display());
        let _ = documents_state.fs_watcher.write().await.watch(&path.clone(), RecursiveMode::Recursive);
    }
    let paths = retrieve_files_in_workspace_folders(
        vec![path.clone()], false, false).await;
    let docs: Vec<Document> = paths.into_iter().map(|p| Document { doc_path: p, doc_text: None }).collect();
    enqueue_some_docs(gcx, &docs, false).await;
}

pub async fn remove_folder(gcx: Arc<ARwLock<GlobalContext>>, path: &PathBuf)
{
    {
        let documents_state = &mut gcx.write().await.documents_state;
        documents_state.workspace_folders.lock().unwrap().retain(|p| p != path);
        let _ = documents_state.fs_watcher.write().await.unwatch(&path.clone());
    }
    enqueue_all_files_from_workspace_folders(gcx.clone(), false, false).await;
}

pub async fn file_watcher_event(event: Event, gcx_weak: Weak<ARwLock<GlobalContext>>)
{
    async fn on_create_modify(gcx_weak: Weak<ARwLock<GlobalContext>>, event: Event) {
        let mut docs = vec![];
        for p in &event.paths {
            if is_this_inside_blacklisted_dir(&p) {  // important to filter BEFORE canonical_path
                continue;
            }

            let mut go_ahead = true;
            {
                let is_it_good = is_valid_file(p, false, false);
                if is_it_good.is_err() {
                    // info!("{:?} ignoring changes: {}", p, is_it_good.err().unwrap());
                    go_ahead = false;
                }
            }

            if go_ahead {
                let cpath = crate::files_correction::canonical_path(&p.to_string_lossy().to_string());
                docs.push(Document { doc_path: cpath, doc_text: None });
            }
        }
        if docs.is_empty() {
            return;
        }
        // info!("EventKind::Create/Modify {} paths", event.paths.len());
        if let Some(gcx) = gcx_weak.clone().upgrade() {
            enqueue_some_docs(gcx, &docs, false).await;
        }
    }

    async fn on_remove(gcx_weak: Weak<ARwLock<GlobalContext>>, event: Event) {
        let mut never_mind = true;
        for p in &event.paths {
            never_mind &= is_this_inside_blacklisted_dir(&p);
        }
        let mut docs = vec![];
        if !never_mind {
            for p in &event.paths {
                if is_this_inside_blacklisted_dir(&p) {
                    continue;
                }
                let cpath = crate::files_correction::canonical_path(&p.to_string_lossy().to_string());
                docs.push(Document { doc_path: cpath, doc_text: None });
            }
        }
        if docs.is_empty() {
            return;
        }
        if let Some(gcx) = gcx_weak.clone().upgrade() {
            enqueue_some_docs(gcx, &docs, false).await;
        }
    }

    match event.kind {
        EventKind::Any => {},
        EventKind::Access(_) => {},
        EventKind::Create(CreateKind::File) => on_create_modify(gcx_weak.clone(), event).await,
        EventKind::Remove(RemoveKind::File) => on_remove(gcx_weak.clone(), event).await,
        EventKind::Modify(ModifyKind::Data(DataChange::Content)) => on_create_modify(gcx_weak.clone(), event).await,
        EventKind::Other => {}
        _ => {}
    }
}
