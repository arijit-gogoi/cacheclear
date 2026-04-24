//! Parallel execution + per-manager timeouts + size-probe wrapping.
//!
//! One `tokio::spawn` per manager, joined with `futures::future::join_all`.
//! Per-task timeout via `tokio::time::timeout`. Progress reporting flows
//! into `report` through the returned `(name, ManagerResult)` pairs.
//!
//! ## Detection-then-execute
//!
//! Each task first calls `Manager::detect`. If false → `SkippedMissing`,
//! no `execute` call. This keeps `execute` impls free of detection guards.
//!
//! ## Size probe (subprocess managers)
//!
//! Direct-deletion managers (cargo, pipx) compute `freed` themselves — they
//! return `Manager::cache_dir() == None` and we trust their `freed` value.
//!
//! Subprocess managers (uv, pip, npm, pnpm, bun) don't expose freed-bytes
//! machine-readably — they return `freed: 0` from `execute`. They DO
//! implement `cache_dir()`. The runner generically probes the dir before
//! and after, replacing `freed: 0` with the real delta.
//!
//! ## Timeout semantics
//!
//! `tokio::time::timeout` wraps the manager's `execute` future (NOT the
//! pre-probe; that's expected to be fast). On elapse, the inner future
//! is dropped — `tokio::process::Child` with `kill_on_drop(true)` ensures
//! the OS process dies with the future.

use crate::fs_util::dir_size;
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

/// Run a single manager: detect, probe-before, execute under timeout, probe-after.
async fn run_one(manager: &dyn Manager, ctx: &RunCtx) -> ManagerResult {
    if !manager.detect().await {
        return ManagerResult::SkippedMissing;
    }

    // Probe before execute, ONLY if the manager exposes a cache_dir AND
    // we're not in dry-run mode (dry-run never modifies, so the delta
    // would always be 0; not worth the I/O).
    let probed_cache = if !ctx.dry_run {
        manager.cache_dir().await
    } else {
        None
    };
    let before = probed_cache
        .as_ref()
        .and_then(|p| dir_size(p).ok());

    let result = match timeout(ctx.timeout, manager.execute(ctx)).await {
        Ok(result) => result,
        Err(_elapsed) => {
            return ManagerResult::Failed {
                error: format!("timed out after {}s", ctx.timeout.as_secs()),
            };
        }
    };

    // If we have a before-size and the manager reported freed: 0 (the
    // subprocess-manager convention), substitute the real delta.
    match (result, before, probed_cache.as_ref()) {
        (ManagerResult::Done { freed: 0, took }, Some(before_bytes), Some(path)) => {
            let after_bytes = dir_size(path).unwrap_or(before_bytes);
            ManagerResult::Done {
                freed: before_bytes.saturating_sub(after_bytes),
                took,
            }
        }
        (other, _, _) => other,
    }
}
