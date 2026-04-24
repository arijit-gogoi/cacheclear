//! Parallel execution + per-manager timeouts.
//!
//! One `tokio::spawn` per manager, joined with `futures::future::join_all`.
//! Per-task timeout via `tokio::time::timeout`. Progress reporting flows into
//! `report` through a channel (wired in Phase 1).

use crate::manager::{Manager, ManagerResult, RunCtx};
use std::sync::Arc;

/// Run every manager in parallel, honouring `ctx.timeout` per task.
/// Stub: real implementation lands when manager modules are filled in.
pub async fn run_all(_managers: Vec<Arc<dyn Manager>>, _ctx: RunCtx) -> Vec<(String, ManagerResult)> {
    Vec::new()
}
