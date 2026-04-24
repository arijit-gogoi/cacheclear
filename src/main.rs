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
use std::sync::Arc;
use std::time::Duration;

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

    // Phase-1 wiring: only `uv` is implemented end-to-end. The other six
    // managers still return `SkippedMissing` from their stubs, which the
    // runner treats correctly (we don't even call `execute` on them).
    let managers: Vec<Arc<dyn Manager>> = vec![
        Arc::new(managers::uv::Uv),
        Arc::new(managers::pip::Pip),
        Arc::new(managers::pipx::Pipx),
        Arc::new(managers::cargo::Cargo),
        Arc::new(managers::npm::Npm),
        Arc::new(managers::pnpm::Pnpm),
        Arc::new(managers::bun::Bun),
    ];

    let results = runner::run_all(managers, ctx).await;

    // Provisional textual output until `report` is wired up.
    let mut any_failed = false;
    for (name, result) in &results {
        match result {
            ManagerResult::Done { freed, took } => {
                println!("  ✓ {name:<8} done       {freed:>10} bytes  ({:.1?})", took);
            }
            ManagerResult::SkippedMissing => {
                println!("  ⊘ {name:<8} skipped    not installed");
            }
            ManagerResult::SkippedBusy { reason } => {
                println!("  ⊘ {name:<8} skipped    busy: {reason}");
            }
            ManagerResult::Failed { error } => {
                any_failed = true;
                println!("  ✗ {name:<8} failed     {error}");
            }
        }
    }

    if any_failed {
        std::process::exit(1);
    }
    Ok(())
}
