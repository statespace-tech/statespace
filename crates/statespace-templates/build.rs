// Build scripts abort by panicking; unwrap/expect/panic are appropriate here.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::fmt::Write as FmtWrite;
use std::fs;
use std::io::Write as IoWrite;
use std::path::Path;

fn escape_rust_str(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 64);
    for c in s.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '\0' => out.push_str("\\0"),
            c if (c as u32) < 32 => write!(out, "\\u{{{:04X}}}", c as u32).unwrap(),
            c => out.push(c),
        }
    }
    out
}

fn main() {
    let manifest_dir = std::env::var("CARGO_MANIFEST_DIR").unwrap();
    let app_dir = Path::new(&manifest_dir).join("src").join("init");
    let out_dir = std::env::var("OUT_DIR").unwrap();
    let dest = Path::new(&out_dir).join("templates_generated.rs");
    let mut f = fs::File::create(&dest).unwrap();

    let mut entries: Vec<_> = fs::read_dir(&app_dir)
        .expect("crates/statespace-templates/src/init/ directory not found")
        .filter_map(Result::ok)
        .filter(|e| e.file_type().map(|t| t.is_dir()).unwrap_or(false))
        .collect();
    entries.sort_by_key(std::fs::DirEntry::file_name);

    writeln!(f, "/// Returns the template for `name`, or `None` if unrecognized.").unwrap();
    writeln!(f, "/// Matching is case-insensitive; hyphens and underscores are equivalent.").unwrap();
    writeln!(f, "pub fn get(name: &str) -> Option<Template> {{").unwrap();
    writeln!(f, "    let key = name.to_lowercase().replace('-', \"_\");").unwrap();
    writeln!(f, "    match key.as_str() {{").unwrap();

    let mut slugs: Vec<String> = Vec::new();

    for entry in &entries {
        let slug = entry.file_name().to_string_lossy().to_string();
        let readme_path = entry.path().join("README.md");
        let dockerfile_path = entry.path().join("Dockerfile");

        let readme = fs::read_to_string(&readme_path)
            .unwrap_or_else(|_| panic!("Missing README.md for template `{slug}`"));
        let dockerfile = if dockerfile_path.exists() {
            Some(fs::read_to_string(&dockerfile_path).unwrap())
        } else {
            None
        };

        writeln!(f, "        {slug:?} => Some(Template {{").unwrap();
        writeln!(f, "            readme: \"{}\",", escape_rust_str(&readme)).unwrap();
        match dockerfile {
            Some(df) => writeln!(
                f,
                "            dockerfile: Some(\"{}\"),",
                escape_rust_str(&df)
            )
            .unwrap(),
            None => writeln!(f, "            dockerfile: None,").unwrap(),
        }
        writeln!(f, "        }}),").unwrap();

        slugs.push(slug);
    }

    writeln!(f, "        _ => None,").unwrap();
    writeln!(f, "    }}").unwrap();
    writeln!(f, "}}").unwrap();
    writeln!(f).unwrap();

    writeln!(f, "/// Canonical template names accepted by `get`.").unwrap();
    writeln!(f, "pub const NAMES: &[&str] = &[").unwrap();
    for slug in &slugs {
        let display = slug.replace('_', "-");
        writeln!(f, "    {display:?},").unwrap();
    }
    writeln!(f, "];").unwrap();

    // Rerun if the directory listing or any template file changes.
    println!("cargo:rerun-if-changed={}", app_dir.display());
    for entry in &entries {
        println!(
            "cargo:rerun-if-changed={}",
            entry.path().join("README.md").display()
        );
        let df = entry.path().join("Dockerfile");
        if df.exists() {
            println!("cargo:rerun-if-changed={}", df.display());
        }
    }
}
