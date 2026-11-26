//! `cacheclear` — clear download/build caches of common package managers in parallel,
//! without ever uninstalling anything.
//!
//! See `spec/SPEC.md` for the full design, especially the safety invariants:
//! - never uninstall installed packages or binaries
//! - never follow symlinks below a cache root
//! - always `\\?\`-prefix cache roots on Windows

mod cli;
mod fs_util;
mod manager;
mod managers;
mod report;
mod runner;

use anyhow::Result;
use clap::Parser;
use futures::future::join_all;
use std::sync::Arc;
use std::time::Duration;

use crate::fs_util::dir_size;
use crate::manager::{Manager, ManagerResult, RunCtx};

#[tokio::main(flavor = "multi_thread")]
async fn main() -> Result<()> {
    let cli = cli::Cli::parse();

    let ctx = RunCtx {
        verbose: cli.verbose,
        dry_run: cli.dry_run,
        timeout: Duration::from_secs(cli.timeout),
        no_color: cli.no_color,
    };

    let all: Vec<Arc<dyn Manager>> = vec![
        Arc::new(managers::uv::Uv),
        Arc::new(managers::pip::Pip),
        Arc::new(managers::pipx::Pipx),
        Arc::new(managers::cargo::Cargo),
        Arc::new(managers::npm::Npm),
        Arc::new(managers::pnpm::Pnpm),
        Arc::new(managers::bun::Bun),
    ];
    // Filter once, before the dry-run / real-run split, so both paths see
    // the same set. Filtered-out managers are dropped from the report.
    let managers = select_managers(all, cli.only.as_deref(), &cli.skip);

    if ctx.dry_run {
        // Build (name, detected, plan, would_freed) for the dry-run emitter.
        // detect() and cache_dir() probes happen in parallel per manager.
        let entries: Vec<(String, bool, crate::manager::Plan, Option<u64>)> =
            join_all(managers.iter().cloned().map(|m| async move {
                let name = m.name().to_string();
                let detected = m.detect().await;
                let plan = m.plan();
                // Best-effort size estimate: prefer the plan's concrete paths
                // (direct-deletion managers), fall back to cache_dir() probe
                // (subprocess managers).
                let would_free = if !detected {
                    None
                } else if !plan.paths.is_empty() {
                    let mut total: u64 = 0;
                    for p in &plan.paths {
                        if let Ok(sz) = dir_size(p) {
                            total = total.saturating_add(sz);
                        }
                    }
                    Some(total)
                } else if let Some(cd) = m.cache_dir().await {
                    dir_size(&cd).ok()
                } else {
                    None
                };
                (name, detected, plan, would_free)
            }))
            .await;
        if cli.json {
            report::emit_json_dry_run(&entries);
        } else {
            report::emit_dry_run_plan(&entries);
        }
        return Ok(());
    }

    // Live board only for humans: --json wants clean output and -v's
    // child output would interleave with the redrawn lines.
    let live_board = !cli.json && !cli.verbose;
    let results = runner::run_all(managers, ctx, live_board).await;
    if cli.json {
        report::emit_json_run(&results);
    } else {
        report::emit_run_report(&results);
    }

    let any_failed = results
        .iter()
        .any(|(_, r)| matches!(r, ManagerResult::Failed { .. }));
    if any_failed {
        std::process::exit(1);
    }
    Ok(())
}

/// Apply `--only` (keep listed managers), then `--skip` (drop listed ones).
/// Order of the returned managers follows the built-in order.
fn select_managers(
    all: Vec<Arc<dyn Manager>>,
    only: Option<&[cli::ManagerName]>,
    skip: &[cli::ManagerName],
) -> Vec<Arc<dyn Manager>> {
    let listed = |list: &[cli::ManagerName], name: &str| list.iter().any(|n| n.as_str() == name);
    all.into_iter()
        .filter(|m| only.is_none_or(|o| listed(o, m.name())))
        .filter(|m| !listed(skip, m.name()))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use cli::ManagerName;
    use clap::ValueEnum;

    fn all() -> Vec<Arc<dyn Manager>> {
        vec![
            Arc::new(managers::uv::Uv),
            Arc::new(managers::pip::Pip),
            Arc::new(managers::pipx::Pipx),
            Arc::new(managers::cargo::Cargo),
            Arc::new(managers::npm::Npm),
            Arc::new(managers::pnpm::Pnpm),
            Arc::new(managers::bun::Bun),
        ]
    }

    fn names(ms: &[Arc<dyn Manager>]) -> Vec<&'static str> {
        ms.iter().map(|m| m.name()).collect()
    }

    #[test]
    fn manager_names_match_cli_enum() {
        let mut cli_names: Vec<&str> = ManagerName::value_variants()
            .iter()
            .map(|n| n.as_str())
            .collect();
        let mut mgr_names = names(&all());
        cli_names.sort_unstable();
        mgr_names.sort_unstable();
        assert_eq!(cli_names, mgr_names);
        for n in ManagerName::value_variants() {
            assert_eq!(ManagerName::from_str(n.as_str(), false), Ok(*n));
        }
    }

    #[test]
    fn select_only_then_skip() {
        use ManagerName::*;
        assert_eq!(names(&select_managers(all(), None, &[])).len(), 7);
        assert_eq!(
            names(&select_managers(all(), Some(&[Npm, Cargo]), &[])),
            ["cargo", "npm"]
        );
        assert_eq!(
            names(&select_managers(all(), None, &[Npm, Pnpm, Bun])),
            ["uv", "pip", "pipx", "cargo"]
        );
        assert_eq!(
            names(&select_managers(all(), Some(&[Npm, Cargo]), &[Npm])),
            ["cargo"]
        );
    }
}
