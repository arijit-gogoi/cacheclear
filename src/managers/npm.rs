//! npm: subprocess manager. `npm cache clean --force` (the `--force` is required by npm).

use crate::manager::{Manager, ManagerResult, Plan, RunCtx};
use async_trait::async_trait;

pub struct Npm;

#[async_trait]
impl Manager for Npm {
    fn name(&self) -> &'static str { "npm" }
    async fn detect(&self) -> bool { false }
    fn plan(&self) -> Plan {
        Plan { description: "run `npm cache clean --force`".into(), paths: vec![] }
    }
    async fn execute(&self, _ctx: &RunCtx) -> ManagerResult {
        ManagerResult::SkippedMissing
    }
}
