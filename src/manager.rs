//! Core `Manager` trait — the contract every cache cleaner implements.
//!
//! Two implementations exist in `managers/`:
//! - subprocess managers (uv, pip, npm, pnpm, bun) shell out to a documented cache command
//! - direct-deletion managers (cargo, pipx) walk a cache directory and remove contents
//!   under the rules in `fs_util` (canonicalize root, never follow symlinks below)

use async_trait::async_trait;
use std::time::Duration;

/// What a manager will do, surfaced by `--dry-run`.
#[derive(Debug, Clone)]
pub struct Plan {
    /// Human-readable description (e.g. "run `uv cache clean`" or "delete contents of …").
    pub description: String,
    /// Concrete paths that will be touched (empty for subprocess-only managers).
    pub paths: Vec<std::path::PathBuf>,
}

/// Result of running a single manager.
#[derive(Debug)]
pub enum ManagerResult {
    /// Cleared successfully; how many bytes were freed.
    Done { freed: u64, took: Duration },
    /// Detection failed — manager not installed.
    SkippedMissing,
    /// Cache was busy (cargo lockfile held / pnpm store locked).
    SkippedBusy { reason: String },
    /// Something went wrong; not catastrophic, just this one manager.
    Failed { error: String },
}

/// Per-run shared context — verbosity, timeout, dry-run, color preference.
#[derive(Debug, Clone)]
pub struct RunCtx {
    pub verbose: bool,
    pub dry_run: bool,
    pub timeout: Duration,
    pub no_color: bool,
}

/// The core trait. Every manager module implements this.
#[async_trait]
pub trait Manager: Send + Sync {
    /// Stable identifier (e.g. `"cargo"`).
    fn name(&self) -> &'static str;

    /// True if this manager is installed and clearable.
    async fn detect(&self) -> bool;

    /// What the manager would do — for `--dry-run` and the planning phase.
    fn plan(&self) -> Plan;

    /// Actually clear. Must respect `ctx.dry_run` (return `Done { freed: 0 }` if true).
    async fn execute(&self, ctx: &RunCtx) -> ManagerResult;
}
