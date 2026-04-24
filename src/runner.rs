//! Parallel execution + per-manager timeouts.
//!
//! One `tokio::spawn` per manager, joined with `futures::future::join_all`.
//! Per-task timeout via `tokio::time::timeout`. Progress reporting flows into
//! `report` through a channel (wired later in Phase 1).
//!
//! ## Detection-then-execute
//!
//! Each task first calls `Manager::detect`. If false → `SkippedMissing`,
//! no `execute` call. This keeps `execute` impls free of detection guards.
//!
//! ## Timeout semantics
//!
//! `tokio::time::timeout` wraps the manager's `execute` future. If it elapses,
//! the wrapping future resolves to `Err(Elapsed)` and we surface that as
//! `ManagerResult::Failed { error: "timed out …" }`. Note: this drops the inner
//! future, which on a subprocess-managing manager will ALSO drop its `Child`
//! handle — `tokio::process::Child` is configured with `kill_on_drop` by
//! convention in our managers, so the OS process dies with the future.

use crate::manager::{Manager, ManagerResult, RunCtx};
use futures::future::join_all;
use std::sync::Arc;
use tokio::time::timeout;

/// Run every manager in parallel, honouring `ctx.timeout` per task.
///
/// Returns one `(name, result)` per input manager, in input order.
pub async fn run_all(
    managers: Vec<Arc<dyn Manager>>,
    ctx: RunCtx,
) -> Vec<(String, ManagerResult)> {
    let tasks = managers.into_iter().map(|m| {
        let ctx = ctx.clone();
        async move {
            let name = m.name().to_string();
            let result = run_one(m.as_ref(), &ctx).await;
            (name, result)
        }
    });
    join_all(tasks).await
}

/// Run a single manager: detect, then execute under a timeout.
async fn run_one(manager: &dyn Manager, ctx: &RunCtx) -> ManagerResult {
    if !manager.detect().await {
        return ManagerResult::SkippedMissing;
    }
    match timeout(ctx.timeout, manager.execute(ctx)).await {
        Ok(result) => result,
        Err(_elapsed) => ManagerResult::Failed {
            error: format!("timed out after {}s", ctx.timeout.as_secs()),
        },
    }
}
