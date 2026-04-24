//! bun: subprocess manager. `bun pm cache rm` clears `~/.bun/install/cache`.

use crate::manager::{Manager, ManagerResult, Plan, RunCtx};
use async_trait::async_trait;

pub struct Bun;

#[async_trait]
impl Manager for Bun {
    fn name(&self) -> &'static str { "bun" }
    async fn detect(&self) -> bool { false }
    fn plan(&self) -> Plan {
        Plan { description: "run `bun pm cache rm`".into(), paths: vec![] }
    }
    async fn execute(&self, _ctx: &RunCtx) -> ManagerResult {
        ManagerResult::SkippedMissing
    }
}
