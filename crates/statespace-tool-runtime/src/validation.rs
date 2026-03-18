//! Command validation and env expansion.

use crate::error::Error;
use crate::frontmatter::Frontmatter;
use crate::spec::{ToolPart, ToolSpec, find_matching_spec, is_valid_tool_call};
use std::collections::HashMap;

/// # Errors
///
/// Returns an error when the command is empty or not present in frontmatter.
pub fn validate_command(frontmatter: &Frontmatter, command: &[String]) -> Result<(), Error> {
    if command.is_empty() {
        return Err(Error::InvalidCommand("command cannot be empty".to_string()));
    }

    if !frontmatter.has_tool(command) {
        return Err(Error::CommandNotFound {
            command: command.join(" "),
        });
    }

    Ok(())
}

/// # Errors
///
/// Returns an error when the command is empty or does not match any spec.
pub fn validate_command_with_specs(specs: &[ToolSpec], command: &[String]) -> Result<(), Error> {
    if command.is_empty() {
        return Err(Error::InvalidCommand("command cannot be empty".to_string()));
    }

    if !is_valid_tool_call(command, specs) {
        return Err(Error::CommandNotFound {
            command: command.join(" "),
        });
    }

    Ok(())
}

#[must_use]
pub fn expand_env_vars<S: std::hash::BuildHasher>(
    command: &[String],
    env: &HashMap<String, String, S>,
) -> Vec<String> {
    command
        .iter()
        .map(|part| {
            let mut result = part.clone();

            for (key, value) in env {
                let var = format!("${key}");
                result = result.replace(&var, value);
            }

            result
        })
        .collect()
}

fn expand_literal_segment<S: std::hash::BuildHasher>(
    segment: &str,
    env: &HashMap<String, String, S>,
) -> String {
    let mut expanded = segment.to_string();
    for (key, value) in env {
        let variable = format!("${key}");
        expanded = expanded.replace(&variable, value);
    }
    expanded
}

/// Expand trusted env only in spec-declared literal `$VAR` segments.
///
/// Placeholder-derived arguments remain opaque to prevent secret expansion in
/// caller-controlled values.
#[must_use]
pub fn expand_command_for_execution<S: std::hash::BuildHasher>(
    command: &[String],
    specs: &[ToolSpec],
    env: &HashMap<String, String, S>,
) -> Vec<String> {
    if let Some(spec) = find_matching_spec(command, specs) {
        return command
            .iter()
            .enumerate()
            .map(|(index, part)| match spec.parts.get(index) {
                Some(ToolPart::Literal(literal)) if literal == part && literal.contains('$') => {
                    expand_literal_segment(part, env)
                }
                _ => part.clone(),
            })
            .collect();
    }

    command.to_vec()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::spec::ToolPart;

    fn legacy_frontmatter(tools: Vec<Vec<String>>) -> Frontmatter {
        Frontmatter {
            specs: vec![],
            tools,
        }
    }

    #[test]
    fn test_validate_command_empty() {
        let fm = legacy_frontmatter(vec![]);
        let result = validate_command(&fm, &[]);
        assert!(matches!(result, Err(Error::InvalidCommand(_))));
    }

    #[test]
    fn test_validate_command_not_found() {
        let fm = legacy_frontmatter(vec![vec!["ls".to_string()]]);

        let result = validate_command(&fm, &["cat".to_string(), "file.md".to_string()]);
        assert!(matches!(result, Err(Error::CommandNotFound { .. })));
    }

    #[test]
    fn test_validate_command_success() {
        let fm = legacy_frontmatter(vec![
            vec!["ls".to_string(), "{path}".to_string()],
            vec!["cat".to_string(), "{path}".to_string()],
        ]);

        let result = validate_command(&fm, &["ls".to_string(), "docs/".to_string()]);
        assert!(result.is_ok());

        let result = validate_command(&fm, &["cat".to_string(), "index.md".to_string()]);
        assert!(result.is_ok());
    }

    #[test]
    fn test_expand_env_vars() {
        let command = vec![
            "curl".to_string(),
            "-H".to_string(),
            "Authorization: Bearer $API_KEY".to_string(),
        ];

        let mut env = HashMap::new();
        env.insert("API_KEY".to_string(), "secret123".to_string());

        let expanded = expand_env_vars(&command, &env);
        assert_eq!(
            expanded,
            vec!["curl", "-H", "Authorization: Bearer secret123"]
        );
    }

    #[test]
    fn test_expand_command_for_execution_expands_matching_literal_segments() {
        let specs = vec![ToolSpec {
            parts: vec![
                ToolPart::Literal("echo".to_string()),
                ToolPart::Literal("$SECRET".to_string()),
            ],
            options_disabled: false,
        }];
        let command = vec!["echo".to_string(), "$SECRET".to_string()];
        let env = HashMap::from([("SECRET".to_string(), "trusted".to_string())]);

        let expanded = expand_command_for_execution(&command, &specs, &env);

        assert_eq!(expanded, vec!["echo", "trusted"]);
    }

    #[test]
    fn test_expand_command_for_execution_does_not_expand_placeholder_segments() {
        let specs = vec![ToolSpec {
            parts: vec![
                ToolPart::Literal("echo".to_string()),
                ToolPart::Placeholder { regex: None },
            ],
            options_disabled: false,
        }];
        let command = vec!["echo".to_string(), "$SECRET".to_string()];
        let env = HashMap::from([("SECRET".to_string(), "trusted".to_string())]);

        let expanded = expand_command_for_execution(&command, &specs, &env);

        assert_eq!(expanded, command);
    }

    #[test]
    fn test_expand_command_for_execution_leaves_missing_literal_var_opaque() {
        let specs = vec![ToolSpec {
            parts: vec![
                ToolPart::Literal("echo".to_string()),
                ToolPart::Literal("$MISSING".to_string()),
            ],
            options_disabled: false,
        }];
        let command = vec!["echo".to_string(), "$MISSING".to_string()];
        let env = HashMap::from([("OTHER".to_string(), "value".to_string())]);

        let expanded = expand_command_for_execution(&command, &specs, &env);

        assert_eq!(expanded, command);
    }
}
