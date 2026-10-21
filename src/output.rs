//! What commands print: text by default, or one JSON document with `--json`.

use std::sync::OnceLock;

use serde::Serialize;
use serde_json::Value;

static JSON: OnceLock<bool> = OnceLock::new();

pub fn set_json(json: bool) {
    let _ = JSON.set(json);
}

pub fn json() -> bool {
    JSON.get().copied().unwrap_or(false)
}

/// Print `value` as JSON with `--json`, else the text that `text` writes.
pub fn emit(value: &impl Serialize, text: impl FnOnce() -> String) -> anyhow::Result<()> {
    if json() {
        println!("{}", serde_json::to_string_pretty(value)?);
    } else {
        println!("{}", text().trim_end());
    }
    Ok(())
}

/// An error that the command printed already, as part of its output. It
/// only sets the exit status.
#[derive(Debug)]
pub struct Reported;

impl std::fmt::Display for Reported {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("the command failed")
    }
}

impl std::error::Error for Reported {}

/// A note for people, on standard error. `--json` leaves it out.
pub fn note(message: &str) {
    if !json() {
        eprintln!("{message}");
    }
}

/// Labels and values, with the values aligned.
pub fn fields(rows: &[(&str, String)]) -> String {
    let width = rows.iter().map(|(label, _)| label.len()).max().unwrap_or(0) + 1;
    rows.iter()
        .map(|(label, value)| format!("{:width$} {value}\n", format!("{label}:")))
        .collect()
}

/// A table with a header row and aligned columns.
pub fn table(headers: &[&str], rows: Vec<Vec<String>>) -> String {
    let mut widths: Vec<usize> = headers.iter().map(|header| header.len()).collect();
    for row in &rows {
        for (width, cell) in widths.iter_mut().zip(row) {
            *width = (*width).max(cell.chars().count());
        }
    }
    let line = |cells: Vec<String>| {
        let last = cells.len().saturating_sub(1);
        let mut line = String::new();
        for (index, (cell, width)) in cells.iter().zip(&widths).enumerate() {
            if index == last {
                line.push_str(cell);
            } else {
                line.push_str(&format!("{cell:width$}  "));
            }
        }
        line.trim_end().to_owned() + "\n"
    };
    let mut text = line(headers.iter().map(ToString::to_string).collect());
    for row in rows {
        text.push_str(&line(row));
    }
    text
}

/// A string field, or `-` when it is null or empty.
pub fn text(value: &Value) -> String {
    match value {
        Value::Null => "-".into(),
        Value::String(text) if text.is_empty() => "-".into(),
        Value::String(text) => text.clone(),
        other => other.to_string(),
    }
}

/// A list of strings, joined with commas.
pub fn list(value: &Value) -> String {
    match value.as_array() {
        Some(items) if !items.is_empty() => items.iter().map(text).collect::<Vec<_>>().join(", "),
        _ => "-".into(),
    }
}

/// A share of subjects as a percentage, such as `5%` or `12.5%`.
pub fn share(share: f64) -> String {
    let percent = format!("{:.2}", share * 100.0);
    format!("{}%", percent.trim_end_matches('0').trim_end_matches('.'))
}

/// A timestamp in UTC to the minute, such as `2026-10-15 17:40 UTC`.
pub fn time(value: &Value) -> String {
    value
        .as_str()
        .and_then(|time| chrono::DateTime::parse_from_rfc3339(time).ok())
        .map_or_else(
            || text(value),
            |time| {
                time.with_timezone(&chrono::Utc)
                    .format("%Y-%m-%d %H:%M UTC")
                    .to_string()
            },
        )
}

/// A size in bytes, in binary units such as `1.5 GiB`.
pub fn bytes(bytes: i64) -> String {
    let mut size = bytes as f64;
    for unit in ["B", "KiB", "MiB", "GiB"] {
        if size < 1024.0 {
            return if unit == "B" {
                format!("{bytes} B")
            } else {
                format!("{size:.1} {unit}")
            };
        }
        size /= 1024.0;
    }
    format!("{size:.1} TiB")
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn formats_values() {
        assert_eq!(share(0.05), "5%");
        assert_eq!(share(0.125), "12.5%");
        assert_eq!(share(1.0 / 3.0), "33.33%");
        assert_eq!(
            time(&json!("2026-10-15T17:40:12.5+00:00")),
            "2026-10-15 17:40 UTC"
        );
        assert_eq!(time(&Value::Null), "-");
        assert_eq!(bytes(999), "999 B");
        assert_eq!(bytes(10 * 1024 * 1024 * 1024), "10.0 GiB");
        assert_eq!(list(&json!(["a", "b"])), "a, b");
    }

    #[test]
    fn aligns_fields_and_tables() {
        assert_eq!(
            fields(&[("Name", "web".into()), ("Retention", "30 days".into())]),
            "Name:      web\nRetention: 30 days\n"
        );
        assert_eq!(
            table(
                &["NAME", "VERSION"],
                vec![vec!["checkout".into(), "2".into()]]
            ),
            "NAME      VERSION\ncheckout  2\n"
        );
    }
}
