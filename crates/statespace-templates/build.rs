use std::env;
use std::fmt::Write;
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Clone, Copy)]
struct TemplateSet {
    const_name: &'static str,
    source_dir: &'static str,
}

const TEMPLATE_SETS: &[TemplateSet] = &[
    TemplateSet {
        const_name: "RAG_TEMPLATE_ASSETS",
        source_dir: "rag",
    },
    TemplateSet {
        const_name: "KNOWLEDGE_BASE_TEMPLATE_ASSETS",
        source_dir: "knowledge_base",
    },
    TemplateSet {
        const_name: "AGENT_SKILL_TEMPLATE_ASSETS",
        source_dir: "agent_skill",
    },
    TemplateSet {
        const_name: "TEXT_TO_SQL_TEMPLATE_ASSETS",
        source_dir: "text_to_sql",
    },
    TemplateSet {
        const_name: "WORKFLOW_TEMPLATE_ASSETS",
        source_dir: "workflow",
    },
];

fn collect_files(root: &Path) -> Result<Vec<PathBuf>, String> {
    let mut files = Vec::new();
    let mut stack = vec![root.to_path_buf()];

    while let Some(current) = stack.pop() {
        let entries = fs::read_dir(&current)
            .map_err(|error| format!("failed to read directory {}: {error}", current.display()))?;

        for entry in entries {
            let entry = entry.map_err(|error| {
                format!(
                    "failed to read directory entry in {}: {error}",
                    current.display()
                )
            })?;
            let path = entry.path();
            let file_type = entry.file_type().map_err(|error| {
                format!("failed to read file type for {}: {error}", path.display())
            })?;

            if file_type.is_dir() {
                stack.push(path);
                continue;
            }
            if file_type.is_file() {
                files.push(path);
            }
        }
    }

    files.sort();
    Ok(files)
}

fn emit_template_set(
    source_root: &Path,
    set: TemplateSet,
    output: &mut String,
) -> Result<(), String> {
    let set_root = source_root.join(set.source_dir);
    if !set_root.is_dir() {
        return Err(format!(
            "template source directory missing: {}",
            set_root.display()
        ));
    }

    let files = collect_files(&set_root)?;
    if files.is_empty() {
        return Err(format!(
            "template source directory is empty: {}",
            set_root.display()
        ));
    }

    writeln!(
        output,
        "pub const {}: &[(&str, &[u8])] = &[",
        set.const_name
    )
    .map_err(|error| format!("failed to write generated output: {error}"))?;

    for file_path in files {
        println!("cargo:rerun-if-changed={}", file_path.display());
        let rel = file_path.strip_prefix(source_root).map_err(|error| {
            format!(
                "failed to strip examples root {} from {}: {error}",
                source_root.display(),
                file_path.display()
            )
        })?;
        let rel_str = rel.to_string_lossy().replace('\\', "/");
        writeln!(
            output,
            "    ({rel_str:?}, include_bytes!(concat!(env!(\"CARGO_MANIFEST_DIR\"), \"/../../examples/{rel_str}\"))),"
        )
        .map_err(|error| format!("failed to write generated output: {error}"))?;
    }

    writeln!(output, "];\n")
        .map_err(|error| format!("failed to write generated output: {error}"))?;
    Ok(())
}

fn generate_template_assets_module() -> Result<String, String> {
    let manifest_dir = PathBuf::from(
        env::var("CARGO_MANIFEST_DIR")
            .map_err(|error| format!("CARGO_MANIFEST_DIR should be set by Cargo: {error}"))?,
    );
    let examples_root = manifest_dir.join("../../examples");

    if !examples_root.is_dir() {
        return Err(format!(
            "examples directory missing: {}",
            examples_root.display()
        ));
    }

    let mut output = String::new();
    for set in TEMPLATE_SETS {
        emit_template_set(&examples_root, *set, &mut output)?;
    }

    Ok(output)
}

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=../../examples");

    let generated = match generate_template_assets_module() {
        Ok(generated) => generated,
        Err(error) => {
            eprintln!("failed to generate template assets module: {error}");
            std::process::exit(1);
        }
    };

    let out_dir = match env::var("OUT_DIR") {
        Ok(path) => PathBuf::from(path),
        Err(error) => {
            eprintln!("OUT_DIR should be set by Cargo: {error}");
            std::process::exit(1);
        }
    };

    let generated_path = out_dir.join("generated_template_assets.rs");

    if let Err(error) = fs::write(&generated_path, generated) {
        eprintln!(
            "failed to write generated template assets module {}: {error}",
            generated_path.display()
        );
        std::process::exit(1);
    }
}
