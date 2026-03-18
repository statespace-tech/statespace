use clap::{Arg, ArgAction, Command, CommandFactory};
use std::path::Path;

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
    let name = arg
        .get_value_names()
        .and_then(|n| n.first())
        .map(ToString::to_string)
        .unwrap_or_else(|| arg.get_id().to_string().to_uppercase());
    let desc = arg
        .get_help()
        .map(|h| format!("\n: {h}"))
        .unwrap_or_default();
    format!("`{name}`{desc}")
}

/// Derive the markdown anchor for a heading (lowercase, spaces → hyphens).
fn anchor(full_name: &str) -> String {
    full_name.to_lowercase().replace(' ', "-")
}

fn usage_line(cmd: &Command, full_name: &str) -> String {
    let positionals: Vec<_> = cmd
        .get_positionals()
        .filter(|a| !a.is_hide_set())
        .collect();
    let options: Vec<_> = cmd
        .get_opts()
        .filter(|a| !a.is_hide_set())
        .filter(|a| !matches!(a.get_action(), ArgAction::Help | ArgAction::Version))
        .collect();
    let subcommands: Vec<_> = cmd
        .get_subcommands()
        .filter(|s| !s.is_hide_set())
        .collect();

    let mut parts = vec![full_name.to_string()];
    if !options.is_empty() {
        parts.push("[OPTIONS]".to_string());
    }
    if !subcommands.is_empty() {
        parts.push("<COMMAND>".to_string());
    }
    for arg in &positionals {
        let name = arg
            .get_value_names()
            .and_then(|n| n.first())
            .map(ToString::to_string)
            .unwrap_or_else(|| arg.get_id().to_string().to_uppercase());
        if arg.is_required_set() {
            parts.push(format!("<{name}>"));
        } else {
            parts.push(format!("[{name}]"));
        }
    }
    parts.join(" ")
}

/// Write the body sections (**Usage**, **Commands**, **Arguments**, #### Options)
/// for a command. H4 keeps these out of the sidebar.
fn write_command_body(out: &mut String, cmd: &Command, full_name: &str, options_heading: &str) {
    // **Usage**
    let usage = usage_line(cmd, full_name);
    out.push_str("**Usage**\n\n");
    out.push_str(&format!("```\n{usage}\n```\n\n"));

    // **Commands** — subcommand names link to their own section
    let subcommands: Vec<_> = cmd.get_subcommands().filter(|s| !s.is_hide_set()).collect();
    if !subcommands.is_empty() {
        out.push_str("**Commands**\n\n");
        for sub in &subcommands {
            let sub_full = format!("{full_name} {}", sub.get_name());
            let desc = sub
                .get_about()
                .map(|a| format!("\n: {a}"))
                .unwrap_or_default();
            out.push_str(&format!(
                "[`{sub_full}`](#{anchor}){desc}\n\n",
                anchor = anchor(&sub_full)
            ));
        }
    }

    // **Arguments**
    let positionals: Vec<_> = cmd
        .get_positionals()
        .filter(|a| !a.is_hide_set())
        .collect();
    if !positionals.is_empty() {
        out.push_str("**Arguments**\n\n");
        for arg in &positionals {
            out.push_str(&format!("{}\n\n", positional_item(arg)));
        }
    }

    // #### Options
    let options: Vec<_> = cmd
        .get_opts()
        .filter(|a| !a.is_hide_set())
        .filter(|a| !matches!(a.get_action(), ArgAction::Help | ArgAction::Version))
        .collect();
    if !options.is_empty() {
        out.push_str(&format!("**{options_heading}**\n\n"));
        for arg in &options {
            out.push_str(&format!("{}\n\n", option_item(arg)));
        }
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let workspace_root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .ok_or("expected tools/gen-cli-docs to have a parent")?
        .parent()
        .ok_or("expected tools to have a parent")?;

    let output_path = workspace_root.join("docs/pages/reference/cli.md");
    let cmd = statespace::Cli::command();

    let mut out = String::new();

    out.push_str("---\nicon: lucide/terminal\n---\n\n");
    out.push_str("# CLI reference\n\n");

    // Root: ## statespace
    out.push_str("## `statespace`\n\n");
    if let Some(about) = cmd.get_about() {
        out.push_str(&format!("{about}\n\n"));
    }
    write_command_body(&mut out, &cmd, "statespace", "Global options");

    // Top-level commands: ##
    // Their subcommands: ### (creates sidebar nesting)
    // Usage/Arguments/Options within each: #### (hidden from sidebar)
    for top in cmd.get_subcommands().filter(|s| !s.is_hide_set()) {
        let top_name = format!("statespace {}", top.get_name());
        let subcommands: Vec<_> = top.get_subcommands().filter(|s| !s.is_hide_set()).collect();

        out.push_str(&format!("## `{top_name}`\n\n"));
        if let Some(about) = top.get_about() {
            out.push_str(&format!("{about}\n\n"));
        }
        write_command_body(&mut out, top, &top_name, "Options");

        for sub in &subcommands {
            let sub_name = format!("{top_name} {}", sub.get_name());
            out.push_str(&format!("### `{sub_name}`\n\n"));
            if let Some(about) = sub.get_about() {
                out.push_str(&format!("{about}\n\n"));
            }
            write_command_body(&mut out, sub, &sub_name, "Options");
        }
    }

    std::fs::write(&output_path, &out)?;
    println!("Generated {}", output_path.display());

    Ok(())
}
