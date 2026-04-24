//! pip: subprocess manager. `pip cache purge` is documented cache-only.
//! Detection: `pip --version`, with `pip3` fallback.

use crate::manager::{Manager, ManagerResult, Plan, RunCtx};
use async_trait::async_trait;

pub struct Pip;

#[async_trait]
impl Manager for Pip {
    fn name(&self) -> &'static str { "pip" }
    async fn detect(&self) -> bool { false }
    fn plan(&self) -> Plan {
        Plan { description: "run `pip cache purge`".into(), paths: vec![] }
    }
    async fn execute(&self, _ctx: &RunCtx) -> ManagerResult {
        ManagerResult::SkippedMissing
    }
}
