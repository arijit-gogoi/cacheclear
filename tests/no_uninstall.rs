//! Phase 1 exit criterion (per `spec/SPEC.md`):
//!
//! > "Integration test: create fake caches in a sandboxed `$HOME`, run,
//! >  assert only cache paths change and no 'installed' paths
//! >  (bin dirs, venvs) are touched."
//!
//! Strategy:
//! 1. Build a fake `$HOME` in a `tempfile::TempDir`. Populate it with both
//!    cache trees (which we expect to be cleared) AND installed-binary trees
//!    (which the no-uninstall invariant forbids us from touching).
//! 2. Override `HOME` (and Windows equivalents) so `dirs::home_dir()` resolves
//!    inside the tempdir.
//! 3. Override `PATH` so detection of real package managers can't sneak in.
//! 4. Run the `cacheclear` binary via `assert_cmd`.
//! 5. Assert: every cache path was emptied (or unchanged if there was nothing
//!    to clear), every "installed" path is byte-identical to before.
//!
//! This is the safety net behind the no-uninstall invariant. If anyone ever
//! breaks `safe_rm_contents`, breaks `is_safe_root`, or accidentally points
//! a manager at the wrong dir, this test fails.

use assert_cmd::Command;
use std::collections::BTreeMap;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use tempfile::TempDir;

/// All the file paths we lay down in the fake `$HOME`.
///
/// `cache_*` paths are expected to be EMPTIED (file removed).
/// `installed_*` paths are FORBIDDEN to touch (must remain byte-identical).
struct FakeHome {
    root: TempDir,
    /// Paths that the tool is allowed to delete (and that we expect to be
    /// gone after the run).
    cache_paths: Vec<PathBuf>,
    /// Paths that the tool MUST NOT delete or modify. Our invariant.
    installed_paths: Vec<PathBuf>,
}

impl FakeHome {
    fn new() -> io::Result<Self> {
        let root = tempfile::tempdir()?;
        let home = root.path();

        let mut cache_paths = Vec::new();
        let mut installed_paths = Vec::new();

        // ----- cargo: cache dirs (delete-allowed) + bin & registry/index (FORBIDDEN) -----
        // Layout under $HOME/.cargo:
        let cargo = home.join(".cargo");
        // Allowed-to-delete:
        for sub in [
            "registry/cache/index.crates.io-abc/foo-1.0.0.crate",
            "registry/src/index.crates.io-abc/foo-1.0.0/src/lib.rs",
            "git/db/cargo-test-repo-12345/HEAD",
            "git/checkouts/cargo-test-repo-12345/abcd1234/Cargo.toml",
        ] {
            let p = cargo.join(sub);
            write_with_parents(&p, b"cargo cache content\n")?;
            cache_paths.push(p);
        }
        // FORBIDDEN to delete:
        for sub in [
            "bin/cargo.exe",
            "bin/rustc.exe",
            "registry/index/index.crates.io-abc/.git/HEAD",
            "config.toml",
        ] {
            let p = cargo.join(sub);
            write_with_parents(&p, b"installed binary or metadata\n")?;
            installed_paths.push(p);
        }

        // ----- pipx: .cache/ (delete-allowed) + venvs/ (FORBIDDEN) -----
        // Layout under $HOME/.local/pipx (Linux/macOS path):
        let pipx_lin = home.join(".local").join("pipx");
        for sub in [
            ".cache/somehash-3.11/bin/python",
            ".cache/somehash-3.11/lib/python3.11/site.py",
        ] {
            let p = pipx_lin.join(sub);
            write_with_parents(&p, b"pipx ephemeral venv (cache)\n")?;
            cache_paths.push(p);
        }
        for sub in [
            "venvs/black/bin/black",
            "venvs/black/lib/python3.11/site-packages/black/__init__.py",
            "venvs/ruff/bin/ruff.exe",
        ] {
            let p = pipx_lin.join(sub);
            write_with_parents(&p, b"INSTALLED TOOL via pipx\n")?;
            installed_paths.push(p);
        }
        // Same for the Windows-default location: %LOCALAPPDATA%\pipx\pipx
        // (we'll point LOCALAPPDATA at $HOME\AppData\Local in the env block).
        let pipx_win = home.join("AppData").join("Local").join("pipx").join("pipx");
        for sub in [".cache/winhash-3.11/bin/python.exe"] {
            let p = pipx_win.join(sub);
            write_with_parents(&p, b"pipx ephemeral venv (cache, win)\n")?;
            cache_paths.push(p);
        }
        for sub in ["venvs/black/Scripts/black.exe"] {
            let p = pipx_win.join(sub);
            write_with_parents(&p, b"INSTALLED TOOL via pipx (win)\n")?;
            installed_paths.push(p);
        }

        // ----- random unrelated content (FORBIDDEN to touch) -----
        // Just a sanity check: a file completely unrelated to any cache must
        // also remain untouched.
        for sub in [
            "Documents/secret.txt",
            ".bashrc",
            "projects/my-app/src/main.rs",
        ] {
            let p = home.join(sub);
            write_with_parents(&p, "USER DATA - do not touch\n".as_bytes())?;
            installed_paths.push(p);
        }

        Ok(Self {
            root,
            cache_paths,
            installed_paths,
        })
    }

    fn home(&self) -> &Path {
        self.root.path()
    }
}

/// Write `bytes` to `path`, creating parent directories as needed.
fn write_with_parents(path: &Path, bytes: &[u8]) -> io::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(path, bytes)
}

#[test]
fn no_uninstall_invariant_holds_against_fake_home() {
    let fake = FakeHome::new().expect("setting up fake $HOME");

    // Snapshot the FORBIDDEN paths' contents BEFORE running.
    let installed_before: BTreeMap<PathBuf, Vec<u8>> = fake
        .installed_paths
        .iter()
        .map(|p| (p.clone(), fs::read(p).expect("reading installed path")))
        .collect();

    // Build env that quarantines the run inside the fake $HOME:
    // - HOME / USERPROFILE → fake home (so dirs::home_dir() resolves inside)
    // - LOCALAPPDATA → fake AppData\Local (so pipx Windows default resolves inside)
    // - CARGO_HOME → fake $HOME/.cargo (belt-and-suspenders against env leakage)
    // - PIPX_HOME unset (we want the platform default to be used so we exercise
    //   pipx_cache_dir's defaulting logic)
    // - PATH cleared to a single non-existent dir, so detect()'s which::which
    //   calls all return false. The managers should fall back to "cache dir
    //   exists?" detection where applicable (cargo, pipx).
    let home = fake.home();
    let fake_path = home.join("__no_real_binaries__");
    fs::create_dir_all(&fake_path).unwrap();

    let mut cmd = Command::cargo_bin("cacheclear").expect("locating cacheclear binary");
    cmd.env_clear()
        .env("HOME", home)
        .env("USERPROFILE", home)
        .env("LOCALAPPDATA", home.join("AppData").join("Local"))
        .env("CARGO_HOME", home.join(".cargo"))
        .env("PATH", &fake_path);

    // SystemRoot is required on Windows for many syscalls; preserve it.
    if let Ok(sr) = std::env::var("SystemRoot") {
        cmd.env("SystemRoot", sr);
    }

    let output = cmd.output().expect("running cacheclear");
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    eprintln!("--- cacheclear stdout ---\n{stdout}");
    eprintln!("--- cacheclear stderr ---\n{stderr}");
    // Exit status: should be 0 (everything either ran or was skipped). If
    // any manager Failed, that's a test failure — print so we can see why.
    assert!(
        output.status.success(),
        "cacheclear exited with non-zero status. stdout:\n{stdout}\nstderr:\n{stderr}"
    );

    // ---- INVARIANT 1: every installed path remains byte-identical ----
    for (path, before_bytes) in &installed_before {
        let after_bytes = fs::read(path).unwrap_or_else(|e| {
            panic!(
                "FORBIDDEN path was deleted: {} (err: {e})",
                path.display()
            )
        });
        assert_eq!(
            *before_bytes,
            after_bytes,
            "FORBIDDEN path was modified: {}",
            path.display()
        );
    }

    // ---- INVARIANT 2: cache files were cleared ----
    //
    // Only assert deletion for cache files whose owning manager is BOTH
    // (a) detectable in this sandbox (no real binaries on PATH, so detection
    //     must succeed via cache-dir existence — true for cargo and pipx, false
    //     for the five subprocess managers), AND
    // (b) using the per-platform default cache path (e.g. on Windows, pipx
    //     uses %LOCALAPPDATA%\pipx\pipx\.cache, NOT ~/.local/pipx/.cache).
    //
    // We populated BOTH platform layouts so the test is portable; on each
    // platform, only the in-platform layout's cache files are expected to
    // disappear. The cross-platform files are documentation, not assertions.
    let mut deletion_assertions = 0_usize;
    for cache_path in &fake.cache_paths {
        if !is_targeted_by_current_platform(cache_path, fake.home()) {
            continue;
        }
        deletion_assertions += 1;
        assert!(
            !cache_path.exists(),
            "cache file should have been deleted but survived: {}",
            cache_path.display()
        );
    }
    // Sanity: at least cargo's 4 files should always be in the assertion set
    // since cargo's layout is identical across platforms.
    assert!(
        deletion_assertions >= 4,
        "expected at least 4 cache deletion assertions (cargo's 4 dirs), got {deletion_assertions}"
    );

    // ---- INVARIANT 3: no NEW files appeared anywhere outside the home tempdir ----
    // (We can't easily assert this on the whole filesystem, but we DO assert
    // that the install paths still exist and have correct content, which
    // covers the most likely failure mode.)

    // The TempDir auto-cleans on drop; keep `fake` alive until here.
    drop(fake);
}

/// True if this cache file is owned by a direct-deletion manager AND lives
/// at the path that THIS platform's manager impl actually targets.
fn is_targeted_by_current_platform(path: &Path, home: &Path) -> bool {
    let rel = path.strip_prefix(home).unwrap_or(path);
    let s = rel.to_string_lossy().replace('\\', "/");

    // cargo: layout is the same on every platform — always asserted.
    if s.starts_with(".cargo/") {
        return true;
    }

    // pipx: per-platform default split.
    #[cfg(windows)]
    {
        if s.starts_with("AppData/Local/pipx/") {
            return true;
        }
    }
    #[cfg(not(windows))]
    {
        if s.starts_with(".local/pipx/") {
            return true;
        }
    }

    false
}

