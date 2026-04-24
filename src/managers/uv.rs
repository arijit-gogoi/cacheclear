//! uv: subprocess manager. `uv cache clean` is documented cache-only.
//! Detection: `uv --version`.

use crate::manager::{Manager, ManagerResult, Plan, RunCtx};
use async_trait::async_trait;

pub struct Uv;

#[async_trait]
impl Manager for Uv {
    fn name(&self) -> &'static str { "uv" }
    async fn detect(&self) -> bool { false }
    fn plan(&self) -> Plan {
        Plan { description: "run `uv cache clean`".into(), paths: vec![] }
    }
    async fn execute(&self, _ctx: &RunCtx) -> ManagerResult {
        ManagerResult::SkippedMissing
    }
}
