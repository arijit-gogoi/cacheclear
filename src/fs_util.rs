//! Filesystem helpers shared by direct-deletion managers (cargo, pipx).
//!
//! Two invariants from `spec/SPEC.md`:
//! 1. Cache root is canonicalized once; symlinks BELOW the root are never followed.
//! 2. On Windows, the canonicalized root is `\\?\`-prefixed so deep paths
//!    (cargo's `registry/src/...`) don't trip MAX_PATH. Display strings strip
//!    the prefix via `dunce`; **internal ops keep it**.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

/// Canonicalize + (on Windows) `\\?\`-prefix. Use this once at the top of any
/// direct-deletion routine; every descendant inherits the prefix automatically.
#[cfg(windows)]
pub fn prepare_root(p: &Path) -> io::Result<PathBuf> {
    let canonical = fs::canonicalize(p)?;
    let s = canonical.as_os_str().to_string_lossy();
    if s.starts_with(r"\\?\") {
        Ok(canonical)
    } else {
        Ok(PathBuf::from(format!(r"\\?\{}", s)))
    }
}

#[cfg(not(windows))]
pub fn prepare_root(p: &Path) -> io::Result<PathBuf> {
    fs::canonicalize(p)
}

/// Sum the on-disk size of a directory tree, in bytes.
/// Stub: real impl uses `walkdir` and respects the same symlink rules as `safe_rm_contents`.
pub fn dir_size(_root: &Path) -> io::Result<u64> {
    Ok(0)
}

/// Remove the *contents* of `root` (not `root` itself), refusing to descend into
/// symlinks and refusing to operate if the canonical root is at/above a system path.
/// Stub: real impl lands with the `cargo` and `pipx` modules.
pub fn safe_rm_contents(_root: &Path) -> io::Result<u64> {
    Ok(0)
}
