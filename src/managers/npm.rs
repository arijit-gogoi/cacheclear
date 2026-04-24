//! npm: subprocess manager.
//!
//! `npm cache clean --force` — the `--force` flag is REQUIRED by npm itself
//! (npm refuses to clean its cache without it, on the grounds that doing so
//! is "extremely dangerous" per their docs). Per the spec's "use the
//! manager's documented cache command" rule, this is the documented one.

use crate::manager::{Manager, ManagerResult, Plan, RunCtx};
use async_trait::async_trait;
use std::time::Instant;
use tokio::process::Command;

pub struct Npm;

#[async_trait]
impl Manager for Npm {
    fn name(&self) -> &'static str {
        "npm"
    }

    async fn detect(&self) -> bool {
        which::which("npm").is_ok()
    }

    fn plan(&self) -> Plan {
        Plan {
            description: "run `npm cache clean --force`".into(),
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

        // On Windows, `npm` is a `.cmd` shim — `Command::new("npm")` fails
        // unless we pass the resolved full path including the `.cmd` extension.
        // `which::which` follows PATHEXT and returns the right one.
        let bin = match which::which("npm") {
            Ok(p) => p,
            Err(e) => {
                return ManagerResult::Failed {
                    error: format!("could not resolve `npm` on PATH: {e}"),
                };
            }
        };

        let started = Instant::now();
        let mut cmd = Command::new(&bin);
        cmd.args(["cache", "clean", "--force"]);
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
                error: format!("`npm cache clean --force` exited with {status}"),
            },
            Err(e) => ManagerResult::Failed {
                error: format!("failed to spawn `{}`: {e}", bin.display()),
            },
        }
    }
}
