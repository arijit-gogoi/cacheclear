//! Command-line interface — `clap` derive definitions.
//!
//! Surface (v1, from `spec/SPEC.md`):
//! ```text
//! cacheclear [--only LIST] [--skip LIST] [--dry-run] [--json]
//!            [-v|--verbose] [--timeout SECS] [--no-color]
//!            [-h|--help] [-V|--version]
//! ```

use clap::Parser;

/// Clear download/build caches of common package managers in parallel.
#[derive(Debug, Parser)]
#[command(name = "cacheclear", version, about, long_about = None)]
pub struct Cli {
    /// Comma-separated managers to include (default: all detected).
    #[arg(long, value_name = "LIST")]
    pub only: Option<String>,

    /// Comma-separated managers to exclude.
    #[arg(long, value_name = "LIST")]
    pub skip: Option<String>,

    /// Print what would be done; touch nothing.
    #[arg(long)]
    pub dry_run: bool,

    /// Emit machine-readable result to stdout.
    #[arg(long)]
    pub json: bool,

    /// Stream child process output.
    #[arg(short, long)]
    pub verbose: bool,

    /// Kill any single manager that runs longer than this (seconds).
    #[arg(long, value_name = "SECS", default_value_t = 120)]
    pub timeout: u64,

    /// Disable color output.
    #[arg(long)]
    pub no_color: bool,
}

impl Cli {
    /// Placeholder so the scaffold compiles before we wire the real entry point.
    pub fn placeholder() -> Self {
        Self {
            only: None,
            skip: None,
            dry_run: false,
            json: false,
            verbose: false,
            timeout: 120,
            no_color: false,
        }
    }
}
