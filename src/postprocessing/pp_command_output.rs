use serde::Deserialize;
use regex::Regex;


#[derive(Deserialize)]
pub struct CmdlineOutputFilter {
    pub limit_lines: usize,
    pub limit_chars: usize,
    pub top_or_bottom: String,
    pub grep: String,
    pub grep_context_lines: usize,
    pub remove_from_output: String,
}

impl Default for CmdlineOutputFilter {
    fn default() -> Self {
        CmdlineOutputFilter {
            limit_lines: 100,
            limit_chars: 10000,
            top_or_bottom: "top".to_string(),
            grep: "error|warning".to_string(),
            grep_context_lines: 5,
            remove_from_output: "".to_string(),
        }
    }
}

pub fn output_mini_postprocessing(filter: &CmdlineOutputFilter, output: &str) -> String {
    let lines: Vec<&str> = output.lines().collect();
    let mut ratings: Vec<f64> = vec![0.0; lines.len()];
    let mut approve: Vec<bool> = vec![false; lines.len()];

    if !filter.grep.is_empty() {
        let re = Regex::new(&filter.grep).unwrap();
        for (i, line) in lines.iter().enumerate() {
            if re.is_match(line) {
                ratings[i] += 1.0;
                for j in 1..=filter.grep_context_lines {
                    let lower_bound = i.saturating_sub(j);
                    let upper_bound = i + j;
                    if lower_bound < lines.len() {
                        ratings[lower_bound] += 0.9;
                    }
                    if upper_bound < lines.len() {
                        ratings[upper_bound] += 0.9;
                    }
                }
            }
        }
    }

    if filter.top_or_bottom == "top" {
        for i in 0..lines.len() {
            ratings[i] += (lines.len() - i) as f64 / lines.len() as f64;
        }
    } else if filter.top_or_bottom == "bottom" {
        for i in 0..lines.len() {
            ratings[i] += i as f64 / lines.len() as f64;
        }
    }

    let mut line_indices: Vec<usize> = (0..lines.len()).collect();
    line_indices.sort_by(|&a, &b| ratings[b].partial_cmp(&ratings[a]).unwrap());

    let mut current_lines = 0;
    let mut current_chars = 0;
    let remove_re = Regex::new(&filter.remove_from_output).unwrap();

    for &index in &line_indices {
        if current_lines > filter.limit_lines || current_chars > filter.limit_chars {
            break;
        }
        if filter.remove_from_output.is_empty() || !remove_re.is_match(lines[index]) {
            if ratings[index] > 0.0 {
                approve[index] = true;
            }
            current_lines += 1;
            current_chars += lines[index].len();
        }
    }

    println!("{:#?}", lines);
    println!("{:#?}", ratings);
    println!("{:#?}", approve);

    let mut result = String::new();
    let mut skipped_lines = 0;
    for (i, &line) in lines.iter().enumerate() {
        if approve[i] {
            if skipped_lines > 0 {
                result.push_str(&format!("...{} lines skipped...\n", skipped_lines));
                skipped_lines = 0;
            }
            result.push_str(line);
            result.push('\n');
        } else {
            skipped_lines += 1;
        }
    }
    if skipped_lines > 0 {
        result.push_str(&format!("...{} lines skipped...\n", skipped_lines));
    }
    result
}


#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_cmdline_output_filter() {
        let output_to_filter = r#"line1
line2
line3
line4
line5
line6
"#;

        let result = output_mini_postprocessing(&CmdlineOutputFilter {
            limit_lines: 2,
            limit_chars: 1000,
            top_or_bottom: "top".to_string(),
            grep: "".to_string(),
            grep_context_lines: 1,
            remove_from_output: "".to_string(),
        }, output_to_filter);
        assert_eq!(result, "line1\nline2\nline3\n...3 lines skipped...\n");

        let result = output_mini_postprocessing(&CmdlineOutputFilter {
            limit_lines: 2,
            limit_chars: 1000,
            top_or_bottom: "bottom".to_string(),
            grep: "".to_string(),
            grep_context_lines: 1,
            remove_from_output: "".to_string(),
        }, output_to_filter);
        assert_eq!(result, "...3 lines skipped...\nline4\nline5\nline6\n");

        let result = output_mini_postprocessing(&CmdlineOutputFilter {
            limit_lines: 3,
            limit_chars: 1000,
            top_or_bottom: "".to_string(),
            grep: "line4".to_string(),
            grep_context_lines: 1,
            remove_from_output: "".to_string(),
        }, output_to_filter);
        assert_eq!(result, "...2 lines skipped...\nline3\nline4\nline5\n...1 lines skipped...\n");
    }
}

