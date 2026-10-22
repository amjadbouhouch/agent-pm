use std::path::Path;

use super::Ctx;
use crate::error::Result;
use crate::{db, output};

/// `apm db`: where the database is and which schema it has.
pub fn run(ctx: &Ctx, path: &Path) -> Result<()> {
    let version = db::schema_version(&ctx.conn)?;
    if ctx.json {
        output::json(&serde_json::json!({
            "path": path.display().to_string(),
            "schema_version": version,
        }))
    } else {
        println!("{}\nschema v{version}", path.display());
        Ok(())
    }
}
