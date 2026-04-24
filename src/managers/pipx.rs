//! pipx: direct-deletion manager — pipx has no `cache clean` command (pypa/pipx#1681).
//! Cache dir: `$PIPX_HOME/.cache` if set, else `~/.local/pipx/.cache`
//! (Linux/macOS) or `%LOCALAPPDATA%\pipx\pipx\.cache` (Windows).
//! **Never touch `venvs/`** — those are installed tools, not cache.
//! Symlink rule applies: pipx ephemeral venvs symlink to system Python.
//!
//! NOTE: detection is wired (PATH lookup for `pipx`); the actual
//! cache-dir resolution + safe deletion lands with the cargo/pipx pass
//! (uses `fs_util::dir_size` + `fs_util::safe_rm_contents`, not yet real).

use crate::manager::{Manager, ManagerResult, Plan, RunCtx};
use async_trait::async_trait;

pub struct Pipx;

#[async_trait]
impl Manager for Pipx {
    fn name(&self) -> &'static str {
        "pipx"
    }

    async fn detect(&self) -> bool {
        which::which("pipx").is_ok()
    }

    fn plan(&self) -> Plan {
        Plan {
            description: "delete contents of pipx `.cache/`".into(),
            paths: vec![],
        }
    }

    async fn execute(&self, _ctx: &RunCtx) -> ManagerResult {
        // TODO(direct-deletion pass): resolve $PIPX_HOME/.cache or platform default,
        // run prepare_root + safe_rm_contents, return Done { freed, took }.
        ManagerResult::Failed {
            error: "pipx direct-deletion not yet implemented".into(),
        }
    }
}
