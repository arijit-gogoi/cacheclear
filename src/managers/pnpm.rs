//! pnpm: subprocess manager. `pnpm store prune` removes unreferenced packages
//! from the content-addressable store. **Never wipe the store dir wholesale** —
//! every pnpm project on the machine symlinks into it.
//!
//! Concurrency: try a non-blocking lock on the store's lock file
//! (discover via `pnpm store path`); if held, return `SkippedBusy`.

use crate::manager::{Manager, ManagerResult, Plan, RunCtx};
use async_trait::async_trait;

pub struct Pnpm;

#[async_trait]
impl Manager for Pnpm {
    fn name(&self) -> &'static str { "pnpm" }
    async fn detect(&self) -> bool { false }
    fn plan(&self) -> Plan {
        Plan { description: "run `pnpm store prune`".into(), paths: vec![] }
    }
    async fn execute(&self, _ctx: &RunCtx) -> ManagerResult {
        ManagerResult::SkippedMissing
    }
}
