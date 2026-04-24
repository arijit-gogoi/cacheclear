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

fn main() -> Result<()> {
    // Real entry point lands in Phase 1; for now the binary just compiles.
    let _ = cli::Cli::placeholder();
    Ok(())
}
