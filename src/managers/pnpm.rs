//! pnpm: subprocess manager. `pnpm store prune` removes unreferenced packages
//! from the content-addressable store. **Never wipe the store dir wholesale** —
//! every pnpm project on the machine symlinks into it.
//!
//! Concurrency: try a non-blocking lock on the store's lock file
//! (discover via `pnpm store path`); if held, return `SkippedBusy`.

use crate::fs_util::try_lock_exclusive_nonblocking;
use crate::manager::{Manager, ManagerResult, Plan, RunCtx};
use async_trait::async_trait;
use std::path::PathBuf;
use std::time::Instant;
use tokio::process::Command;

pub struct Pnpm;

/// Discover the pnpm store path by shelling out to `pnpm store path`.
/// Returns `None` if pnpm isn't on PATH or the call fails.
async fn discover_store_path() -> Option<PathBuf> {
    let bin = which::which("pnpm").ok()?;
    let output = Command::new(&bin)
        .args(["store", "path"])
        .kill_on_drop(true)
        .output()
        .await
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let raw = String::from_utf8(output.stdout).ok()?;
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(PathBuf::from(trimmed))
    }
}

/// Where the pnpm store keeps its lock. Modern pnpm uses `<store>/v3/.lock`;
/// older versions used `<store>/.lock`. We try both.
fn candidate_lock_files(store: &std::path::Path) -> Vec<PathBuf> {
    vec![store.join("v3").join(".lock"), store.join(".lock")]
}

#[async_trait]
impl Manager for Pnpm {
    fn name(&self) -> &'static str {
        "pnpm"
    }

    async fn detect(&self) -> bool {
        which::which("pnpm").is_ok()
    }

    fn plan(&self) -> Plan {
        Plan {
            description: "run `pnpm store prune`".into(),
            paths: vec![],
        }
    }

    async fn cache_dir(&self) -> Option<PathBuf> {
        discover_store_path().await
    }

    async fn execute(&self, ctx: &RunCtx) -> ManagerResult {
        if ctx.dry_run {
            return ManagerResult::Done {
                freed: 0,
                took: std::time::Duration::ZERO,
            };
        }

        // Resolve full path for Windows .cmd shim compatibility.
        let bin = match which::which("pnpm") {
            Ok(p) => p,
            Err(e) => {
                return ManagerResult::Failed {
                    error: format!("could not resolve `pnpm` on PATH: {e}"),
                };
            }
        };

        // Discover the store and check its lock. If discovery fails or no
        // lock file exists at the expected paths, we proceed without the
        // flock — `pnpm store prune` is itself blocking-aware, so the worst
        // case is the prune waits a moment instead of skipping. Per the
        // spec we PREFER to skip-if-busy for fast UX, so we attempt the
        // lock check first and bail with SkippedBusy if any candidate is held.
        let mut held_lock_guard = None;
        if let Some(store) = discover_store_path().await {
            for lock_path in candidate_lock_files(&store) {
                match try_lock_exclusive_nonblocking(&lock_path) {
                    Ok(Some(file)) => {
                        held_lock_guard = Some(file);
                        // Hold THIS lock for the duration; don't bother
                        // checking other candidates (one is enough).
                        break;
                    }
                    Ok(None) if lock_path.exists() => {
                        return ManagerResult::SkippedBusy {
                            reason: "another pnpm process is using the store".into(),
                        };
                    }
                    Ok(None) => {} // lock file doesn't exist, try next candidate
                    Err(e) => {
                        return ManagerResult::Failed {
                            error: format!("flock check on {} failed: {e}", lock_path.display()),
                        };
                    }
                }
            }
        }
        // Suppress unused-variable warning for the guard — we only care
        // about its lifetime, not its identity.
        let _ = held_lock_guard;

        let started = Instant::now();
        let mut cmd = Command::new(&bin);
        cmd.args(["store", "prune"]);
        cmd.kill_on_drop(true);
        if !ctx.verbose {
            cmd.stdout(std::process::Stdio::null());
            cmd.stderr(std::process::Stdio::null());
        }

        match cmd.status().await {
            Ok(status) if status.success() => ManagerResult::Done {
                freed: 0, // pnpm prints stats but doesn't expose freed-bytes machine-readably
                took: started.elapsed(),
            },
            Ok(status) => ManagerResult::Failed {
                error: format!("`pnpm store prune` exited with {status}"),
            },
            Err(e) => ManagerResult::Failed {
                error: format!("failed to spawn `{}`: {e}", bin.display()),
            },
        }
    }
}
