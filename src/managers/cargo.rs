//! cargo: direct-deletion manager — cargo has no cache command.
//! Under `$CARGO_HOME` (default `~/.cargo`) we delete contents of:
//!   registry/cache/*  registry/src/*  git/db/*  git/checkouts/*
//! **Never touch `bin/`** (installed binaries) or **`registry/index/`** (metadata).
//!
//! Concurrency: try a non-blocking exclusive flock on `$CARGO_HOME/.package-cache`
//! before deleting; if held, return `SkippedBusy`.

use crate::fs_util::{dir_size, rm_contents_cancellable, try_lock_exclusive_nonblocking};
use crate::manager::{Manager, ManagerResult, Plan, RunCtx};
use async_trait::async_trait;
use std::path::PathBuf;
use std::time::Instant;

pub struct Cargo;

/// Resolve `$CARGO_HOME` honoring the env var; fall back to `~/.cargo`.
fn cargo_home() -> Option<PathBuf> {
    if let Ok(env) = std::env::var("CARGO_HOME") {
        if !env.is_empty() {
            return Some(PathBuf::from(env));
        }
    }
    crate::fs_util::home_dir().map(|h| h.join(".cargo"))
}

/// The four documented-safe-to-delete dirs (relative to `$CARGO_HOME`).
const SAFE_DIRS: &[&str] = &[
    "registry/cache",
    "registry/src",
    "git/db",
    "git/checkouts",
];

#[async_trait]
impl Manager for Cargo {
    fn name(&self) -> &'static str {
        "cargo"
    }

    async fn detect(&self) -> bool {
        // Cargo is "installed" for our purposes if either the binary is on
        // PATH or $CARGO_HOME exists with at least one of the safe dirs.
        // The binary check covers the common case; the dir check covers
        // unusual setups (custom CARGO_HOME pointed at a populated cache,
        // even if `cargo` itself isn't on this PATH).
        if which::which("cargo").is_ok() {
            return true;
        }
        if let Some(home) = cargo_home() {
            return SAFE_DIRS.iter().any(|d| home.join(d).exists());
        }
        false
    }

    fn plan(&self) -> Plan {
        let paths = cargo_home()
            .map(|h| SAFE_DIRS.iter().map(|d| h.join(d)).collect::<Vec<_>>())
            .unwrap_or_default();
        Plan {
            description: "delete contents of cargo registry/cache, registry/src, git/db, git/checkouts".into(),
            paths,
        }
    }

    async fn execute(&self, ctx: &RunCtx) -> ManagerResult {
        let Some(home) = cargo_home() else {
            return ManagerResult::Failed {
                error: "could not resolve $CARGO_HOME or ~/.cargo".into(),
            };
        };

        // Spec §"Concurrent-Install Handling": non-blocking exclusive flock
        // on $CARGO_HOME/.package-cache before deleting. If held by another
        // process (active `cargo build`), Skipped (busy) — don't wait, don't retry.
        let lock_path = home.join(".package-cache");
        let _lock_guard = match try_lock_exclusive_nonblocking(&lock_path) {
            Ok(guard) => guard,
            Err(e) => {
                return ManagerResult::Failed {
                    error: format!("flock check on {} failed: {e}", lock_path.display()),
                };
            }
        };
        // Note: if .package-cache doesn't exist, `try_lock_…` returns Ok(None)
        // and we proceed — there's nothing to race against.
        // If we got Some(file), we hold the exclusive flock for the duration
        // of this function (drop releases it). If Ok(None) AND the file did
        // exist, that means another process has it — surface as SkippedBusy.
        if lock_path.exists() && _lock_guard.is_none() {
            return ManagerResult::SkippedBusy {
                reason: "another cargo process is using the cache".into(),
            };
        }

        let started = Instant::now();
        let mut total_freed: u64 = 0;

        for sub in SAFE_DIRS {
            let path = home.join(sub);
            if !path.exists() {
                continue;
            }
            if ctx.dry_run {
                // For dry-run, count what *would* be freed but don't delete.
                if let Ok(sz) = dir_size(&path) {
                    total_freed = total_freed.saturating_add(sz);
                }
                continue;
            }
            match rm_contents_cancellable(path.clone()).await {
                Ok(freed) => {
                    total_freed = total_freed.saturating_add(freed);
                }
                Err(e) => {
                    return ManagerResult::Failed {
                        error: format!("clearing {}: {e}", path.display()),
                    };
                }
            }
        }

        ManagerResult::Done {
            freed: total_freed,
            took: started.elapsed(),
        }
    }
}
