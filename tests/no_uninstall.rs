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
//!
//! Only cargo and pipx are exercised: with no real binaries on PATH, they are
//! the only managers detectable (via cache-dir existence).
//!
//! Other tests in this file cover `--only` / `--skip`, `--dry-run` touching
//! nothing, a busy cargo lock, and Windows long paths.

use assert_cmd::Command;
use std::collections::BTreeMap;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::process::Output;
use tempfile::TempDir;

/// All the file paths we lay down in the fake `$HOME`.
struct FakeHome {
    root: TempDir,
    /// cargo cache files: expected to be deleted when cargo runs.
    cargo_cache: Vec<PathBuf>,
    /// pipx cache files: expected to be deleted when pipx runs.
    pipx_cache: Vec<PathBuf>,
    /// Paths that the tool MUST NOT delete or modify. Our invariant.
    installed_paths: Vec<PathBuf>,
}

/// pipx layout for the current platform: (cache files, installed files),
/// relative to `$HOME`. Mirrors `managers::pipx::default_cache_dirs`.
fn pipx_layout() -> (Vec<&'static str>, Vec<&'static str>) {
    // Pre-1.3 home, used on every platform.
    let mut cache = vec![".local/pipx/.cache/legacyhash/bin/python"];
    let mut installed = vec![
        ".local/pipx/venvs/black/bin/black",
        ".local/pipx/venvs/black/lib/python3.11/site-packages/black/__init__.py",
    ];
    if cfg!(windows) {
        // pipx >= 1.3 venv cache (platformdirs) + Windows `~\pipx` home.
        cache.push("AppData/Local/pipx/pipx/Cache/winhash/Scripts/python.exe");
        cache.push("pipx/.cache/oldwinhash/Scripts/python.exe");
        installed.push("pipx/venvs/ruff/Scripts/ruff.exe");
        installed.push("AppData/Local/pipx/pipx/venvs/black/Scripts/black.exe");
    } else if cfg!(target_os = "macos") {
        cache.push("Library/Caches/pipx/machash/bin/python");
        installed.push(".local/share/pipx/venvs/ruff/bin/ruff");
    } else {
        cache.push(".cache/pipx/linhash/bin/python");
        installed.push(".local/share/pipx/venvs/ruff/bin/ruff");
    }
    (cache, installed)
}

impl FakeHome {
    fn new() -> io::Result<Self> {
        let root = tempfile::tempdir()?;
        let home = root.path();

        let mut cargo_cache = Vec::new();
        let mut pipx_cache = Vec::new();
        let mut installed_paths = Vec::new();

        // ----- cargo: cache dirs (delete-allowed) + bin & registry/index (FORBIDDEN) -----
        let cargo = home.join(".cargo");
        for sub in [
            "registry/cache/index.crates.io-abc/foo-1.0.0.crate",
            "registry/src/index.crates.io-abc/foo-1.0.0/src/lib.rs",
            "git/db/cargo-test-repo-12345/HEAD",
            "git/checkouts/cargo-test-repo-12345/abcd1234/Cargo.toml",
        ] {
            let p = cargo.join(sub);
            write_with_parents(&p, b"cargo cache content\n")?;
            cargo_cache.push(p);
        }
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

        // ----- pipx: cache dirs (delete-allowed) + venvs/ (FORBIDDEN) -----
        let (pipx_c, pipx_i) = pipx_layout();
        for sub in pipx_c {
            let p = home.join(sub);
            write_with_parents(&p, b"pipx ephemeral venv (cache)\n")?;
            pipx_cache.push(p);
        }
        for sub in pipx_i {
            let p = home.join(sub);
            write_with_parents(&p, b"INSTALLED TOOL via pipx\n")?;
            installed_paths.push(p);
        }

        // ----- random unrelated content (FORBIDDEN to touch) -----
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
            cargo_cache,
            pipx_cache,
            installed_paths,
        })
    }

    fn home(&self) -> &Path {
        self.root.path()
    }

    /// Run `cacheclear` with `args`, quarantined inside the fake `$HOME`:
    /// - HOME / USERPROFILE → fake home (so `dirs::home_dir()` resolves inside)
    /// - LOCALAPPDATA → fake AppData\Local (pipx Windows default)
    /// - CARGO_HOME → fake `$HOME/.cargo`
    /// - PIPX_HOME / XDG_CACHE_HOME unset (env cleared), so defaults are used
    /// - PATH → one empty dir, so no real package manager is detected
    fn run(&self, args: &[&str]) -> Output {
        let home = self.home();
        let fake_path = home.join("__no_real_binaries__");
        fs::create_dir_all(&fake_path).unwrap();

        let mut cmd = Command::cargo_bin("cacheclear").expect("locating cacheclear binary");
        cmd.env_clear()
            .env("HOME", home)
            .env("USERPROFILE", home)
            .env("LOCALAPPDATA", home.join("AppData").join("Local"))
            .env("CARGO_HOME", home.join(".cargo"))
            .env("PATH", &fake_path)
            .args(args);
        // SystemRoot is required on Windows for many syscalls; preserve it.
        if let Ok(sr) = std::env::var("SystemRoot") {
            cmd.env("SystemRoot", sr);
        }

        let output = cmd.output().expect("running cacheclear");
        eprintln!(
            "--- cacheclear {args:?} (exit {:?}) ---\nstdout:\n{}\nstderr:\n{}",
            output.status.code(),
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr),
        );
        output
    }

    fn snapshot_installed(&self) -> BTreeMap<PathBuf, Vec<u8>> {
        self.installed_paths
            .iter()
            .map(|p| (p.clone(), fs::read(p).expect("reading installed path")))
            .collect()
    }
}

/// Write `bytes` to `path`, creating parent directories as needed.
fn write_with_parents(path: &Path, bytes: &[u8]) -> io::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(path, bytes)
}

/// Every installed path must still exist with identical content.
fn assert_installed_untouched(before: &BTreeMap<PathBuf, Vec<u8>>) {
    for (path, before_bytes) in before {
        let after_bytes = fs::read(path).unwrap_or_else(|e| {
            panic!("FORBIDDEN path was deleted: {} (err: {e})", path.display())
        });
        assert_eq!(
            *before_bytes,
            after_bytes,
            "FORBIDDEN path was modified: {}",
            path.display()
        );
    }
}

fn assert_all_gone(paths: &[PathBuf]) {
    for p in paths {
        assert!(!p.exists(), "cache file should have been deleted: {}", p.display());
    }
}

fn assert_all_present(paths: &[PathBuf]) {
    for p in paths {
        assert!(p.exists(), "cache file should NOT have been deleted: {}", p.display());
    }
}

/// Every file under `root` with its content. Used to prove nothing changed.
fn snapshot_tree(root: &Path) -> BTreeMap<PathBuf, Vec<u8>> {
    let mut out = BTreeMap::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        for entry in fs::read_dir(&dir).expect("reading dir") {
            let entry = entry.expect("dir entry");
            let path = entry.path();
            let ft = entry.file_type().expect("file type");
            if ft.is_dir() {
                // Record directories too, so a removed empty dir shows up.
                out.insert(path.clone(), Vec::new());
                stack.push(path);
            } else {
                out.insert(path.clone(), fs::read(&path).expect("reading file"));
            }
        }
    }
    out
}

fn stdout(o: &Output) -> String {
    String::from_utf8_lossy(&o.stdout).into_owned()
}

#[test]
fn no_uninstall_invariant_holds_against_fake_home() {
    let fake = FakeHome::new().expect("setting up fake $HOME");
    let installed_before = fake.snapshot_installed();

    let output = fake.run(&[]);
    assert!(output.status.success(), "cacheclear exited non-zero");

    assert_installed_untouched(&installed_before);
    assert_all_gone(&fake.cargo_cache);
    assert_all_gone(&fake.pipx_cache);
}

#[test]
fn only_pipx_leaves_cargo_cache_alone() {
    let fake = FakeHome::new().expect("setting up fake $HOME");
    let installed_before = fake.snapshot_installed();

    let output = fake.run(&["--only", "pipx"]);
    assert!(output.status.success());
    let out = stdout(&output);
    assert!(out.contains("pipx"), "pipx missing from report");
    assert!(!out.contains("cargo"), "filtered-out cargo appears in report");

    assert_installed_untouched(&installed_before);
    assert_all_present(&fake.cargo_cache);
    assert_all_gone(&fake.pipx_cache);
}

#[test]
fn skip_cargo_leaves_cargo_cache_alone() {
    let fake = FakeHome::new().expect("setting up fake $HOME");
    let installed_before = fake.snapshot_installed();

    let output = fake.run(&["--skip", "cargo,npm"]);
    assert!(output.status.success());
    assert!(!stdout(&output).contains("cargo"));

    assert_installed_untouched(&installed_before);
    assert_all_present(&fake.cargo_cache);
    assert_all_gone(&fake.pipx_cache);
}

#[test]
fn unknown_manager_name_is_usage_error() {
    let fake = FakeHome::new().expect("setting up fake $HOME");
    // The PATH dir is created by `run`; create it first so it is in both snapshots.
    fs::create_dir_all(fake.home().join("__no_real_binaries__")).unwrap();
    let before = snapshot_tree(fake.home());

    for args in [&["--only", "cargo,nope"][..], &["--skip", "nope"][..]] {
        let output = fake.run(args);
        assert_eq!(output.status.code(), Some(2), "args {args:?}");
    }
    assert_eq!(before, snapshot_tree(fake.home()), "usage error modified files");
}

#[test]
fn dry_run_touches_nothing() {
    let fake = FakeHome::new().expect("setting up fake $HOME");
    // The PATH dir is created by `run`; create it first so it is in both snapshots.
    fs::create_dir_all(fake.home().join("__no_real_binaries__")).unwrap();
    let before = snapshot_tree(fake.home());

    for args in [&["--dry-run"][..], &["--dry-run", "--json"][..]] {
        let output = fake.run(args);
        assert!(output.status.success(), "args {args:?}");
    }

    assert_eq!(before, snapshot_tree(fake.home()), "--dry-run modified files");
}

#[test]
fn busy_cargo_lock_is_skipped_not_failed() {
    let fake = FakeHome::new().expect("setting up fake $HOME");
    let installed_before = fake.snapshot_installed();

    // Hold cargo's package-cache lock, as a running `cargo build` would.
    let lock_path = fake.home().join(".cargo").join(".package-cache");
    let lock = fs::File::create(&lock_path).unwrap();
    lock.lock().unwrap();

    let output = fake.run(&["--only", "cargo"]);
    let out = stdout(&output);
    assert!(output.status.success(), "busy cargo must exit 0");
    assert!(out.contains("busy"), "expected a busy skip, got:\n{out}");

    lock.unlock().unwrap();
    assert_installed_untouched(&installed_before);
    assert_all_present(&fake.cargo_cache);
}

#[test]
fn long_paths_are_cleared() {
    let fake = FakeHome::new().expect("setting up fake $HOME");
    let installed_before = fake.snapshot_installed();

    // Build a path well past Windows MAX_PATH (260) under registry/src.
    // std::fs handles long absolute paths itself, so creating it works.
    let mut deep = fake.home().join(".cargo").join("registry").join("src");
    while deep.as_os_str().len() < 320 {
        deep.push("a-fairly-long-directory-name-0123456789");
    }
    let deep_file = deep.join("lib.rs");
    write_with_parents(&deep_file, b"deep cache file\n").unwrap();
    assert!(deep_file.as_os_str().len() > 300);

    let output = fake.run(&["--only", "cargo"]);
    assert!(output.status.success());

    assert!(!deep_file.exists(), "long-path cache file survived");
    let src = fake.home().join(".cargo").join("registry").join("src");
    assert_eq!(
        fs::read_dir(&src).unwrap().count(),
        0,
        "registry/src should be empty"
    );
    assert_installed_untouched(&installed_before);
    assert_all_gone(&fake.cargo_cache);
}

/// Phase 2 exit criterion: `--json` output validates against
/// `schema/output.schema.json`, in both run and dry-run modes.
#[test]
fn json_output_matches_schema() {
    let schema: serde_json::Value = serde_json::from_str(
        &fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("schema/output.schema.json"))
            .expect("reading schema"),
    )
    .expect("schema is JSON");
    let validator = jsonschema::validator_for(&schema).expect("schema compiles");

    let fake = FakeHome::new().expect("setting up fake $HOME");
    for (args, mode) in [
        (&["--json", "--dry-run"][..], "dry_run"),
        (&["--json"][..], "run"),
    ] {
        let output = fake.run(args);
        assert!(output.status.success(), "args {args:?}");
        let doc: serde_json::Value =
            serde_json::from_slice(&output.stdout).expect("stdout is one JSON document");
        let errors: Vec<String> = validator.iter_errors(&doc).map(|e| e.to_string()).collect();
        assert!(errors.is_empty(), "args {args:?}: schema errors: {errors:#?}");
        assert_eq!(doc["mode"], mode);
    }
}
