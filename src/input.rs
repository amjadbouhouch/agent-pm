//! Long text from the command line, stdin or a file: task bodies and comments.

use std::io::{IsTerminal, Read};
use std::path::Path;

use crate::error::{Result, err};

/// How one kind of long text is given, for messages.
pub struct Flags {
    /// The stdin form, e.g. `-b -`.
    pub stdin: &'static str,
    /// The file flag, e.g. `--body-file`.
    pub file: &'static str,
    /// The error for text read from stdin or a file that is empty.
    pub empty: &'static str,
}

pub const BODY: Flags = Flags {
    stdin: "-b -",
    file: "--body-file",
    empty: "body is empty; to clear a body use edit --unset body",
};

pub const COMMENT: Flags = Flags {
    stdin: "comment KEY-N -",
    file: "--file",
    empty: "comment text cannot be empty",
};

/// Text given inline, as `-` (read stdin) or from a file; None when neither is given.
/// Inline text is kept as is. Read text must be UTF-8 (a leading BOM is dropped, so files
/// saved by Windows editors work) and loses its trailing newlines (a heredoc or an editor
/// adds one); empty read text is an error. The caller's clap args make inline and file
/// mutually exclusive.
pub fn read_text(
    inline: Option<String>,
    file: Option<&Path>,
    flags: &Flags,
) -> Result<Option<String>> {
    let text = match (inline, file) {
        (Some(t), _) if t == "-" => {
            let mut stdin = std::io::stdin();
            // Reading a terminal would wait for input that an agent never sends.
            if stdin.is_terminal() {
                return Err(err(format!(
                    "{} reads stdin, but nothing is piped (use {} or pipe the text)",
                    flags.stdin, flags.file
                )));
            }
            let mut bytes = Vec::new();
            stdin
                .read_to_end(&mut bytes)
                .map_err(|e| err(format!("cannot read stdin: {e}")))?;
            decode(bytes).ok_or_else(|| err("stdin is not UTF-8 text"))?
        }
        (Some(t), _) => return Ok(Some(t)),
        (None, Some(path)) => {
            let bytes = std::fs::read(path)
                .map_err(|e| err(format!("cannot read {}: {e}", path.display())))?;
            decode(bytes)
                .ok_or_else(|| err(format!("file is not UTF-8 text: {}", path.display())))?
        }
        (None, None) => return Ok(None),
    };
    let text = text.trim_end_matches(['\n', '\r']);
    if text.trim().is_empty() {
        return Err(err(flags.empty));
    }
    Ok(Some(text.to_string()))
}

/// UTF-8 bytes as text without a leading BOM; None if they are not UTF-8 (e.g. the UTF-16
/// that Windows PowerShell 5.1 writes with `>`).
fn decode(bytes: Vec<u8>) -> Option<String> {
    let text = String::from_utf8(bytes).ok()?;
    Some(match text.strip_prefix('\u{feff}') {
        Some(rest) => rest.to_string(),
        None => text,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decode_drops_a_bom_and_rejects_utf16() {
        assert_eq!(decode(b"\xef\xbb\xbfhi".to_vec()).as_deref(), Some("hi"));
        assert_eq!(decode("é".as_bytes().to_vec()).as_deref(), Some("é"));
        // "hi" as UTF-16LE with its BOM.
        assert_eq!(decode(vec![0xff, 0xfe, b'h', 0, b'i', 0]), None);
    }
}
