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
use crate::report::{format_bytes, format_duration};
use futures::future::join_all;
use indicatif::{MultiProgress, ProgressBar, ProgressDrawTarget, ProgressStyle};
use std::sync::Arc;
use std::time::Duration;
use tokio::time::timeout;

/// Run every manager in parallel, honouring `ctx.timeout` per task.
///
/// Returns one `(name, result)` per input manager, in input order.
///
/// With live_board, draws one status line per manager on stderr while the
/// run is in progress (pending, running, then the outcome), and clears the
/// board before returning so the caller can print the final table. indicatif
/// draws nothing when stderr is not a terminal.
pub async fn run_all(
    managers: Vec<Arc<dyn Manager>>,
    ctx: RunCtx,
    live_board: bool,
) -> Vec<(String, ManagerResult)> {
    let multi = if live_board {
        MultiProgress::new()
    } else {
        MultiProgress::with_draw_target(ProgressDrawTarget::hidden())
    };
    let style = ProgressStyle::with_template("  {spinner} {prefix:<8} {msg}")
        .expect("valid template")
        .tick_chars("⠋⠙⠹⠸⠼⠴⠦⠧⠇⠏ ");
    let finished = ProgressStyle::with_template("  {msg}").expect("valid template");

    let tasks = managers.into_iter().map(|m| {
        let ctx = ctx.clone();
        let finished = finished.clone();
        let bar = multi.add(ProgressBar::new_spinner());
        bar.set_style(style.clone());
        bar.set_prefix(m.name());
        bar.set_message("pending");
        async move {
            bar.enable_steady_tick(Duration::from_millis(100));
            let name = m.name().to_string();
            let result = run_one(m.as_ref(), &ctx, &bar).await;
            bar.set_style(finished);
            bar.finish_with_message(board_outcome(&name, &result));
            (name, result)
        }
    });
    let results = join_all(tasks).await;
    let _ = multi.clear();
    results
}

/// Short outcome text for a finished line on the live board.
fn board_outcome(name: &str, r: &ManagerResult) -> String {
    match r {
        ManagerResult::Done { freed, took } => {
            format!("✓ {name:<8} done     {} ({})", format_bytes(*freed), format_duration(*took))
        }
        ManagerResult::SkippedMissing => format!("⊘ {name:<8} skipped  not installed"),
        ManagerResult::SkippedBusy { reason } => format!("⊘ {name:<8} skipped  busy: {reason}"),
        ManagerResult::Failed { error } => format!("✗ {name:<8} failed   {error}"),
    }
}

/// Run a single manager: detect, probe-before, execute under timeout, probe-after.
async fn run_one(manager: &dyn Manager, ctx: &RunCtx, bar: &ProgressBar) -> ManagerResult {
    bar.set_message("detecting");
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

    bar.set_message("running");
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
