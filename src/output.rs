use serde::Serialize;

use crate::error::{AppError, Result};

/// Print a value as pretty JSON on stdout (the `--json` contract).
pub fn json<T: Serialize + ?Sized>(value: &T) -> Result<()> {
    println!("{}", serde_json::to_string_pretty(value)?);
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

/// Print rows as left-aligned columns separated by two spaces. Prints nothing for no rows.
pub fn table(headers: &[&str], rows: &[Vec<String>]) {
    if rows.is_empty() {
        return;
    }
    let mut widths: Vec<usize> = headers.iter().map(|h| h.chars().count()).collect();
    for row in rows {
        for (i, cell) in row.iter().enumerate() {
            widths[i] = widths[i].max(cell.chars().count());
        }
    }
    let line = |cells: Vec<&str>| {
        let last = cells.len() - 1;
        let mut out = String::new();
        for (i, cell) in cells.iter().enumerate() {
            if i == last {
                out.push_str(cell);
            } else {
                out.push_str(&format!("{:<w$}  ", cell, w = widths[i]));
            }
        }
        println!("{}", out.trim_end());
    };
    line(headers.to_vec());
    for row in rows {
        line(row.iter().map(String::as_str).collect());
    }
}
