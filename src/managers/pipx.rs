//! pipx: direct-deletion manager — pipx has no `cache clean` command (pypa/pipx#1681).
//!
//! pipx keeps ephemeral `pipx run` venvs in its venv cache dir
//! (`PIPX_VENV_CACHEDIR`). Its location changed in pipx 1.3, so we clear
//! every pipx cache dir that exists (see `cache_dirs`).
//! **Never touch `venvs/`** — those are installed tools, not cache.
//! Symlink rule applies: pipx ephemeral venvs symlink to system Python.

use crate::fs_util::{dir_size, rm_contents_cancellable};
use crate::manager::{Manager, ManagerResult, Plan, RunCtx};
use async_trait::async_trait;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Instant;

pub struct Pipx;

/// Every pipx cache dir that exists on this machine, deduplicated.
///
/// Sources, in order:
/// 1. `$PIPX_HOME/.cache` if `PIPX_HOME` is set (pipx puts all its dirs
///    under `PIPX_HOME` when the variable is set).
/// 2. pipx itself, if on PATH: `pipx environment --value PIPX_VENV_CACHEDIR`,
///    plus `<PIPX_HOME>/.cache` from `--value PIPX_HOME` (pre-1.3 layout,
///    often left behind after an upgrade).
/// 3. Platform defaults, for when pipx is missing or too old to answer:
///    - pipx ≥ 1.3 (platformdirs): `%LOCALAPPDATA%\pipx\pipx\Cache` (Windows),
///      `~/Library/Caches/pipx` (macOS), `$XDG_CACHE_HOME/pipx` or
///      `~/.cache/pipx` (Linux)
///    - pre-1.3 homes: `~/.local/pipx/.cache`, and `~\pipx\.cache` (Windows)
fn cache_dirs() -> Vec<PathBuf> {
    let mut candidates: Vec<PathBuf> = Vec::new();

    if let Some(home) = std::env::var_os("PIPX_HOME").filter(|v| !v.is_empty()) {
        candidates.push(PathBuf::from(home).join(".cache"));
    }

    if let Ok(pipx) = which::which("pipx") {
        if let Some(dir) = query_pipx(&pipx, "PIPX_VENV_CACHEDIR") {
            candidates.push(dir);
        }
        if let Some(home) = query_pipx(&pipx, "PIPX_HOME") {
            candidates.push(home.join(".cache"));
        }
    }

    candidates.extend(default_cache_dirs());

    let mut out: Vec<PathBuf> = Vec::new();
    for c in candidates {
        if c.is_dir() && !out.iter().any(|o| same_dir(o, &c)) {
            out.push(c);
        }
    }
    out
}

/// Ask pipx for one of its paths. `None` if pipx fails or prints nothing
/// (pipx before 1.1 has no `environment --value`).
fn query_pipx(pipx: &Path, var: &str) -> Option<PathBuf> {
    let output = Command::new(pipx)
        .args(["environment", "--value", var])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let s = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if s.is_empty() {
        return None;
    }
    let p = PathBuf::from(s);
    p.is_absolute().then_some(p)
}

/// Static fallback locations (see `cache_dirs`).
fn default_cache_dirs() -> Vec<PathBuf> {
    let mut out = Vec::new();
    let home = crate::fs_util::home_dir();

    #[cfg(windows)]
    {
        // Prefer the env var so a sandboxed LOCALAPPDATA is honoured.
        let local = std::env::var_os("LOCALAPPDATA")
            .filter(|v| !v.is_empty())
            .map(PathBuf::from)
            .or_else(dirs::data_local_dir);
        if let Some(local) = local {
            out.push(local.join("pipx").join("pipx").join("Cache"));
        }
        if let Some(h) = &home {
            out.push(h.join("pipx").join(".cache"));
        }
    }
    #[cfg(not(windows))]
    {
        if let Some(cache) = dirs::cache_dir() {
            out.push(cache.join("pipx"));
        }
    }

    if let Some(h) = &home {
        out.push(h.join(".local").join("pipx").join(".cache"));
    }
    out
}

/// True if both paths name the same directory (compares canonical forms,
/// so case and `\\?\` differences on Windows don't produce duplicates).
fn same_dir(a: &Path, b: &Path) -> bool {
    match (std::fs::canonicalize(a), std::fs::canonicalize(b)) {
        (Ok(a), Ok(b)) => a == b,
        _ => a == b,
    }
}

#[async_trait]
impl Manager for Pipx {
    fn name(&self) -> &'static str {
        "pipx"
    }

    async fn detect(&self) -> bool {
        // Either pipx is on PATH, or a cache dir already exists (covers
        // the case where the binary isn't on PATH but a previous install
        // left the cache behind).
        if which::which("pipx").is_ok() {
            return true;
        }
        !cache_dirs().is_empty()
    }

    fn plan(&self) -> Plan {
        Plan {
            description: "delete contents of pipx venv cache dirs (NEVER `venvs/`)".into(),
            paths: cache_dirs(),
        }
    }

    async fn execute(&self, ctx: &RunCtx) -> ManagerResult {
        let started = Instant::now();
        let mut total_freed: u64 = 0;

        // No existing cache dir means nothing to clear: Done with 0 freed.
        for cache in cache_dirs() {
            if ctx.dry_run {
                total_freed = total_freed.saturating_add(dir_size(&cache).unwrap_or(0));
                continue;
            }
            match rm_contents_cancellable(cache.clone()).await {
                Ok(freed) => total_freed = total_freed.saturating_add(freed),
                Err(e) => {
                    return ManagerResult::Failed {
                        error: format!("clearing {}: {e}", cache.display()),
                    };
                }
            }
        }

        ManagerResult::Done {
            freed: total_freed,
            took: started.elapsed(),
        }
    }
}
