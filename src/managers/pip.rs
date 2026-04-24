//! pip: subprocess manager. `pip cache purge` is documented cache-only.
//!
//! Detection: PATH lookup for `pip`, falling back to `pip3` (per spec).
//! On Linux/macOS, `pip` is often unavailable while `pip3` is; on Windows
//! both usually exist. Whichever is found first is also what we invoke.

use crate::manager::{Manager, ManagerResult, Plan, RunCtx};
use async_trait::async_trait;
use std::path::PathBuf;
use std::time::Instant;
use tokio::process::Command;

pub struct Pip;

/// Discover pip's cache dir by shelling `pip cache dir` (or pip3 fallback).
async fn discover_cache_dir() -> Option<PathBuf> {
    let name = pip_binary()?;
    let bin = which::which(name).ok()?;
    let output = Command::new(&bin)
        .args(["cache", "dir"])
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

/// Return the first of `pip` / `pip3` found on PATH, or `None`.
fn pip_binary() -> Option<&'static str> {
    if which::which("pip").is_ok() {
        Some("pip")
    } else if which::which("pip3").is_ok() {
        Some("pip3")
    } else {
        None
    }
}

#[async_trait]
impl Manager for Pip {
    fn name(&self) -> &'static str {
        "pip"
    }

    async fn detect(&self) -> bool {
        pip_binary().is_some()
    }

    fn plan(&self) -> Plan {
        Plan {
            description: "run `pip cache purge`".into(),
            paths: vec![],
        }
    }

    async fn cache_dir(&self) -> Option<PathBuf> {
        discover_cache_dir().await
    }

    async fn execute(&self, ctx: &RunCtx) -> ManagerResult {
        if ctx.dry_run {
            return ManagerResult::Done {
                freed: 0,
                took: std::time::Duration::ZERO,
            };
        }

        let Some(name) = pip_binary() else {
            return ManagerResult::SkippedMissing;
        };
        // Resolve the full path so Windows `.cmd` shims work.
        let bin = match which::which(name) {
            Ok(p) => p,
            Err(e) => {
                return ManagerResult::Failed {
                    error: format!("could not resolve `{name}` on PATH: {e}"),
                };
            }
        };

        let started = Instant::now();
        let mut cmd = Command::new(&bin);
        cmd.args(["cache", "purge"]);
        cmd.kill_on_drop(true);
        if !ctx.verbose {
            cmd.stdout(std::process::Stdio::null());
            cmd.stderr(std::process::Stdio::null());
        }

        match cmd.status().await {
            Ok(status) if status.success() => ManagerResult::Done {
                freed: 0,
                took: started.elapsed(),
            },
            Ok(status) => ManagerResult::Failed {
                error: format!("`{} cache purge` exited with {status}", bin.display()),
            },
            Err(e) => ManagerResult::Failed {
                error: format!("failed to spawn `{}`: {e}", bin.display()),
            },
        }
    }
}
