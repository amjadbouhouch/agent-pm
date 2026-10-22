use crate::error::Result;
use crate::output;

/// The guide is static text compiled into the binary.
const GUIDE: &str = include_str!("guide.txt");

/// `apm guide`: a compact usage guide for agents. Never opens the database.
pub fn run(json: bool) -> Result<()> {
    if json {
        output::json(&serde_json::json!({ "guide": GUIDE }))
    } else {
        print!("{GUIDE}");
        Ok(())
    }
}
