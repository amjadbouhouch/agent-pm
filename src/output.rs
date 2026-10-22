use serde::Serialize;

use crate::error::{AppError, Result};

/// Print a value as pretty JSON on stdout (the `--json` contract).
pub fn json<T: Serialize + ?Sized>(value: &T) -> Result<()> {
    println!("{}", serde_json::to_string_pretty(value)?);
    Ok(())
}

/// A write's result: the written object under `--json`, else one line (`created APM-3`).
pub fn written<T: Serialize + ?Sized>(json: bool, value: &T, line: &str) -> Result<()> {
    if json {
        return self::json(value);
    }
    println!("{line}");
    Ok(())
}

/// Errors go to stderr: `error: <msg>`, or `{"error": "<msg>"}` under `--json`.
pub fn print_error(e: &AppError, json: bool) {
    if json {
        eprintln!("{}", serde_json::json!({ "error": e.0 }));
    } else {
        eprintln!("error: {e}");
    }
}

/// A problem that does not fail the command (it already did its work): `warning: <msg>` on
/// stderr, or `{"warning": "<msg>"}` under `--json`.
pub fn print_warning(msg: &str, json: bool) {
    if json {
        eprintln!("{}", serde_json::json!({ "warning": msg }));
    } else {
        eprintln!("warning: {msg}");
    }
}

/// Print rows as left-aligned columns separated by two spaces, under a header line.
/// Prints nothing for no rows.
pub fn table(headers: &[&str], rows: &[Vec<String>]) {
    if rows.is_empty() {
        return;
    }
    let mut all = vec![headers.iter().map(|h| h.to_string()).collect()];
    all.extend_from_slice(rows);
    for line in aligned(&all) {
        println!("{line}");
    }
}

/// Rows as lines of left-aligned columns separated by two spaces (the last column is not
/// padded). For tables without a header, e.g. the subtasks in `show`.
pub fn aligned(rows: &[Vec<String>]) -> Vec<String> {
    let mut widths: Vec<usize> = Vec::new();
    for row in rows {
        for (i, cell) in row.iter().enumerate() {
            let w = cell.chars().count();
            match widths.get_mut(i) {
                Some(max) => *max = (*max).max(w),
                None => widths.push(w),
            }
        }
    }
    rows.iter()
        .map(|row| {
            let last = row.len().saturating_sub(1);
            let mut out = String::new();
            for (i, cell) in row.iter().enumerate() {
                if i == last {
                    out.push_str(cell);
                } else {
                    out.push_str(&format!("{:<w$}  ", cell, w = widths[i]));
                }
            }
            out.trim_end().to_string()
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn aligned_pads_all_but_the_last_column() {
        let rows = vec![
            vec!["APM-5".to_string(), "doing".to_string(), "a".to_string()],
            vec!["APM-10".to_string(), "todo".to_string(), "b c".to_string()],
        ];
        assert_eq!(aligned(&rows), ["APM-5   doing  a", "APM-10  todo   b c"]);
        assert!(aligned(&[]).is_empty());
    }
}
