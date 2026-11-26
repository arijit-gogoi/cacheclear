//! Filesystem helpers shared by direct-deletion managers (cargo, pipx).
//!
//! Two invariants from `spec/SPEC.md`:
//! 1. Cache root is canonicalized once; symlinks BELOW the root are never followed.
//! 2. On Windows, the canonicalized root is `\\?\`-prefixed so deep paths
//!    (cargo's `registry/src/...`) don't trip MAX_PATH. Display strings strip
//!    the prefix via `dunce`; **internal ops keep it**.

use std::fs::{self, File, TryLockError};
use std::io;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use walkdir::WalkDir;

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

/// The user's home directory. Every manager resolves home-relative cache
/// paths through this, never through `dirs::home_dir()` directly.
///
/// On Windows, `dirs::home_dir()` asks the shell for the profile folder and
/// ignores `USERPROFILE`. `std::env::home_dir` (fixed in Rust 1.86) honours
/// `USERPROFILE` on Windows and `HOME` on Unix, so an overridden or sandboxed
/// home is respected.
pub fn home_dir() -> Option<PathBuf> {
    std::env::home_dir().filter(|p| p.is_absolute())
}

/// Refuse if the resolved path is at or above a system root.
///
/// Per `spec/SPEC.md` §"Symlink Policy": "after canonicalizing the cache root,
/// refuse to operate if the resolved path is at or above any of: `/`, `$HOME`,
/// `/etc`, `/usr`, `/var`, `C:\`, `C:\Users`. A manager's "cache" resolving to
/// a system root indicates a broken config, not a legitimate clear target."
///
/// The check uses `path == forbidden || forbidden.starts_with(path)` so that
/// either an exact match OR a path that *contains* the forbidden root counts.
/// (We test the canonicalized path; on Windows we strip the `\\?\` prefix
/// first via `dunce::simplified` so comparisons against `C:\` work.)
pub fn is_safe_root(canonical: &Path) -> bool {
    #[cfg(windows)]
    let display = dunce::simplified(canonical).to_path_buf();
    #[cfg(not(windows))]
    let display: PathBuf = canonical.to_path_buf();

    let mut forbidden: Vec<PathBuf> = vec![];
    #[cfg(not(windows))]
    {
        forbidden.extend(["/", "/etc", "/usr", "/var"].iter().map(PathBuf::from));
    }
    #[cfg(windows)]
    {
        forbidden.extend([r"C:\", r"C:\Users"].iter().map(PathBuf::from));
    }
    // Both the env-based home and the OS profile dir, in case they differ.
    forbidden.extend(home_dir());
    forbidden.extend(dirs::home_dir());

    for f in &forbidden {
        if display == *f {
            return false;
        }
        // If the *forbidden* path is a strict prefix of `display`, that means
        // `display` is *under* it (deeper) — fine. We're checking the inverse:
        // is `display` at-or-above forbidden? Equivalent to: is `display` a
        // prefix of (or equal to) `forbidden`?
        if f.starts_with(&display) {
            return false;
        }
    }
    true
}

/// Sum the on-disk size of a directory tree, in bytes.
///
/// Walks `root` without following symlinks and without crossing filesystem
/// boundaries — same rules as `safe_rm_contents`, so the size we report
/// matches what we'd actually free. Errors on unreadable entries are
/// silently skipped (the entry contributes 0 bytes), so a partially-readable
/// cache returns a *lower bound* on size rather than an error.
pub fn dir_size(root: &Path) -> io::Result<u64> {
    if !root.exists() {
        return Ok(0);
    }
    let mut total: u64 = 0;
    for entry in WalkDir::new(root)
        .follow_links(false)
        .same_file_system(true)
        .into_iter()
        .filter_map(Result::ok)
    {
        if let Ok(meta) = entry.metadata()
            && meta.is_file() {
                total = total.saturating_add(meta.len());
            }
    }
    Ok(total)
}

/// Remove the *contents* of `root` (not `root` itself), refusing to descend into
/// symlinks below the root and refusing to operate if the canonical root is at
/// or above a system path.
///
/// Returns the number of bytes that *were freed* (sum of file sizes removed).
/// On any error mid-walk, partial deletion is left in place — the caller may
/// retry; the next run will continue from where this one stopped.
///
/// **Policy summary** (from `spec/SPEC.md` §"Symlink Policy"):
/// - Cache root: canonicalize once via `prepare_root`. If the root is itself a
///   symlink, we operate inside the resolved target (respects users who
///   relocated caches off the main disk).
/// - Below the root: never follow symlinks. Symlink entries are removed via
///   `fs::remove_file` on the link itself — never traversed into.
/// - Sanity check: if the canonical root resolves to a system path, refuse.
#[cfg(test)]
pub fn safe_rm_contents(root: &Path) -> io::Result<u64> {
    safe_rm_contents_until(root, &AtomicBool::new(false))
}

/// `safe_rm_contents` that stops early once `cancel` is set, returning
/// `ErrorKind::Interrupted`. Checked before each entry, so at most one more
/// file is removed after cancellation.
pub fn safe_rm_contents_until(root: &Path, cancel: &AtomicBool) -> io::Result<u64> {
    if !root.exists() {
        return Ok(0);
    }
    let canonical = prepare_root(root)?;
    if !is_safe_root(&canonical) {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            format!(
                "refusing to clear {}: resolves to a system path",
                display_path(&canonical).display()
            ),
        ));
    }

    // Tally bytes BEFORE deletion (single walk; removing files invalidates
    // metadata). We walk depth-first so children are removed before parents.
    let mut freed: u64 = 0;

    // Collect all entries depth-first first; deleting during iteration is
    // fragile across platforms.
    let entries: Vec<_> = WalkDir::new(&canonical)
        .min_depth(1) // skip `canonical` itself — we delete contents only
        .follow_links(false) // NEVER follow symlinks below the root
        .same_file_system(true)
        .contents_first(true) // children before parents → safe rmdir order
        .into_iter()
        .filter_map(Result::ok)
        .take_while(|_| !cancel.load(Ordering::Relaxed))
        .collect();
    if cancel.load(Ordering::Relaxed) {
        return Err(cancelled());
    }

    for entry in entries {
        if cancel.load(Ordering::Relaxed) {
            return Err(cancelled());
        }
        let path = entry.path();
        let file_type = entry.file_type();

        if file_type.is_symlink() {
            // Remove the symlink itself, never traverse into it.
            // On Unix, remove_file works on symlinks of any kind.
            // On Windows, junctions/dir-symlinks need remove_dir; we try
            // remove_file first, fall back to remove_dir.
            if let Err(_e) = fs::remove_file(path) {
                let _ = fs::remove_dir(path);
            }
        } else if file_type.is_file() {
            if let Ok(meta) = entry.metadata() {
                let len = meta.len();
                if fs::remove_file(path).is_ok() {
                    freed = freed.saturating_add(len);
                }
            } else {
                let _ = fs::remove_file(path);
            }
        } else if file_type.is_dir() {
            // contents_first ensures the dir is empty by the time we reach it.
            let _ = fs::remove_dir(path);
        }
    }

    Ok(freed)
}

fn cancelled() -> io::Error {
    io::Error::new(io::ErrorKind::Interrupted, "cancelled (timed out)")
}

/// Sets the flag when dropped. Dropping the future that owns it (the
/// runner's `--timeout` firing) therefore stops the blocking walk.
struct CancelOnDrop(Arc<AtomicBool>);

impl Drop for CancelOnDrop {
    fn drop(&mut self) {
        self.0.store(true, Ordering::Relaxed);
    }
}

/// Async, interruptible `safe_rm_contents`.
///
/// Runs the walk on tokio's blocking pool, so the calling task yields and
/// `tokio::time::timeout` can fire. If this future is dropped before the
/// walk finishes, the walk stops at the next entry.
pub async fn rm_contents_cancellable(root: PathBuf) -> io::Result<u64> {
    let flag = Arc::new(AtomicBool::new(false));
    let _guard = CancelOnDrop(flag.clone());
    tokio::task::spawn_blocking(move || safe_rm_contents_until(&root, &flag))
        .await
        .map_err(io::Error::other)?
}

/// Display-friendly view of a path: on Windows, strip the `\\?\` prefix via
/// `dunce::simplified`. **Internal ops keep the prefix; only display strips.**
pub fn display_path(p: &Path) -> &Path {
    #[cfg(windows)]
    {
        dunce::simplified(p)
    }
    #[cfg(not(windows))]
    {
        p
    }
}

/// Try to acquire a non-blocking exclusive flock on `path`.
///
/// - `Ok(Some(file))` — we hold the lock; keep the `File` alive for the
///   duration of the cleanup. Dropping the file releases the lock.
/// - `Ok(None)` — the lock is held by another process. Caller should mark
///   the manager `SkippedBusy` and continue (per spec: don't wait, don't retry).
/// - `Err(_)` — real I/O problem (path missing, permission denied, etc.).
///
/// If the lock file does not exist, this returns `Ok(None)` — the assumption
/// being that no concurrent process can hold a lock that doesn't exist, so
/// it's safe to proceed (caller should treat it as "lock acquired").
pub fn try_lock_exclusive_nonblocking(path: &Path) -> io::Result<Option<File>> {
    if !path.exists() {
        return Ok(None);
    }
    let file = File::open(path)?;
    // std's `try_lock` reports contention as `TryLockError::WouldBlock` on
    // every platform, including Windows' ERROR_LOCK_VIOLATION (os error 33),
    // which a plain `io::ErrorKind` check would miss.
    match file.try_lock() {
        Ok(()) => Ok(Some(file)),
        Err(TryLockError::WouldBlock) => Ok(None),
        Err(TryLockError::Error(e)) => Err(e),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tree() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        for i in 0..20 {
            let p = dir.path().join(format!("d{i}")).join("f.bin");
            fs::create_dir_all(p.parent().unwrap()).unwrap();
            fs::write(p, b"x").unwrap();
        }
        dir
    }

    #[test]
    fn cancelled_walk_deletes_nothing() {
        let dir = tree();
        let err = safe_rm_contents_until(dir.path(), &AtomicBool::new(true)).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::Interrupted);
        assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 20);
    }

    #[test]
    fn uncancelled_walk_clears_contents_only() {
        let dir = tree();
        assert_eq!(safe_rm_contents(dir.path()).unwrap(), 20);
        assert!(dir.path().exists());
        assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 0);
    }

    #[test]
    fn dropping_guard_sets_cancel_flag() {
        let flag = Arc::new(AtomicBool::new(false));
        drop(CancelOnDrop(flag.clone()));
        assert!(flag.load(Ordering::Relaxed));
    }

    #[tokio::test]
    async fn cancellable_clears_when_not_dropped() {
        let dir = tree();
        assert_eq!(rm_contents_cancellable(dir.path().to_path_buf()).await.unwrap(), 20);
        assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 0);
    }
}
