//! Component block processing for dynamic markdown content.

use std::fmt::Write;
use std::path::Path;
use std::time::Duration;
use tokio::process::Command;
use tracing::warn;

pub const EVAL_BLOCK_TIMEOUT: Duration = Duration::from_secs(5);
pub const EVAL_MAX_BLOCKS_PER_DOCUMENT: usize = 20;
pub const EVAL_MAX_OUTPUT_BYTES: usize = 1024 * 1024; // 1MB

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EvalBlock {
    /// Byte range of the entire fenced block (including the ``` delimiters).
    pub range: (usize, usize),
    pub code: String,
}

#[derive(Debug, Clone)]
pub struct EvalResult {
    pub output: String,
    pub success: bool,
}

pub fn parse_eval_blocks(content: &str) -> Vec<EvalBlock> {
    let mut blocks = Vec::new();
    let mut search_from = 0;

    while let Some(block) = find_next_eval_block(content, search_from) {
        search_from = block.range.1;
        blocks.push(block);
    }

    blocks
}

fn find_next_eval_block(content: &str, start: usize) -> Option<EvalBlock> {
    let haystack = &content[start..];

    let mut pos = 0;
    loop {
        let remaining = &haystack[pos..];
        let fence_pos = remaining.find("```")?;
        let abs_fence_start = start + pos + fence_pos;

        if abs_fence_start > 0 && content.as_bytes()[abs_fence_start - 1] != b'\n' {
            pos += fence_pos + 3;
            continue;
        }

        let after_backticks = &content[abs_fence_start + 3..];

        let Some(newline_pos) = after_backticks.find('\n') else {
            pos += fence_pos + 3;
            continue;
        };

        let info_string = after_backticks[..newline_pos].trim();

        if !is_eval_info_string(info_string) {
            pos += fence_pos + 3;
            continue;
        }

        let code_start = abs_fence_start + 3 + newline_pos + 1;
        let code_region = &content[code_start..];
        let close_pos = find_closing_fence(code_region)?;
        let code = &content[code_start..code_start + close_pos];
        let block_end = code_start + close_pos + 3; // skip closing ```

        let block_end = if block_end < content.len() && content.as_bytes()[block_end] == b'\n' {
            block_end + 1
        } else {
            block_end
        };

        return Some(EvalBlock {
            range: (abs_fence_start, block_end),
            code: code.trim_end_matches('\n').to_string(),
        });
    }
}

fn find_closing_fence(content: &str) -> Option<usize> {
    let mut pos = 0;
    loop {
        let remaining = &content[pos..];
        let fence_pos = remaining.find("```")?;
        let abs_pos = pos + fence_pos;

        if abs_pos == 0 || content.as_bytes()[abs_pos - 1] == b'\n' {
            return Some(abs_pos);
        }

        pos = abs_pos + 3;
    }
}

fn is_eval_info_string(info: &str) -> bool {
    info == "component"
}

pub async fn execute_eval_block(
    block: &EvalBlock,
    working_dir: &Path,
    scratch_dir: Option<&Path>,
    workspace_dir: Option<&Path>,
) -> EvalResult {
    let mut command = Command::new("sh");
    command
        .args(["-c", &block.code])
        .current_dir(working_dir)
        .env_clear()
        .env("PATH", "/usr/local/bin:/usr/bin:/bin")
        .env("HOME", "/tmp")
        .env("LANG", "C.UTF-8")
        .kill_on_drop(true);

    if let Some(dir) = scratch_dir {
        command.env("STATESPACE_SCRATCH", dir);
    }
    if let Some(dir) = workspace_dir {
        command.env("STATESPACE_WORKSPACE", dir);
    }

    let fut = command.output();

    let Ok(result) = tokio::time::timeout(EVAL_BLOCK_TIMEOUT, fut).await else {
        warn!("Eval block timed out after {:?}", EVAL_BLOCK_TIMEOUT);
        return EvalResult {
            output: "[eval error: timed out after 5s]".to_string(),
            success: false,
        };
    };

    match result {
        Ok(output) => {
            let stdout = String::from_utf8_lossy(&output.stdout);
            let stderr = String::from_utf8_lossy(&output.stderr);

            if output.status.success() {
                let mut out = stdout.trim_end().to_string();
                if out.len() > EVAL_MAX_OUTPUT_BYTES {
                    let mut limit = EVAL_MAX_OUTPUT_BYTES;
                    while !out.is_char_boundary(limit) {
                        limit -= 1;
                    }
                    out.truncate(limit);
                }
                EvalResult {
                    output: out,
                    success: true,
                }
            } else {
                let code = output.status.code().unwrap_or(-1);
                let mut msg = format!("[eval error: exit {code}");
                let combined = if stderr.is_empty() {
                    stdout.trim_end().to_string()
                } else {
                    stderr.trim_end().to_string()
                };
                if !combined.is_empty() {
                    let mut detail = combined;
                    if detail.len() > 256 {
                        let mut limit = 256;
                        while !detail.is_char_boundary(limit) {
                            limit -= 1;
                        }
                        detail.truncate(limit);
                        detail.push('…');
                    }
                    let _ = write!(msg, " — {detail}");
                }
                msg.push(']');
                warn!(exit_code = code, "Eval block failed");
                EvalResult {
                    output: msg,
                    success: false,
                }
            }
        }
        Err(e) => {
            warn!(error = %e, "Eval block execution failed");
            EvalResult {
                output: format!("[eval error: {e}]"),
                success: false,
            }
        }
    }
}

pub async fn process_eval_blocks(content: &str, working_dir: &Path) -> String {
    let mut blocks = parse_eval_blocks(content);

    if blocks.is_empty() {
        return content.to_string();
    }

    if blocks.len() > EVAL_MAX_BLOCKS_PER_DOCUMENT {
        warn!(
            count = blocks.len(),
            limit = EVAL_MAX_BLOCKS_PER_DOCUMENT,
            "Truncating eval blocks to limit"
        );
        blocks.truncate(EVAL_MAX_BLOCKS_PER_DOCUMENT);
    }

    let block_ranges: Vec<(usize, (usize, usize))> = blocks
        .iter()
        .enumerate()
        .map(|(i, b)| (i, b.range))
        .collect();

    let sem = std::sync::Arc::new(tokio::sync::Semaphore::new(4));
    let mut set = tokio::task::JoinSet::new();

    for (i, block) in blocks.into_iter().enumerate() {
        let sem = sem.clone();
        let wd = working_dir.to_path_buf();
        set.spawn(async move {
            let Ok(_permit) = sem.acquire().await else {
                return (
                    i,
                    block.range,
                    EvalResult {
                        output: "[eval error: internal]".to_string(),
                        success: false,
                    },
                );
            };
            let result = execute_eval_block(&block, &wd, None, None).await;
            (i, block.range, result)
        });
    }

    let mut outputs: Vec<(usize, (usize, usize), EvalResult)> =
        Vec::with_capacity(block_ranges.len());
    while let Some(res) = set.join_next().await {
        match res {
            Ok(item) => outputs.push(item),
            Err(e) => {
                warn!("eval block task panicked: {e}");
            }
        }
    }

    let completed: std::collections::HashSet<usize> = outputs.iter().map(|(i, _, _)| *i).collect();
    for (i, range) in &block_ranges {
        if !completed.contains(i) {
            outputs.push((
                *i,
                *range,
                EvalResult {
                    output: "[eval error: internal failure]".to_string(),
                    success: false,
                },
            ));
        }
    }

    outputs.sort_by(|a, b| b.1.0.cmp(&a.1.0));

    let mut result = content.to_string();
    for (_, (start, end), eval_result) in &outputs {
        result.replace_range(*start..*end, &eval_result.output);
    }

    result
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use crate::eval::{is_eval_info_string, parse_eval_blocks};

    #[test]
    fn info_string_component() {
        assert!(is_eval_info_string("component"));
    }

    #[test]
    fn info_string_rejects_non_component() {
        assert!(!is_eval_info_string("eval"));
        assert!(!is_eval_info_string("rust"));
        assert!(!is_eval_info_string("json"));
        assert!(!is_eval_info_string(""));
    }

    #[test]
    fn parse_single_component_block() {
        let md = "# Title\n\n```component\necho hello\n```\n\nMore text\n";
        let blocks = parse_eval_blocks(md);
        assert_eq!(blocks.len(), 1);
        assert_eq!(blocks[0].code, "echo hello");
    }

    #[test]
    fn parse_multiple_component_blocks() {
        let md = "```component\necho one\n```\n\ntext\n\n```component\necho two\n```\n";
        let blocks = parse_eval_blocks(md);
        assert_eq!(blocks.len(), 2);
        assert_eq!(blocks[0].code, "echo one");
        assert_eq!(blocks[1].code, "echo two");
    }

    #[test]
    fn skip_non_component_code_blocks() {
        let md = "```rust\nfn main() {}\n```\n\n```component\necho hi\n```\n";
        let blocks = parse_eval_blocks(md);
        assert_eq!(blocks.len(), 1);
        assert_eq!(blocks[0].code, "echo hi");
    }

    #[test]
    fn no_component_blocks() {
        let md = "# Just a doc\n\nSome text.\n\n```json\n{}\n```\n";
        let blocks = parse_eval_blocks(md);
        assert!(blocks.is_empty());
    }

    #[test]
    fn multiline_component_block() {
        let md = "```component\necho hello\necho world\n```\n";
        let blocks = parse_eval_blocks(md);
        assert_eq!(blocks.len(), 1);
        assert_eq!(blocks[0].code, "echo hello\necho world");
    }

    #[test]
    fn component_block_preserves_range() {
        let prefix = "# Title\n\n";
        let block = "```component\necho hi\n```\n";
        let suffix = "\nMore text\n";
        let md = format!("{prefix}{block}{suffix}");
        let blocks = parse_eval_blocks(&md);
        assert_eq!(blocks.len(), 1);
        assert_eq!(blocks[0].range.0, prefix.len());
        assert_eq!(blocks[0].range.1, prefix.len() + block.len());
    }

    #[tokio::test]
    async fn execute_eval_block_success() {
        use crate::eval::{EvalBlock, execute_eval_block};
        let block = EvalBlock {
            range: (0, 0),
            code: "echo hello".to_string(),
        };
        let result = execute_eval_block(&block, std::path::Path::new("/tmp"), None, None).await;
        assert!(result.success);
        assert_eq!(result.output, "hello");
    }

    #[tokio::test]
    async fn execute_eval_block_failure() {
        use crate::eval::{EvalBlock, execute_eval_block};
        let block = EvalBlock {
            range: (0, 0),
            code: "exit 42".to_string(),
        };
        let result = execute_eval_block(&block, std::path::Path::new("/tmp"), None, None).await;
        assert!(!result.success);
        assert!(result.output.contains("eval error"));
        assert!(result.output.contains("42"));
    }

    #[tokio::test]
    async fn execute_eval_block_command_not_found() {
        use crate::eval::{EvalBlock, execute_eval_block};
        let block = EvalBlock {
            range: (0, 0),
            code: "nonexistent_command_xyz_123".to_string(),
        };
        let result = execute_eval_block(&block, std::path::Path::new("/tmp"), None, None).await;
        assert!(!result.success);
        assert!(result.output.contains("eval error"));
    }

    #[tokio::test]
    async fn process_replaces_component_blocks() {
        use crate::eval::process_eval_blocks;
        let md = "# Title\n\n```component\necho 42\n```\n\nEnd\n";
        let result = process_eval_blocks(md, std::path::Path::new("/tmp")).await;
        assert!(result.contains("42"));
        assert!(!result.contains("```component"));
        assert!(result.contains("# Title"));
        assert!(result.contains("End"));
    }

    #[tokio::test]
    async fn process_no_component_blocks_returns_unchanged() {
        use crate::eval::process_eval_blocks;
        let md = "# Just text\n\n```json\n{}\n```\n";
        let result = process_eval_blocks(md, std::path::Path::new("/tmp")).await;
        assert_eq!(result, md);
    }

    #[tokio::test]
    async fn process_multiple_blocks_replaced_in_order() {
        use crate::eval::process_eval_blocks;
        let md = "A\n\n```component\necho first\n```\n\nB\n\n```component\necho second\n```\n\nC\n";
        let result = process_eval_blocks(md, std::path::Path::new("/tmp")).await;
        let first_pos = result.find("first").expect("first should be present");
        let second_pos = result.find("second").expect("second should be present");
        assert!(first_pos < second_pos);
        assert!(result.contains("A\n"));
        assert!(result.contains("B\n"));
        assert!(result.contains("C\n"));
    }

    #[tokio::test]
    async fn execute_eval_block_timeout() {
        use crate::eval::{EvalBlock, execute_eval_block};
        let block = EvalBlock {
            range: (0, 0),
            code: "sleep 10".to_string(),
        };
        let result = execute_eval_block(&block, std::path::Path::new("/tmp"), None, None).await;
        assert!(!result.success);
        assert!(result.output.contains("timed out"));
    }

    #[test]
    fn parse_finds_all_component_blocks_beyond_limit() {
        use std::fmt::Write;
        let mut md = String::new();
        for i in 0..25 {
            let _ = write!(md, "```component\necho {i}\n```\n\n");
        }
        let blocks = parse_eval_blocks(&md);
        assert_eq!(blocks.len(), 25);
    }
}
