//! uv: subprocess manager. `uv cache clean` is documented cache-only.
//!
//! Detection: PATH lookup for `uv` via the `which` crate (cheaper than
//! spawning a process for `uv --version` just to check existence).
//!
//! This is the reference impl for the four other subprocess managers
//! (pip, npm, bun, pnpm) — pattern: detect via PATH, run the cache
//! subcommand under tokio with `kill_on_drop`, time the wall-clock,
//! map exit status to `ManagerResult`.

use crate::manager::{Manager, ManagerResult, Plan, RunCtx};
use async_trait::async_trait;
use std::time::Instant;
use tokio::process::Command;

pub struct Uv;

#[async_trait]
impl Manager for Uv {
    fn name(&self) -> &'static str {
        "uv"
    }

    async fn detect(&self) -> bool {
        which::which("uv").is_ok()
    }

    fn plan(&self) -> Plan {
        Plan {
            description: "run `uv cache clean`".into(),
            paths: vec![],
        }
    }

    async fn execute(&self, ctx: &RunCtx) -> ManagerResult {
        if ctx.dry_run {
            return ManagerResult::Done {
                freed: 0,
                took: std::time::Duration::ZERO,
            };
        }

        let started = Instant::now();
        let mut cmd = Command::new("uv");
        cmd.args(["cache", "clean"]);
        cmd.kill_on_drop(true);
        if !ctx.verbose {
            cmd.stdout(std::process::Stdio::null());
            cmd.stderr(std::process::Stdio::null());
        }

        match cmd.status().await {
            Ok(status) if status.success() => ManagerResult::Done {
                // Bytes-freed wiring lands when fs_util::dir_size is real and
                // we can probe `uv cache dir` before/after. For now: 0.
                freed: 0,
                took: started.elapsed(),
            },
            Ok(status) => ManagerResult::Failed {
                error: format!("`uv cache clean` exited with {status}"),
            },
            Err(e) => ManagerResult::Failed {
                error: format!("failed to spawn `uv`: {e}"),
            },
        }
    }
}
