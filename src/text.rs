use std::path::Path;

use crate::error::Result;

/// Read text-like external files without failing on non-UTF-8 bytes.
pub(crate) fn read_lossy(path: &Path) -> Result<String> {
    let bytes = std::fs::read(path)?;
    Ok(String::from_utf8_lossy(&bytes).into_owned())
}
