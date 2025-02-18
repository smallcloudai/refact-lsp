use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use async_trait::async_trait;
use serde_json::Value;
use tokio::sync::Mutex as AMutex;
use tokio::sync::RwLock as ARwLock;
use chrono::Local;
use crate::at_commands::at_tree::{construct_tree_out_of_flat_list_of_paths, PathsHolderNodeArc};

use crate::at_commands::at_commands::AtCommandsContext;
use crate::tools::tools_description::Tool;
use crate::call_validation::{ChatMessage, ChatContent, ContextEnum, ChatUsage};
use crate::subchat::subchat;
use crate::global_context::GlobalContext;
use crate::files_correction::{get_project_dirs, paths_from_anywhere};
use std::path::PathBuf;
use crate::files_in_workspace::{get_file_text_from_memory_or_disk, ls_files};
use crate::call_validation::{ContextFile, PostprocessSettings};
use crate::postprocessing::pp_context_files::postprocess_context_files;
use crate::cached_tokenizers;

#[derive(Debug, Clone, Hash, Eq, PartialEq)]
struct ExplorationTarget {
    target_type: String,
    target_name: String,
}

#[derive(Debug)]
struct ExplorationState {
    // What we've found
    entries: Vec<String>,
    entries_count: usize,
    
    // What we've explored and what's left
    explored: HashSet<ExplorationTarget>,
    to_explore: Vec<ExplorationTarget>,
    // Tree of project directories based on the flat list
    project_tree: Option<Vec<PathsHolderNodeArc>>,
}

impl ExplorationState {
    // Get tree statistics to make relative decisions
    fn get_tree_stats(tree: &Vec<PathsHolderNodeArc>) -> (usize, f64) {
        fn get_depth_and_sizes(node: &PathsHolderNodeArc) -> (usize, Vec<usize>) {
            let node_lock = node.read();
            let children = node_lock.child_paths();
            
            if children.is_empty() {
                (1, vec![1])
            } else {
                let child_results: Vec<(usize, Vec<usize>)> = children.iter()
                    .map(|child| get_depth_and_sizes(child))
                    .collect();
                
                let max_depth = 1 + child_results.iter().map(|(depth, _)| *depth).max().unwrap_or(0);
                let mut all_sizes: Vec<usize> = vec![children.len()];
                for (_, sizes) in child_results {
                    all_sizes.extend(sizes);
                }
                
                (max_depth, all_sizes)
            }
        }

        let mut all_depths_and_sizes = Vec::new();
        for node in tree {
            let (depth, sizes) = get_depth_and_sizes(node);
            all_depths_and_sizes.push((depth, sizes));
        }

        let max_depth = all_depths_and_sizes.iter()
            .map(|(depth, _)| *depth)
            .max()
            .unwrap_or(1);
            
        let avg_size = all_depths_and_sizes.iter()
            .flat_map(|(_, sizes)| sizes)
            .sum::<usize>() as f64 / 
            all_depths_and_sizes.iter()
            .flat_map(|(_, sizes)| sizes)
            .count() as f64;

        (max_depth, avg_size)
    }

    // Calculate importance score for a directory
    fn calculate_importance_score(
        node: &PathsHolderNodeArc,
        depth: usize,
        max_tree_depth: usize,
        avg_dir_size: f64,
        _project_dirs: &[std::path::PathBuf],  // Not needed as tree is already filtered
    ) -> Option<f64> {
        let node_lock = node.read();
        
        // Skip hidden directories
        if node_lock.file_name().starts_with('.') {
            return None;
        }
        
        // Consider a node a directory if it has child paths
        if node_lock.child_paths().is_empty() {
            return None;
        }

        // Calculate relative depth (0.0 = root, 1.0 = deepest possible)
        let relative_depth = depth as f64 / max_tree_depth as f64;
        
        // Count direct and total children
        let direct_children = node_lock.child_paths().len() as f64;
        let total_children: f64 = {
            fn count_recursive(n: &PathsHolderNodeArc) -> usize {
                let n_lock = n.read();
                let direct = n_lock.child_paths().len();
                let nested: usize = n_lock.child_paths().iter().map(count_recursive).sum();
                direct + nested
            }
            count_recursive(node) as f64
        };
        
        // Calculate importance score components:
        // 1. Depth score: higher for directories closer to root
        let depth_score = 1.0 - relative_depth;
        
        // 2. Size score: based on number of children relative to average
        let size_score = (direct_children + total_children) / (avg_dir_size * (1.0 + relative_depth));
        
        // 3. Root proximity bonus
        let root_bonus = if relative_depth < 0.2 { 0.5 } else { 0.0 };
        
        // Combine scores
        Some(depth_score * 0.5 + size_score * 0.3 + root_bonus)
    }

    // Helper: traverse the project tree to collect exploration targets.
    async fn collect_targets_from_tree(
        tree: &Vec<PathsHolderNodeArc>,
        gcx: Arc<ARwLock<GlobalContext>>,
    ) -> Vec<ExplorationTarget> {
        let (max_depth, avg_size) = Self::get_tree_stats(tree);
        let mut scored_targets = Vec::new();
        
        // Get project directories
        let project_dirs = get_project_dirs(gcx.clone()).await;
        
        fn traverse(
            node: &PathsHolderNodeArc,
            scored_targets: &mut Vec<(ExplorationTarget, f64)>,
            depth: usize,
            max_depth: usize,
            avg_size: f64,
            project_dirs: &[std::path::PathBuf],
        ) {
            if let Some(score) = ExplorationState::calculate_importance_score(node, depth, max_depth, avg_size, project_dirs) {
                let node_lock = node.read();
                // The path is already relative to project root
                let relative_path = node.read().get_path().to_string_lossy().to_string();

                scored_targets.push((
                    ExplorationTarget {
                        target_type: "directory".to_string(),
                        target_name: relative_path,
                    },
                    score
                ));
                
                // Continue traversing children
                for child in node_lock.child_paths() {
                    traverse(child, scored_targets, depth + 1, max_depth, avg_size, project_dirs);
                }
            }
        }
        
        for node in tree {
            traverse(node, &mut scored_targets, 0, max_depth, avg_size, &project_dirs);
        }
        
        // Sort by importance score in descending order
        scored_targets.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
        
        // Return only the targets, scores were just for sorting
        scored_targets.into_iter().map(|(target, _)| target).collect()
    }

    async fn new(gcx: Arc<ARwLock<GlobalContext>>) -> Result<Self, String> {
        // Get all paths and project directories
        let paths_from_anywhere = paths_from_anywhere(gcx.clone()).await;
        let project_dirs = get_project_dirs(gcx.clone()).await;

        // Filter paths to only those within project directories and convert them to relative paths
        let filtered_paths: Vec<PathBuf> = paths_from_anywhere.into_iter()
            .filter_map(|path| {
                project_dirs.iter()
                    .find(|project_dir| path.starts_with(project_dir))
                    .map(|project_dir| {
                        path.strip_prefix(project_dir).unwrap_or(&path).to_path_buf()
                    })
            })
            .collect();

        // Build the project tree from the filtered paths
        let tree = construct_tree_out_of_flat_list_of_paths(&filtered_paths);
        
        // Use the tree data as the main source of exploration targets
        let to_explore = Self::collect_targets_from_tree(&tree, gcx.clone()).await;

        Ok(Self {
            entries: vec![],
            entries_count: 0,
            explored: HashSet::new(),
            to_explore,
            project_tree: Some(tree),
        })
    }

    fn get_next_target(&self) -> Option<ExplorationTarget> {
        self.to_explore.first().cloned()
    }

    fn mark_explored(&mut self, target: ExplorationTarget) {
        self.explored.insert(target.clone());
        self.to_explore.retain(|x| x != &target);
    }

    fn has_unexplored_targets(&self) -> bool {
        !self.to_explore.is_empty()
    }

    fn get_entries_summary(&self) -> String {
        if self.entries.is_empty() {
            "No entries created yet.".to_string()
        } else {
            format!("Previously discovered and documented:\n{}", self.entries.join("\n"))
        }
    }

    fn get_exploration_summary(&self) -> String {
        let mut summary = vec![];
        
        // Count by type
        let mut dirs = 0;
        let mut modules = 0;
        let mut classes = 0;
        
        for target in &self.explored {
            match target.target_type.as_str() {
                "directory" => dirs += 1,
                "module" => modules += 1,
                "class" => classes += 1,
                _ => {}
            }
        }

        summary.push(format!("Created {} knowledge entries", self.entries_count));
        summary.push(format!("Explored {} directories", dirs));
        summary.push(format!("Explored {} modules", modules));
        summary.push(format!("Explored {} classes", classes));

        summary.join(". ")
    }

    fn project_tree_summary(&self) -> String {
        // If our project tree exists, recursively print a simple tree
        if let Some(tree_nodes) = &self.project_tree {
            fn traverse(node: &PathsHolderNodeArc, depth: usize) -> String {
                let node_lock = node.read();
                let indent = "  ".repeat(depth);
                let name = node_lock.file_name();
                let mut result = format!("{}{}\n", indent, name);
                for child in node_lock.child_paths() {
                    result.push_str(&traverse(child, depth + 1));
                }
                result
            }
            let mut summary = String::new();
            for node in tree_nodes {
                summary.push_str(&traverse(node, 0));
            }
            summary
        } else {
            "".to_string()
        }
    }
}

async fn read_and_compress_directory(
    gcx: Arc<ARwLock<GlobalContext>>,
    dir_relative: String,
    tokens_limit: usize,
    model: String,
) -> Result<String, String> {
    // Get the project directories and pick the first one as base
    let project_dirs = get_project_dirs(gcx.clone()).await;
    let base_dir = project_dirs.get(0)
        .ok_or("No project directory found")?;
    let abs_dir = base_dir.join(&dir_relative);

    // List files in the directory (non-recursively)
    let indexing_everywhere = crate::files_blocklist::reload_indexing_everywhere_if_needed(gcx.clone()).await;
    let files = ls_files(&indexing_everywhere, &abs_dir, false).unwrap_or(vec![]);
    if files.is_empty() {
        return Ok("Directory is empty; no files to read.".to_string());
    }

    // For each file, read its content and build a ContextFile
    let mut context_files = vec![];
    for f in files {
        let text = get_file_text_from_memory_or_disk(gcx.clone(), &f)
            .await
            .unwrap_or_else(|_| "".to_string());
        let lines = text.lines().count().max(1);
        context_files.push(ContextFile {
            file_name: f.to_string_lossy().to_string(),
            file_content: text,
            line1: 1,
            line2: lines,
            symbols: vec![],
            gradient_type: -1,
            usefulness: 0.0,
        });
    }

    // Get tokenizer
    let caps = gcx.read().await.caps.clone()
        .ok_or("No caps available")?;
    let tokenizer = cached_tokenizers::cached_tokenizer(caps, gcx.clone(), model)
        .await
        .map_err(|e| format!("Tokenizer error: {}", e))?;

    // Use default postprocessing settings
    let mut pp_settings = PostprocessSettings::new();
    pp_settings.max_files_n = context_files.len();
    let compressed = postprocess_context_files(
        gcx.clone(),
        &mut context_files,
        tokenizer,
        tokens_limit,
        false,
        &pp_settings,
    ).await;

    // Format the output
    let mut out = String::new();
    for cf in compressed {
        out.push_str(&format!("Filename: {}\n```\n{}\n```\n\n", cf.file_name, cf.file_content));
    }
    Ok(out)
}

impl ToolCreateMemoryBank {

    fn build_step_prompt(
        state: &ExplorationState,
        target: &ExplorationTarget,
        file_context: Option<&String>,
    ) -> String {
        let mut prompt = String::new();

        // Include the main system instructions
        prompt.push_str(MB_SYSTEM_PROMPT);

        // Append a summary of already documented findings
        prompt.push_str("\n\nPreviously documented:\n");
        prompt.push_str(&state.get_entries_summary());

        // Append context for the current exploration target
        prompt.push_str(&format!("\n\nNow exploring {}: '{}'", target.target_type, target.target_name));

        // For directories, specify what to look for and show the current project structure
        if target.target_type == "directory" {
            prompt.push_str("\nFocus on collecting details about:");
            prompt.push_str("\n- Directory purpose and overall organization");
            prompt.push_str("\n- Notable files or subdirectories");
            prompt.push_str("\n- Naming conventions and file patterns");
            prompt.push_str("\n\nCurrent project structure:\n");
            prompt.push_str(&state.project_tree_summary());

            if let Some(files) = file_context {
                prompt.push_str("\n\nFiles context:\n");
                prompt.push_str(files);
            }
        }

        prompt
    }
}

pub struct ToolCreateMemoryBank;

const MB_SYSTEM_PROMPT: &str = r###"You are an expert software architect specializing in project directory analysis.

Your goal is to deeply examine a specific folder within the project. I will provide its relative path.

I will provide you with:
1. The directory structure showing all files and subdirectories
2. The content of all files in the directory (automatically read and compressed)

Based on this information, analyze:
   - The folder's main purpose and organization
   - Notable files or subdirectories
   - Naming or file content patterns
   - How the folder supports the overall project

IMPORTANT:
  - Use both the directory structure and file contents for your analysis
  - Focus on concrete findings and patterns
  - If nothing new is found, state so"###;

const MB_EXPERT_WRAP_UP: &str = r###"Сreate a knowledge entries about this directory calling create_knowledge() with following arguments:
- im_going_to_use_tools: List which tree() and cat() commands you used
- im_going_to_apply_to: The directory's relative path
- goal: What you discovered about the directory's purpose and organization
- language_slash_framework: Main technologies found in the directory
- knowledge_entry: A detailed analysis including:
  * Directory's main purpose and role
  * Key files and their purposes
  * Organization patterns found
  * Notable contents discovered in files
  * How this directory integrates with the project

Focus on concrete findings from the files you examined. If no significant information was found, state "No new discoveries found."
Do not repeat previously documented information."###;

#[async_trait]
impl Tool for ToolCreateMemoryBank {
    fn as_any(&self) -> &dyn std::any::Any { self }
    
    async fn tool_execute(
        &mut self,
        ccx: Arc<AMutex<AtCommandsContext>>,
        tool_call_id: &String,
        _args: &HashMap<String, Value>
    ) -> Result<(bool, Vec<ContextEnum>), String> {
        let gcx = ccx.lock().await.global_context.clone();
        let params = crate::tools::tools_execute::unwrap_subchat_params(ccx.clone(), "create_memory_bank").await?;
        
        // Create subchat context
        let ccx_subchat = {
            let ccx_lock = ccx.lock().await;
            let mut t = AtCommandsContext::new(
                ccx_lock.global_context.clone(),
                params.subchat_n_ctx,
                7,  // top_n
                false,
                ccx_lock.messages.clone(),
                ccx_lock.chat_id.clone(),
                ccx_lock.should_execute_remotely,
            ).await;
            t.subchat_tx = ccx_lock.subchat_tx.clone();
            t.subchat_rx = ccx_lock.subchat_rx.clone();
            Arc::new(AMutex::new(t))
        };

        // Initialize exploration state
        let mut state = ExplorationState::new(gcx.clone()).await?;
        let mut final_results = vec![];
        let mut step = 0;
        let max_steps = 100;
        let usage_collector = ChatUsage { ..Default::default() };

        // Continue until we've explored everything or hit max steps
        while state.has_unexplored_targets() && step < max_steps {
            step += 1;
            let log_prefix = Local::now().format("%Y%m%d-%H%M%S").to_string();
            tracing::info!("Memory bank step {}/{}", step, max_steps);

            if let Some(target) = state.get_next_target() {
                let mut step_msgs = vec![];
                // For directories, read and compress their files
                let file_context = if target.target_type == "directory" {
                    match read_and_compress_directory(
                        gcx.clone(),
                        target.target_name.clone(),
                        params.subchat_tokens_for_rag,
                        params.subchat_model.clone(),
                    ).await {
                        Ok(txt) => Some(txt),
                        Err(e) => {
                            tracing::warn!("Failed to read/compress files for {}: {}", target.target_name, e);
                            None
                        }
                    }
                } else {
                    None
                };

                // Build the exploration prompt with file context if available
                step_msgs.push(ChatMessage::new(
                    "user".to_string(),
                    Self::build_step_prompt(&state, &target, file_context.as_ref())
                ));
                _ = subchat(
                    ccx_subchat.clone(),
                    params.subchat_model.as_str(),
                    step_msgs,
                    vec!["create_knowledge".to_string()],  // Only need create_knowledge since we provide structure and content
                    8,  // Allow 8 steps of exploration
                    params.subchat_max_new_tokens,
                    MB_EXPERT_WRAP_UP,  // Use the wrap-up prompt to instruct creating the knowledge entry
                    1,  // One completion
                    None,  // No temperature adjustment for o3
                    Some(tool_call_id.clone()),
                    Some(format!("{log_prefix}-memory-bank-step{}", step)),
                    Some(false),  // Do not prepend the system prompt automatically
                ).await?[0].clone();
                state.mark_explored(target.clone());
            } else {
                break;
            }
        }

        // Add final summary
        final_results.push(ContextEnum::ChatMessage(ChatMessage {
            role: "tool".to_string(),
            content: ChatContent::SimpleText(format!(
                "Memory bank creation completed after {} steps. {}. Usage: {} prompt tokens, {} completion tokens",
                step,
                state.get_exploration_summary(),
                usage_collector.prompt_tokens,
                usage_collector.completion_tokens,
            )),
            usage: Some(usage_collector),
            tool_calls: None,
            tool_call_id: tool_call_id.clone(),
            ..Default::default()
        }));

        Ok((false, final_results))
    }

    fn tool_depends_on(&self) -> Vec<String> {
        vec!["ast".to_string(), "vecdb".to_string()]
    }

    fn tool_description(&self) -> crate::tools::tools_description::ToolDesc {
        crate::tools::tools_description::ToolDesc {
            name: "create_memory_bank".to_string(),
            agentic: true,
            experimental: false,
            description: "Gathers information about the project structure (modules, file relations, classes, etc.) and saves this data into the memory bank.".to_string(),
            parameters: vec![],
            parameters_required: vec![],
        }
    }
}