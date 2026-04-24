//! pipx: direct-deletion manager — pipx has no `cache clean` command (pypa/pipx#1681).
//! Cache dir: `$PIPX_HOME/.cache` if set, else `~/.local/pipx/.cache`
//! (Linux/macOS) or `%LOCALAPPDATA%\pipx\pipx\.cache` (Windows).
//! **Never touch `venvs/`** — those are installed tools, not cache.
//! Symlink rule applies: pipx ephemeral venvs symlink to system Python.

use crate::fs_util::{dir_size, safe_rm_contents};
use crate::manager::{Manager, ManagerResult, Plan, RunCtx};
use async_trait::async_trait;
use std::path::PathBuf;
use std::time::Instant;

pub struct Pipx;

/// Resolve the pipx cache directory.
///
/// Priority:
/// 1. `$PIPX_HOME/.cache` if `PIPX_HOME` is set
/// 2. Platform default:
///    - Windows: `%LOCALAPPDATA%\pipx\pipx\.cache`
///    - Linux/macOS: `~/.local/pipx/.cache`
fn pipx_cache_dir() -> Option<PathBuf> {
    if let Ok(env) = std::env::var("PIPX_HOME") {
        if !env.is_empty() {
            return Some(PathBuf::from(env).join(".cache"));
        }
    }
    #[cfg(windows)]
    {
        dirs::data_local_dir().map(|d| d.join("pipx").join("pipx").join(".cache"))
    }
    #[cfg(not(windows))]
    {
        dirs::home_dir().map(|h| h.join(".local").join("pipx").join(".cache"))
    }
}

#[async_trait]
impl Manager for Pipx {
    fn name(&self) -> &'static str {
        "pipx"
    }

    async fn detect(&self) -> bool {
        // Either pipx is on PATH, or its cache dir already exists (covers
        // the rare case where the binary isn't on PATH but a previous
        // install left the cache behind).
        if which::which("pipx").is_ok() {
            return true;
        }
        pipx_cache_dir().map(|p| p.exists()).unwrap_or(false)
    }

    fn plan(&self) -> Plan {
        Plan {
            description: "delete contents of pipx `.cache/` (NEVER `venvs/`)".into(),
            paths: pipx_cache_dir().into_iter().collect(),
        }
    }

    async fn execute(&self, ctx: &RunCtx) -> ManagerResult {
        let Some(cache) = pipx_cache_dir() else {
            return ManagerResult::Failed {
                error: "could not resolve pipx cache directory".into(),
            };
        };
        if !cache.exists() {
            // Nothing to do; report success with 0 freed.
            return ManagerResult::Done {
                freed: 0,
                took: std::time::Duration::ZERO,
            };
        }

        let started = Instant::now();
        if ctx.dry_run {
            let freed = dir_size(&cache).unwrap_or(0);
            return ManagerResult::Done {
                freed,
                took: started.elapsed(),
            };
        }

        match safe_rm_contents(&cache) {
            Ok(freed) => ManagerResult::Done {
                freed,
                took: started.elapsed(),
            },
            Err(e) => ManagerResult::Failed {
                error: format!("clearing {}: {e}", cache.display()),
            },
        }
    }
}
