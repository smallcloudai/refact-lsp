use std::mem;
use std::path::PathBuf;
use hashbrown::HashMap;

use tokio::io::AsyncWriteExt;
use tokio::fs::OpenOptions;

use crate::call_validation::DiffChunk;


#[derive(Clone, Debug, Default)]
struct DiffLine {
    line_n: usize,
    text: String,
    overwritten_by_id: Option<usize>,
}

pub async fn write_to_file(path: &String, text: &str) -> Result<(), String> {
    let mut file = OpenOptions::new()
        .create(true)
        .truncate(true)
        .write(true)
        .open(path)
        .await
        .map_err(|e| {
            format!("Failed to open file: {}", e)
        })?;

    file.write_all(text.as_bytes()).await.map_err(|e| {
        format!("Failed to write to file: {}", e)
    })?;
    Ok(())
}

fn find_chunk_matches(chunk_lines_remove: &Vec<DiffLine>, orig_lines: &Vec<&DiffLine>) -> Result<Vec<Vec<usize>>, String> {
    let chunk_len = chunk_lines_remove.len();
    let orig_len = orig_lines.len();

    if chunk_len == 0 || orig_len < chunk_len {
        return Err("Invalid input: chunk_lines is empty or orig_lines is smaller than chunk_lines".to_string());
    }

    let mut matches = vec![];
    for i in 0..=(orig_len - chunk_len) {
        let mut match_found = true;

        for j in 0..chunk_len {
            if orig_lines[i + j].text != chunk_lines_remove[j].text {
                match_found = false;
                break;
            }
        }
        if match_found {
            let positions = (i..i + chunk_len).map(|index| orig_lines[index].line_n).collect::<Vec<usize>>();
            matches.push(positions);
        }
    }
    if matches.is_empty() {
        return Err("Chunk text not found in original text".to_string());
    }
    Ok(matches)
}

fn apply_chunk_to_text_fuzzy(
    chunk_id: usize,
    lines_orig: &Vec<DiffLine>,
    chunk: &DiffChunk,
    max_fuzzy_n: usize,
) -> (Option<usize>, Vec<DiffLine>) {
    let chunk_lines_remove: Vec<_> = chunk.lines_remove.lines().map(|l| DiffLine { line_n: 0, text: l.to_string(), overwritten_by_id: None}).collect();
    let chunk_lines_add: Vec<_> = chunk.lines_add.lines().map(|l| DiffLine { line_n: 0, text: l.to_string(), overwritten_by_id: Some(chunk_id)}).collect();
    let mut new_lines = vec![];

    if chunk_lines_remove.is_empty() {
        new_lines.extend(lines_orig[..chunk.line1 - 1].iter().cloned().collect::<Vec<_>>());
        new_lines.extend(chunk_lines_add.iter().cloned().collect::<Vec<_>>());
        new_lines.extend(lines_orig[chunk.line1 - 1..].iter().cloned().collect::<Vec<_>>());
        return (Some(0), new_lines);
    }

    let mut fuzzy_n_used = 0;
    for fuzzy_n in 0..=max_fuzzy_n {
        let search_from = (chunk.line1 as i32 - fuzzy_n as i32).max(0) as usize;
        let search_till = (chunk.line2 as i32 - 1 + fuzzy_n as i32) as usize;
        let search_in_window: Vec<_> = lines_orig.iter()
            .filter(|l| l.overwritten_by_id.is_none() && l.line_n >= search_from && l.line_n <= search_till).collect();

        let matches = find_chunk_matches(&chunk_lines_remove, &search_in_window);

        let best_match = match matches {
            Ok(m) => {
                fuzzy_n_used = fuzzy_n;
                m[0].clone()
            },
            Err(_) => {
                if fuzzy_n >= max_fuzzy_n {
                    return (None, new_lines);
                }
                continue;
            }
        };

        for l in lines_orig.iter() {
            if best_match.ends_with(&[l.line_n]) {
                new_lines.extend(chunk_lines_add.clone());
            }
            if !best_match.contains(&l.line_n) {
                new_lines.push(l.clone());
            }
        }
        break;
    }
    if new_lines.is_empty() {
        return (None, new_lines)
    }
    (Some(fuzzy_n_used), new_lines)
}

fn validate_chunk(chunk: &DiffChunk) -> Result<(), String> {
    if chunk.line1 < 1 {
        return Err("Invalid line range: line1 cannot be < 1".to_string());
    }
    Ok(())
}

fn apply_chunks(
    chunks: &mut Vec<DiffChunk>,
    file_text: &String,
    max_fuzzy_n: usize,
) -> Result<(HashMap<usize, Option<usize>>, Vec<DiffLine>), String> {
    let mut lines_orig = file_text.lines().enumerate().map(|(line_n, l)| DiffLine { line_n: line_n + 1, text: l.to_string(), ..Default::default()}).collect::<Vec<_>>();
    // info!("apply_chunks lines_orig: \n\n{}\n\n", lines_orig.iter().map(|x|x.text.clone()).collect::<Vec<_>>().join("\n"));

    let mut results_fuzzy_ns = HashMap::new();
    for chunk in chunks.iter_mut() {
        if !chunk.apply { continue; }

        validate_chunk(chunk)?;

        let (fuzzy_n_used, lines_orig_new) = apply_chunk_to_text_fuzzy(chunk.chunk_id, &lines_orig, &chunk, max_fuzzy_n);
        if fuzzy_n_used.is_some() {
            lines_orig = lines_orig_new;
        }
        results_fuzzy_ns.insert(chunk.chunk_id, fuzzy_n_used);
    }
    Ok((results_fuzzy_ns, lines_orig))
}

fn undo_chunks(
    chunks: &mut Vec<DiffChunk>,
    file_text: &String,
    max_fuzzy_n: usize,
) -> Result<(HashMap<usize, Option<usize>>, Vec<DiffLine>), String> {
    let mut lines_orig = file_text.lines().enumerate().map(|(line_n, l)| DiffLine { line_n: line_n + 1, text: l.to_string(), ..Default::default()}).collect::<Vec<_>>();

    let mut results_fuzzy_ns = HashMap::new();
    for chunk in chunks.iter_mut() {
        if !chunk.apply { continue; }

        validate_chunk(chunk)?;
        mem::swap(&mut chunk.lines_remove, &mut chunk.lines_add);

        chunk.line2 = chunk.line1 + chunk.lines_remove.lines().count();

        let (fuzzy_n_used, mut lines_orig_new) = apply_chunk_to_text_fuzzy(chunk.chunk_id, &lines_orig, &chunk, max_fuzzy_n);
        if fuzzy_n_used.is_some() {
            lines_orig_new = lines_orig_new.iter_mut().enumerate().map(|(idx, l)| {
                l.line_n = idx + 1;
                return l.clone();
            }).collect::<Vec<_>>();
            lines_orig = lines_orig_new;
        }
        results_fuzzy_ns.insert(chunk.chunk_id, fuzzy_n_used);
    }
    Ok((results_fuzzy_ns, lines_orig))
}

pub fn read_files_from_disk_and_patch(
    chunks: &Vec<DiffChunk>,
    chunks_undo: &Vec<DiffChunk>,
    max_fuzzy_n: usize,
) -> Result<(HashMap<String, String>, HashMap<usize, Option<usize>>), String> {
    let mut chunk_groups = HashMap::new();
    for c in chunks.iter().cloned() {
        chunk_groups.entry(c.file_name.clone()).or_insert(Vec::new()).push(c);
    }
    let mut chunk_undo_groups = HashMap::new();
    for mut c in chunks_undo.iter().cloned() {
        c.apply = true;
        chunk_undo_groups.entry(c.file_name.clone()).or_insert(Vec::new()).push(c);
    }

    let mut results = HashMap::new();
    let mut texts_after_patch = HashMap::new();

    for (file_name, chunks_group) in chunk_groups.iter_mut() {
        chunks_group.sort_by_key(|c| c.line1);

        let mut file_text = crate::files_in_workspace::read_file_from_disk_sync(&PathBuf::from(file_name)).map(|x| x.to_string())?; // XXX: not exit, next file

        if let Some(mut chunks_undo_group) = chunk_undo_groups.get(file_name).cloned() {
            let (_, new_lines) = undo_chunks(&mut chunks_undo_group, &file_text, max_fuzzy_n).map_err(|e| e.to_string())?; // XXX: only undo what is necessary
            file_text = new_lines.iter().map(|l| l.text.as_str()).collect::<Vec<_>>().join("\n");
        }

        let (fuzzy_ns, new_lines) = apply_chunks(chunks_group, &file_text, max_fuzzy_n).map_err(|e| e.to_string())?;  // XXX: not exit, next chunk
        results.extend(fuzzy_ns);

        let new_text = new_lines.iter().map(|l| l.text.as_str()).collect::<Vec<_>>().join("\n");
        texts_after_patch.insert(file_name.clone(), new_text);
    }
    Ok((texts_after_patch, results))
}


#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::io::Write;

    const TEST_MAX_FUZZY: usize = 10;

    const FILE1_FN: &str = "/tmp/file1.txt";
    const FILE1: &str = r#"# line 1
class Point2d:
    def __init__(self, x, y):
        self.x = x
        self.y = y

    def __str__(self):
        return "Point2d(x=%0.2f, y=%0.2f)" % (self.x, self.y)
"#;
const FILE2_FN: &str = "/tmp/file2.txt";
const FILE2: &str = r#"import file1
x = file1.Point2d(5, 6)
print(x)
"#;

    fn delete_file_if_exists(file_name: &str) {
        if fs::metadata(file_name).is_ok() {
            fs::remove_file(file_name).expect("Failed to delete file");
        }
    }

    fn write_file(file_name: &str, content: &str) {
        let mut file = fs::File::create(file_name).expect("Failed to create file");
        file.write_all(content.as_bytes()).expect("Failed to write to file");
    }

    #[test]
    fn test_chunks() {
        // Run this to see println:
        //     cargo test diffs::tests::test_chunks -- --nocapture
        let chunk1 = DiffChunk {
            file_name: "/tmp/file1.txt".to_string(),
            file_action: "edit".to_string(),
            line1: 4,
            line2: 5,
            lines_remove: "        self.x = x\n        self.y = y\n".to_string(),
            lines_add: "        self.x, self.y = x, y\n".to_string(),
            chunk_id: 1,
            apply: false,
        };

        let chunks_undo: Vec<DiffChunk> = vec![chunk1.clone()];

        delete_file_if_exists(FILE1_FN);
        let r1 = read_files_from_disk_and_patch(&vec![chunk1.clone()], &chunks_undo, TEST_MAX_FUZZY);
        println!("r1: {:?}", r1);
        assert!(r1.is_err());

        write_file(FILE1_FN, FILE1);
        let r2 = read_files_from_disk_and_patch(&vec![chunk1.clone()], &chunks_undo, TEST_MAX_FUZZY);
        println!("r2: {:?}", r2);
    }
}
