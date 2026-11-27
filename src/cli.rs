//! Command-line interface — `clap` derive definitions.
//!
//! Surface (v1, from `spec/SPEC.md`):
//! ```text
//! cacheclear [--only LIST] [--skip LIST] [--dry-run] [--json]
//!            [-v|--verbose] [--timeout SECS] [--keep-registry]
//!            [--no-color] [-h|--help] [-V|--version]
//! ```

use clap::{Parser, ValueEnum};

/// Manager names accepted by `--only` / `--skip`.
///
/// Must match `Manager::name()` of each manager. Clap rejects any other
/// name with a usage error (exit 2).
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum ManagerName {
    Uv,
    Pip,
    Pipx,
    Cargo,
    Npm,
    Pnpm,
    Bun,
}

impl ManagerName {
    /// The name as `Manager::name()` reports it.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Uv => "uv",
            Self::Pip => "pip",
            Self::Pipx => "pipx",
            Self::Cargo => "cargo",
            Self::Npm => "npm",
            Self::Pnpm => "pnpm",
            Self::Bun => "bun",
        }
    }
}

/// Clear download/build caches of common package managers in parallel.
#[derive(Debug, Parser)]
#[command(name = "cacheclear", version, about, long_about = None)]
pub struct Cli {
    /// Comma-separated managers to include (default: all detected).
    #[arg(long, value_name = "LIST", value_delimiter = ',')]
    pub only: Option<Vec<ManagerName>>,

    /// Comma-separated managers to exclude. Applied after `--only`.
    #[arg(long, value_name = "LIST", value_delimiter = ',')]
    pub skip: Vec<ManagerName>,

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

    /// cargo only: keep registry/cache and registry/src (the downloaded crates),
    /// so the next build doesn't re-download them. git/db and git/checkouts
    /// are still cleared.
    #[arg(long)]
    pub keep_registry: bool,

    /// Disable color output. Output is currently uncolored, so this is a no-op;
    /// it is accepted so scripts can pass it safely.
    #[arg(long)]
    pub no_color: bool,
}

