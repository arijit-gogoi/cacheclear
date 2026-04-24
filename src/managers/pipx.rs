//! pipx: direct-deletion manager — pipx has no `cache clean` command (pypa/pipx#1681).
//! Cache dir: `$PIPX_HOME/.cache` if set, else `~/.local/pipx/.cache`
//! (Linux/macOS) or `%LOCALAPPDATA%\pipx\pipx\.cache` (Windows).
//! **Never touch `venvs/`** — those are installed tools, not cache.
//! Symlink rule applies: pipx ephemeral venvs symlink to system Python.

use crate::manager::{Manager, ManagerResult, Plan, RunCtx};
use async_trait::async_trait;

pub struct Pipx;

#[async_trait]
impl Manager for Pipx {
    fn name(&self) -> &'static str { "pipx" }
    async fn detect(&self) -> bool { false }
    fn plan(&self) -> Plan {
        Plan { description: "delete contents of pipx `.cache/`".into(), paths: vec![] }
    }
    async fn execute(&self, _ctx: &RunCtx) -> ManagerResult {
        ManagerResult::SkippedMissing
    }
}
