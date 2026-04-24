//! cargo: direct-deletion manager — cargo has no cache command.
//! Under `$CARGO_HOME` (default `~/.cargo`) we delete contents of:
//!   registry/cache/*  registry/src/*  git/db/*  git/checkouts/*
//! **Never touch `bin/`** (installed binaries) or **`registry/index/`** (metadata).
//!
//! Concurrency: try a non-blocking exclusive flock on `$CARGO_HOME/.package-cache`
//! before deleting; if held, return `SkippedBusy`.

use crate::manager::{Manager, ManagerResult, Plan, RunCtx};
use async_trait::async_trait;

pub struct Cargo;

#[async_trait]
impl Manager for Cargo {
    fn name(&self) -> &'static str { "cargo" }
    async fn detect(&self) -> bool { false }
    fn plan(&self) -> Plan {
        Plan {
            description: "delete contents of cargo `registry/cache`, `registry/src`, `git/db`, `git/checkouts`".into(),
            paths: vec![],
        }
    }
    async fn execute(&self, _ctx: &RunCtx) -> ManagerResult {
        ManagerResult::SkippedMissing
    }
}
