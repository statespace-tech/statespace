use std::fmt::Write as FmtWrite;
use std::path::Path;

use clap::{Arg, ArgAction, Command, CommandFactory};

fn option_item(arg: &Arg) -> String {
    let mut parts = Vec::new();
    if let Some(l) = arg.get_long() {
        parts.push(format!("--{l}"));
    }
    if let Some(s) = arg.get_short() {
        parts.push(format!("-{s}"));
    }
    let flags = parts.join(", ");
    let desc = arg
        .get_help()
        .map(|h| format!("\n: {h}"))
        .unwrap_or_default();
    format!("`{flags}`{desc}")
}

fn positional_item(arg: &Arg) -> String {
    let name = arg.get_value_names().and_then(|n| n.first()).map_or_else(
        || arg.get_id().to_string().to_uppercase(),
        ToString::to_string,
    );
    let desc = arg
        .get_help()
        .map(|h| format!("\n: {h}"))
        .unwrap_or_default();
    format!("`{name}`{desc}")
}

fn anchor(full_name: &str) -> String {
    full_name.to_lowercase().replace(' ', "-")
}

fn usage_line(cmd: &Command, full_name: &str) -> String {
    let positionals: Vec<_> = cmd.get_positionals().filter(|a| !a.is_hide_set()).collect();
    let options: Vec<_> = cmd
        .get_opts()
        .filter(|a| !a.is_hide_set())
        .filter(|a| !matches!(a.get_action(), ArgAction::Help | ArgAction::Version))
        .collect();
    let subcommands: Vec<_> = cmd.get_subcommands().filter(|s| !s.is_hide_set()).collect();

    let mut parts = vec![full_name.to_string()];
    if !options.is_empty() {
        parts.push("[OPTIONS]".to_string());
    }
    if !subcommands.is_empty() {
        parts.push("<COMMAND>".to_string());
    }
    for arg in &positionals {
        let name = arg.get_value_names().and_then(|n| n.first()).map_or_else(
            || arg.get_id().to_string().to_uppercase(),
            ToString::to_string,
        );
        if arg.is_required_set() {
            parts.push(format!("<{name}>"));
        } else {
            parts.push(format!("[{name}]"));
        }
    }
    parts.join(" ")
}

fn write_command_body(out: &mut String, cmd: &Command, full_name: &str, options_heading: &str) {
    let usage = usage_line(cmd, full_name);
    out.push_str("**Usage**\n\n");
    let _ = write!(out, "```console\n{usage}\n```\n\n");

    let subcommands: Vec<_> = cmd.get_subcommands().filter(|s| !s.is_hide_set()).collect();
    if !subcommands.is_empty() {
        out.push_str("**Commands**\n\n");
        for sub in &subcommands {
            let sub_full = format!("{full_name} {}", sub.get_name());
            let desc = sub
                .get_about()
                .map(|a| format!("\n: {a}"))
                .unwrap_or_default();
            let _ = write!(
                out,
                "[`{sub_full}`](#{anchor}){desc}\n\n",
                anchor = anchor(&sub_full)
            );
        }
    }

    let positionals: Vec<_> = cmd.get_positionals().filter(|a| !a.is_hide_set()).collect();
    if !positionals.is_empty() {
        out.push_str("**Arguments**\n\n");
        for arg in &positionals {
            let _ = write!(out, "{}\n\n", positional_item(arg));
        }
    }

    let options: Vec<_> = cmd
        .get_opts()
        .filter(|a| !a.is_hide_set())
        .filter(|a| !matches!(a.get_action(), ArgAction::Help | ArgAction::Version))
        .collect();
    if !options.is_empty() {
        let _ = write!(out, "**{options_heading}**\n\n");
        for arg in &options {
            let _ = write!(out, "{}\n\n", option_item(arg));
        }
    }
}

fn render_subcommands(out: &mut String, cmd: &Command, full_name: &str, depth: usize) {
    let heading = "#".repeat(depth);
    for sub in cmd.get_subcommands().filter(|s| !s.is_hide_set()) {
        let sub_name = format!("{full_name} {}", sub.get_name());
        let _ = write!(out, "{heading} `{sub_name}`\n\n");
        if let Some(about) = sub.get_about() {
            let _ = write!(out, "{about}\n\n");
        }
        write_command_body(out, sub, &sub_name, "Options");
        render_subcommands(out, sub, &sub_name, depth + 1);
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let manifest_dir = Path::new(env!("CARGO_MANIFEST_DIR"));
    let workspace_root = manifest_dir
        .parent()
        .ok_or_else(|| {
            format!(
                "expected binaries/statespace-cli to have a parent, got: {}",
                manifest_dir.display()
            )
        })?
        .parent()
        .ok_or_else(|| {
            format!(
                "expected binaries/ to have a parent, got: {}",
                manifest_dir.display()
            )
        })?;

    if !workspace_root.join("Cargo.toml").exists() {
        return Err(format!(
            "workspace root does not contain Cargo.toml: {}",
            workspace_root.display()
        )
        .into());
    }

    let output_path = workspace_root.join("docs/pages/reference/cli.md");
    let cmd = statespace::Cli::command();

    let mut out = String::new();

    out.push_str("---\nicon: lucide/terminal\n---\n\n");
    out.push_str("# CLI reference\n\n");

    out.push_str("## `statespace`\n\n");
    if let Some(about) = cmd.get_about() {
        let _ = write!(out, "{about}\n\n");
    }
    write_command_body(&mut out, &cmd, "statespace", "Global options");

    render_subcommands(&mut out, &cmd, "statespace", 2);

    std::fs::write(&output_path, &out)?;
    println!("Generated {}", output_path.display());

    Ok(())
}
