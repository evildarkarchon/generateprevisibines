use std::path::Path;

use crate::error::Result;

/// Read text-like external files without failing on non-UTF-8 bytes.
pub(crate) fn read_lossy(path: &Path) -> Result<String> {
    let bytes = std::fs::read(path)?;
    Ok(String::from_utf8_lossy(&bytes).into_owned())
}

/// Whether `haystack` contains `needle`, ignoring ASCII case (the batch's `findstr /I /C:`).
///
/// Both sides are folded the same way rather than assuming either is already in one case, so a
/// marker respelled in mixed case keeps matching. The batch's markers are ASCII, so no Unicode
/// case mapping is in play; non-ASCII characters (a plugin name, say) must match exactly.
pub(crate) fn contains_ignore_ascii_case(haystack: &str, needle: &str) -> bool {
    haystack
        .to_ascii_lowercase()
        .contains(&needle.to_ascii_lowercase())
}

#[cfg(test)]
mod tests {
    use super::contains_ignore_ascii_case;

    #[test]
    fn matching_ignores_ascii_case_on_both_sides() {
        assert!(contains_ignore_ascii_case(
            "[00:42] COMPLETED: no errors.",
            "Completed: No Errors."
        ));
        assert!(contains_ignore_ascii_case("completed: ", "COMPLETED: "));
        assert!(!contains_ignore_ascii_case("Completed", "Completed: "));
    }
}
