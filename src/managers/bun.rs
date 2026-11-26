//! bun: subprocess manager. `bun pm --global cache rm` clears `~/.bun/install/cache`.
//!
//! Note: `bun pm cache rm` (without `--global`) operates on the *local*
//! project's cache (i.e. inside the current `node_modules`), not the
//! shared install cache we want to free up. We always want the global one.

use crate::manager::{Manager, ManagerResult, Plan, RunCtx};
use async_trait::async_trait;
use std::path::PathBuf;
use std::time::Instant;
use tokio::process::Command;

pub struct Bun;

/// bun's global install cache lives at `~/.bun/install/cache` by convention.
/// `BUN_INSTALL` overrides the `~/.bun` root if set.
fn cache_dir_path() -> Option<PathBuf> {
    if let Ok(env) = std::env::var("BUN_INSTALL") {
        if !env.is_empty() {
            return Some(PathBuf::from(env).join("install").join("cache"));
        }
    }
    crate::fs_util::home_dir().map(|h| h.join(".bun").join("install").join("cache"))
}

#[async_trait]
impl Manager for Bun {
    fn name(&self) -> &'static str {
        "bun"
    }

    async fn detect(&self) -> bool {
        which::which("bun").is_ok()
    }

    fn plan(&self) -> Plan {
        Plan {
            description: "run `bun pm --global cache rm`".into(),
            paths: vec![],
        }
    }

    async fn cache_dir(&self) -> Option<PathBuf> {
        cache_dir_path()
    }

    async fn execute(&self, ctx: &RunCtx) -> ManagerResult {
        if ctx.dry_run {
            return ManagerResult::Done {
                freed: 0,
                took: std::time::Duration::ZERO,
            };
        }

        // Use which::which so we get the resolved full path (handles
        // `.cmd`/`.exe` shims correctly on Windows).
        let bin = match which::which("bun") {
            Ok(p) => p,
            Err(e) => {
                return ManagerResult::Failed {
                    error: format!("could not resolve `bun` on PATH: {e}"),
                };
            }
        };

        let started = Instant::now();
        let mut cmd = Command::new(&bin);
        cmd.args(["pm", "--global", "cache", "rm"]);
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
                error: format!("`bun pm --global cache rm` exited with {status}"),
            },
            Err(e) => ManagerResult::Failed {
                error: format!("failed to spawn `bun`: {e}"),
            },
        }
    }
}
