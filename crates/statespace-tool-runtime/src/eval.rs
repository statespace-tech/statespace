//! Component block processing for dynamic markdown content.

use crate::env_validation::is_reserved_env_key;
use crate::executor::ExecutionLimits;
use crate::sandbox::SandboxEnv;
use std::collections::HashMap;
use std::fmt::Write;
use std::path::Path;
use tokio::process::Command;
use tracing::warn;

pub const EVAL_MAX_BLOCKS_PER_DOCUMENT: usize = 32;

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
        let block_end = code_start + close_pos + 3;

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

/// Merge request-scoped env vars with trusted env vars for eval execution.
///
/// Untrusted caller-provided keys are applied first, then trusted keys are
/// layered on top so trusted values always win when names collide.
#[must_use]
#[allow(clippy::implicit_hasher)]
pub fn merge_eval_env(
    trusted_env: &HashMap<String, String>,
    untrusted_env: &HashMap<String, String>,
) -> HashMap<String, String> {
    let mut merged = HashMap::with_capacity(trusted_env.len() + untrusted_env.len());

    for (key, value) in untrusted_env {
        if !is_reserved_env_key(key) {
            merged.insert(key.clone(), value.clone());
        }
    }

    for (key, value) in trusted_env {
        if !is_reserved_env_key(key) {
            merged.insert(key.clone(), value.clone());
        }
    }

    merged
}

#[allow(clippy::implicit_hasher)]
pub async fn execute_eval_block(
    block: &EvalBlock,
    working_dir: &Path,
    scratch_dir: Option<&Path>,
    workspace_dir: Option<&Path>,
    user_env: &HashMap<String, String>,
) -> EvalResult {
    execute_eval_block_with_sandbox(
        block,
        working_dir,
        scratch_dir,
        workspace_dir,
        user_env,
        &SandboxEnv::default(),
        &ExecutionLimits::default(),
    )
    .await
}

#[allow(clippy::implicit_hasher)]
pub async fn execute_eval_block_with_sandbox(
    block: &EvalBlock,
    working_dir: &Path,
    scratch_dir: Option<&Path>,
    workspace_dir: Option<&Path>,
    user_env: &HashMap<String, String>,
    sandbox_env: &SandboxEnv,
    limits: &ExecutionLimits,
) -> EvalResult {
    let mut command = Command::new("sh");
    command
        .args(["-c", &block.code])
        .current_dir(working_dir)
        .env_clear()
        .env("PATH", sandbox_env.path())
        .env("HOME", sandbox_env.home())
        .env("LANG", sandbox_env.lang())
        .env("LC_ALL", sandbox_env.lc_all())
        .kill_on_drop(true);

    for (k, v) in user_env {
        if !is_reserved_env_key(k) {
            command.env(k, v);
        }
    }

    if let Some(dir) = scratch_dir {
        command.env("STATESPACE_SCRATCH", dir);
    }
    if let Some(dir) = workspace_dir {
        command.env("STATESPACE_WORKSPACE", dir);
    }

    let fut = command.output();

    let Ok(result) = tokio::time::timeout(limits.timeout, fut).await else {
        warn!("Eval block timed out after {:?}", limits.timeout);
        return EvalResult {
            output: format!(
                "[eval error: timed out after {}s]",
                limits.timeout.as_secs()
            ),
            success: false,
        };
    };

    match result {
        Ok(output) => {
            let stdout = String::from_utf8_lossy(&output.stdout);
            let stderr = String::from_utf8_lossy(&output.stderr);

            if output.status.success() {
                let mut out = stdout.trim_end().to_string();
                if out.len() > limits.max_output_bytes {
                    let mut limit = limits.max_output_bytes;
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

#[allow(clippy::implicit_hasher)]
pub async fn process_eval_blocks(
    content: &str,
    working_dir: &Path,
    user_env: &HashMap<String, String>,
) -> String {
    process_eval_blocks_with_sandbox(
        content,
        working_dir,
        user_env,
        &SandboxEnv::default(),
        &ExecutionLimits::default(),
    )
    .await
}

#[allow(clippy::implicit_hasher)]
pub async fn process_eval_blocks_with_sandbox(
    content: &str,
    working_dir: &Path,
    user_env: &HashMap<String, String>,
    sandbox_env: &SandboxEnv,
    limits: &ExecutionLimits,
) -> String {
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

    let user_env = std::sync::Arc::new(user_env.clone());
    let sandbox_env = std::sync::Arc::new(sandbox_env.clone());
    let limits = std::sync::Arc::new(limits.clone());
    let semaphore = std::sync::Arc::new(tokio::sync::Semaphore::new(4));
    let mut tasks = tokio::task::JoinSet::new();

    for (i, block) in blocks.into_iter().enumerate() {
        let sem = semaphore.clone();
        let wd = working_dir.to_path_buf();
        let env = user_env.clone();
        let sandbox_env = sandbox_env.clone();
        let limits = limits.clone();
        tasks.spawn(async move {
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
            let result = execute_eval_block_with_sandbox(
                &block,
                &wd,
                None,
                None,
                &env,
                &sandbox_env,
                &limits,
            )
            .await;
            (i, block.range, result)
        });
    }

    let mut outputs: Vec<(usize, (usize, usize), EvalResult)> =
        Vec::with_capacity(block_ranges.len());
    while let Some(res) = tasks.join_next().await {
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
    use std::collections::HashMap;

    use crate::eval::{is_eval_info_string, parse_eval_blocks};

    fn empty_env() -> HashMap<String, String> {
        HashMap::new()
    }

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
        let block_with_newline = "```component\necho hi\n```\n";
        let block_without_newline = "```component\necho hi\n```";
        let suffix = "\nMore text\n";
        let md = format!("{prefix}{block_with_newline}{suffix}");
        let blocks = parse_eval_blocks(&md);
        assert_eq!(blocks.len(), 1);
        assert_eq!(blocks[0].range.0, prefix.len());
        assert_eq!(
            blocks[0].range.1,
            prefix.len() + block_without_newline.len()
        );
    }

    #[tokio::test]
    async fn execute_eval_block_success() {
        use crate::eval::{EvalBlock, execute_eval_block};
        let block = EvalBlock {
            range: (0, 0),
            code: "echo hello".to_string(),
        };
        let result = execute_eval_block(
            &block,
            std::path::Path::new("/tmp"),
            None,
            None,
            &empty_env(),
        )
        .await;
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
        let result = execute_eval_block(
            &block,
            std::path::Path::new("/tmp"),
            None,
            None,
            &empty_env(),
        )
        .await;
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
        let result = execute_eval_block(
            &block,
            std::path::Path::new("/tmp"),
            None,
            None,
            &empty_env(),
        )
        .await;
        assert!(!result.success);
        assert!(result.output.contains("eval error"));
    }

    #[tokio::test]
    async fn process_replaces_component_blocks() {
        use crate::eval::process_eval_blocks;
        let md = "# Title\n\n```component\necho 42\n```\n\nEnd\n";
        let result = process_eval_blocks(md, std::path::Path::new("/tmp"), &empty_env()).await;
        assert!(result.contains("42"));
        assert!(!result.contains("```component"));
        assert!(result.contains("# Title"));
        assert!(result.contains("End"));
    }

    #[tokio::test]
    async fn process_no_component_blocks_returns_unchanged() {
        use crate::eval::process_eval_blocks;
        let md = "# Just text\n\n```json\n{}\n```\n";
        let result = process_eval_blocks(md, std::path::Path::new("/tmp"), &empty_env()).await;
        assert_eq!(result, md);
    }

    #[tokio::test]
    async fn process_multiple_blocks_replaced_in_order() {
        use crate::eval::process_eval_blocks;
        let md = "A\n\n```component\necho first\n```\n\nB\n\n```component\necho second\n```\n\nC\n";
        let result = process_eval_blocks(md, std::path::Path::new("/tmp"), &empty_env()).await;
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
            code: "while true; do :; done".to_string(),
        };
        let result = execute_eval_block(
            &block,
            std::path::Path::new("/tmp"),
            None,
            None,
            &empty_env(),
        )
        .await;
        assert!(!result.success);
        assert!(result.output.contains("timed out"));
    }

    #[tokio::test]
    async fn user_env_injected_into_subprocess() {
        use crate::eval::{EvalBlock, execute_eval_block};
        let block = EvalBlock {
            range: (0, 0),
            code: "echo $MY_SECRET".to_string(),
        };
        let env = HashMap::from([("MY_SECRET".into(), "hunter2".into())]);
        let result =
            execute_eval_block(&block, std::path::Path::new("/tmp"), None, None, &env).await;
        assert!(result.success);
        assert_eq!(result.output, "hunter2");
    }

    #[tokio::test]
    async fn reserved_env_keys_not_overridden() {
        use crate::eval::{EvalBlock, execute_eval_block};
        let block = EvalBlock {
            range: (0, 0),
            code: "echo $HOME".to_string(),
        };
        let env = HashMap::from([("HOME".into(), "/evil".into())]);
        let result =
            execute_eval_block(&block, std::path::Path::new("/tmp"), None, None, &env).await;
        assert!(result.success);
        assert_ne!(result.output, "/evil");
    }

    #[tokio::test]
    async fn process_eval_blocks_with_user_env() {
        use crate::eval::process_eval_blocks;
        let md = "```component\necho $DB\n```\n";
        let env = HashMap::from([("DB".into(), "postgresql://localhost/test".into())]);
        let result = process_eval_blocks(md, std::path::Path::new("/tmp"), &env).await;
        assert!(result.contains("postgresql://localhost/test"));
        assert!(!result.contains("```component"));
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

    #[test]
    fn merge_eval_env_trusted_overrides_untrusted() {
        use crate::eval::merge_eval_env;

        let trusted = HashMap::from([("USER_ID".to_string(), "42".to_string())]);
        let untrusted = HashMap::from([
            ("USER_ID".to_string(), "7".to_string()),
            ("PAGE".to_string(), "stats".to_string()),
        ]);

        let merged = merge_eval_env(&trusted, &untrusted);
        assert_eq!(merged.get("USER_ID"), Some(&"42".to_string()));
        assert_eq!(merged.get("PAGE"), Some(&"stats".to_string()));
    }

    #[test]
    fn merge_eval_env_filters_reserved_keys() {
        use crate::eval::merge_eval_env;

        let trusted = HashMap::from([("AWS_SECRET_ACCESS_KEY".to_string(), "x".to_string())]);
        let untrusted = HashMap::from([
            ("LD_PRELOAD".to_string(), "y".to_string()),
            ("PATH".to_string(), "/tmp/evil".to_string()),
        ]);

        let merged = merge_eval_env(&trusted, &untrusted);
        assert!(!merged.contains_key("AWS_SECRET_ACCESS_KEY"));
        assert!(!merged.contains_key("LD_PRELOAD"));
        assert!(!merged.contains_key("PATH"));
    }
}
