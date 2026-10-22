use std::fmt;

/// The one error type. Every failure ends as a message on stderr and exit code 1.
#[derive(Debug)]
pub struct AppError(pub String);

pub type Result<T> = std::result::Result<T, AppError>;

/// Build an error from a message: `return Err(err("no such project: X"))`.
pub fn err(msg: impl Into<String>) -> AppError {
    AppError(msg.into())
}

impl fmt::Display for AppError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl From<rusqlite::Error> for AppError {
    fn from(e: rusqlite::Error) -> Self {
        AppError(format!("database: {e}"))
    }
}

impl From<std::io::Error> for AppError {
    fn from(e: std::io::Error) -> Self {
        AppError(format!("io: {e}"))
    }
}

impl From<serde_json::Error> for AppError {
    fn from(e: serde_json::Error) -> Self {
        AppError(format!("json: {e}"))
    }
}
