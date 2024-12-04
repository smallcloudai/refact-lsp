use std::path::PathBuf;
use tracing::error;
use git2::{Branch, BranchType, DiffOptions, IndexAddOption, Oid, Repository, Signature, Status, StatusOptions};

pub fn git_ls_files(repository_path: &PathBuf) -> Option<Vec<PathBuf>> {
    let repository = Repository::open(repository_path)
        .map_err(|e| error!("Failed to open repository: {}", e)).ok()?;
    
    let mut status_options = StatusOptions::new();
    status_options
        .include_untracked(true)
        .recurse_untracked_dirs(true)
        .include_unmodified(true)
        .exclude_submodules(false)
        .include_ignored(false)
        .recurse_ignored_dirs(false);

    let statuses = repository.statuses(Some(&mut status_options))
        .map_err(|e| error!("Failed to get statuses: {}", e)).ok()?;

    let mut files = Vec::new();
    for entry in statuses.iter() {
        if let Some(path) = entry.path() {
            files.push(repository_path.join(path));
        }
    }
    if !files.is_empty() { Some(files) } else { None }
}

/// Similar to git checkout -b <branch_name>
pub fn create_or_checkout_to_branch<'repo>(repository: &'repo Repository, branch_name: &str) -> Result<Branch<'repo>, String> {
    let branch = match repository.find_branch(branch_name, BranchType::Local) {
        Ok(branch) => branch,
        Err(_) => {
            let head_commit = repository.head()
                .and_then(|h| h.peel_to_commit())
                .map_err(|e| format!("Failed to get HEAD commit: {}", e))?;
            repository.branch(branch_name, &head_commit, false)
                .map_err(|e| format!("Failed to create branch: {}", e))?
        }
    };

    // Checkout to the branch
    let object = repository.revparse_single(&("refs/heads/".to_owned() + branch_name))
        .map_err(|e| format!("Failed to revparse single: {}", e))?;
    repository.checkout_tree(&object, None)
        .map_err(|e| format!("Failed to checkout tree: {}", e))?;
    repository.set_head(&format!("refs/heads/{}", branch_name))
      .map_err(|e| format!("Failed to set head: {}", e))?;

    Ok(branch)
}

/// Similar to git add .
pub fn stage_all_changes(repository: &Repository) -> Result<(), String> {
    let mut index = repository.index()
        .map_err(|e| format!("Failed to get index: {}", e))?;
    index.add_all(["*"].iter(), IndexAddOption::DEFAULT, None)
        .map_err(|e| format!("Failed to add files to index: {}", e))?;
    index.write()
        .map_err(|e| format!("Failed to write index: {}", e))?;
    Ok(()) 
}

/// Returns:
/// 
/// A tuple containing the number of new files, modified files, and deleted files.
pub fn count_file_changes(repository: &Repository) -> Result<(usize, usize, usize), String> {
    let (mut new_files, mut modified_files, mut deleted_files) = (0, 0, 0);

    let statuses = repository.statuses(None)
        .map_err(|e| format!("Failed to get statuses: {}", e))?;
    for entry in statuses.iter() {
        let status = entry.status();
        if status.contains(Status::INDEX_NEW) { new_files += 1; }
        if status.contains(Status::INDEX_MODIFIED) { modified_files += 1;}
        if status.contains(Status::INDEX_DELETED) { deleted_files += 1; }
    }

    Ok((new_files, modified_files, deleted_files))
}

pub fn commit(repository: &Repository, branch: &Branch, message: &str, author_name: &str, author_email: &str) -> Result<Oid, String> {
    
    let mut index = repository.index()
        .map_err(|e| format!("Failed to get index: {}", e))?;
    let tree_id = index.write_tree()
        .map_err(|e| format!("Failed to write tree: {}", e))?;
    let tree = repository.find_tree(tree_id)
        .map_err(|e| format!("Failed to find tree: {}", e))?;

    let signature = Signature::now(author_name, author_email)
        .map_err(|e| format!("Failed to create signature: {}", e))?;

    let branch_ref_name = branch.get().name()
        .ok_or_else(|| "Invalid branch name".to_string())?;

    let parent_commit = if let Some(target) = branch.get().target() {
        repository.find_commit(target)
            .map_err(|e| format!("Failed to find branch commit: {}", e))?
    } else {
        return Err("No parent commits found (initial commit is not supported)".to_string());
    };

    repository.commit(
        Some(branch_ref_name), &signature, &signature, message, &tree, &[&parent_commit]
    ).map_err(|e| format!("Failed to create commit: {}", e))
}

/// Similar to `git diff`, but including untracked files.
pub fn git_diff_from_all_changes(repository: &Repository) -> Result<String, String> {
    let mut diff_options = DiffOptions::new();
    diff_options.include_untracked(true);
    diff_options.recurse_untracked_dirs(true);

    // Create a new temporary tree, with all changes staged
    let mut index = repository.index().map_err(|e| format!("Failed to get repository index: {}", e))?;
    index.add_all(["*"].iter(), IndexAddOption::DEFAULT, None)
        .map_err(|e| format!("Failed to add files to index: {}", e))?;
    let oid = index.write_tree().map_err(|e| format!("Failed to write tree: {}", e))?;
    let new_tree = repository.find_tree(oid).map_err(|e| format!("Failed to find tree: {}", e))?;

    let head = repository.head().and_then(|head_ref| head_ref.peel_to_tree())
        .map_err(|e| format!("Failed to get HEAD tree: {}", e))?;

    let diff = repository.diff_tree_to_tree(Some(&head), Some(&new_tree), Some(&mut diff_options))
        .map_err(|e| format!("Failed to generate diff: {}", e))?;

    let mut diff_str = String::new();
    diff.print(git2::DiffFormat::Patch, |_, _, line| {
        diff_str.push(line.origin());
        diff_str.push_str(std::str::from_utf8(line.content()).unwrap_or(""));
        true
    }).map_err(|e| format!("Failed to print diff: {}", e))?;

    Ok(diff_str)
}
