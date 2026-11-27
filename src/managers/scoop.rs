//! scoop (Windows only): subprocess manager.
//!
//! `scoop cache rm -a` (same as `scoop cache rm *`) deletes everything in
//! scoop's download cache: `<root>\cache`, or `SCOOP_CACHE` / config
//! `cache_path`. It needs no elevation.
//!
//! **Never `scoop cleanup`.** It deletes old *installed* versions
//! (`apps\<app>\<old version>`), which `scoop reset <app>@<version>` can
//! switch back to. That is uninstalling, which breaks the core invariant.
//! Its `-k` flag only adds removal of outdated downloads, which
//! `cache rm -a` already covers. (Checked against scoop 0.6.0's
//! `libexec/scoop-cleanup.ps1` and `libexec/scoop-cache.ps1`.)
//!
//! `scoop-cache.ps1` exits 0 even when a file can't be removed. The runner's
//! before/after size probe of `cache_dir()` still reports what was freed.

use crate::manager::{Manager, ManagerResult, Plan, RunCtx};
use async_trait::async_trait;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::{Duration, Instant};
use tokio::process::Command;

pub struct Scoop;

/// The `scoop` shim on PATH. Scoop exists only on Windows, so on other
/// platforms a `scoop` on PATH is something else and is ignored.
fn find_scoop() -> Option<PathBuf> {
    if cfg!(windows) {
        which::which("scoop").ok()
    } else {
        None
    }
}

/// The scoop root a shim lives in: `<root>\shims\scoop.cmd` gives `<root>`.
fn install_root(shim: &Path) -> Option<PathBuf> {
    let shims = shim.parent()?;
    if shims.file_name()?.eq_ignore_ascii_case("shims") {
        shims.parent().map(Path::to_path_buf)
    } else {
        None
    }
}

/// Scoop's config file, located the way scoop's `lib/core.ps1` does it:
/// `<root>\config.json` for a portable install, else
/// `$XDG_CONFIG_HOME\scoop\config.json`, else `~\.config\scoop\config.json`.
fn config_path(
    env: impl Fn(&str) -> Option<String>,
    install_root: Option<&Path>,
    home: Option<&Path>,
) -> Option<PathBuf> {
    if let Some(root) = install_root {
        let portable = root.join("config.json");
        if portable.is_file() {
            return Some(portable);
        }
    }
    let base = env("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| home.map(|h| h.join(".config")))?;
    Some(base.join("scoop").join("config.json"))
}

/// Parse scoop's config file. Scoop writes it without a BOM but reads it
/// with `ReadAllLines`, which accepts one, so a leading BOM is skipped.
fn read_config(path: &Path) -> Option<serde_json::Value> {
    let text = std::fs::read_to_string(path).ok()?;
    serde_json::from_str(text.trim_start_matches('\u{feff}')).ok()
}

/// Scoop's download cache dir, resolved in the order of scoop's
/// `lib/core.ps1`: `$SCOOP_CACHE`, then config `cache_path`, then
/// `<root>\cache`. The root is `$SCOOP`, then config `root_path`, then the
/// root scoop is installed in, then `~\scoop`.
///
/// `env` must return `None` for unset or empty variables; scoop skips both.
fn resolve_cache_dir(
    env: impl Fn(&str) -> Option<String>,
    config: Option<&serde_json::Value>,
    install_root: Option<&Path>,
    home: Option<&Path>,
) -> Option<PathBuf> {
    let from_config = |key: &str| {
        config?
            .get(key)?
            .as_str()
            .filter(|s| !s.is_empty())
            .map(PathBuf::from)
    };
    if let Some(cache) = env("SCOOP_CACHE")
        .map(PathBuf::from)
        .or_else(|| from_config("cache_path"))
    {
        return Some(cache);
    }
    let root = env("SCOOP")
        .map(PathBuf::from)
        .or_else(|| from_config("root_path"))
        .or_else(|| install_root.map(Path::to_path_buf))
        .or_else(|| home.map(|h| h.join("scoop")))?;
    Some(root.join("cache"))
}

#[async_trait]
impl Manager for Scoop {
    fn name(&self) -> &'static str {
        "scoop"
    }

    async fn detect(&self) -> bool {
        find_scoop().is_some()
    }

    fn plan(&self) -> Plan {
        Plan {
            description: "run `scoop cache rm -a`".into(),
            paths: vec![],
        }
    }

    async fn cache_dir(&self) -> Option<PathBuf> {
        let env = |key: &str| std::env::var(key).ok().filter(|v| !v.is_empty());
        let home = crate::fs_util::home_dir();
        let root = find_scoop().as_deref().and_then(install_root);
        let config = config_path(env, root.as_deref(), home.as_deref())
            .and_then(|p| read_config(&p));
        resolve_cache_dir(env, config.as_ref(), root.as_deref(), home.as_deref())
    }

    async fn execute(&self, ctx: &RunCtx) -> ManagerResult {
        if ctx.dry_run {
            return ManagerResult::Done {
                freed: 0,
                took: Duration::ZERO,
            };
        }

        // `scoop` is a `.cmd` shim; `which` resolves the extension via PATHEXT.
        let Some(bin) = find_scoop() else {
            return ManagerResult::Failed {
                error: "could not resolve `scoop` on PATH".into(),
            };
        };

        let started = Instant::now();
        let mut cmd = Command::new(&bin);
        cmd.args(["cache", "rm", "-a"]);
        cmd.kill_on_drop(true);
        // No stdin. A non-empty subdirectory in the cache makes `Remove-Item`
        // ask for confirmation; without stdin that prompt fails at once
        // instead of waiting, unseen, until --timeout.
        cmd.stdin(Stdio::null());
        if !ctx.verbose {
            cmd.stdout(Stdio::null());
            cmd.stderr(Stdio::null());
        }

        match cmd.status().await {
            Ok(status) if status.success() => ManagerResult::Done {
                freed: 0,
                took: started.elapsed(),
            },
            Ok(status) => ManagerResult::Failed {
                error: format!("`scoop cache rm -a` exited with {status}"),
            },
            Err(e) => ManagerResult::Failed {
                error: format!("failed to spawn `{}`: {e}", bin.display()),
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// An environment holding only `vars`.
    fn env_of<'a>(vars: &'a [(&'a str, &'a str)]) -> impl Fn(&str) -> Option<String> + 'a {
        move |key| {
            vars.iter()
                .find(|(k, _)| *k == key)
                .map(|(_, v)| v.to_string())
        }
    }

    #[test]
    fn cache_dir_follows_scoop_precedence() {
        let home = Path::new("home");
        let installed = Path::new("installed");
        let config = json!({ "cache_path": "cfg-cache", "root_path": "cfg-root" });
        let root_only = json!({ "root_path": "cfg-root" });
        let resolve = |vars: &[(&str, &str)], config: Option<&serde_json::Value>| {
            resolve_cache_dir(env_of(vars), config, Some(installed), Some(home))
        };

        let all_env = [("SCOOP_CACHE", "env-cache"), ("SCOOP", "env-root")];
        assert_eq!(resolve(&all_env, Some(&config)), Some("env-cache".into()));
        let root_env = [("SCOOP", "env-root")];
        assert_eq!(resolve(&root_env, Some(&config)), Some("cfg-cache".into()));
        assert_eq!(
            resolve(&root_env, Some(&root_only)),
            Some(Path::new("env-root").join("cache"))
        );
        assert_eq!(
            resolve(&[], Some(&root_only)),
            Some(Path::new("cfg-root").join("cache"))
        );
        assert_eq!(resolve(&[], None), Some(installed.join("cache")));
        assert_eq!(
            resolve_cache_dir(env_of(&[]), None, None, Some(home)),
            Some(home.join("scoop").join("cache"))
        );
        assert_eq!(resolve_cache_dir(env_of(&[]), None, None, None), None);
    }

    #[test]
    fn empty_config_values_are_skipped() {
        let config = json!({ "cache_path": "", "root_path": "" });
        assert_eq!(
            resolve_cache_dir(env_of(&[]), Some(&config), None, Some(Path::new("home"))),
            Some(Path::new("home").join("scoop").join("cache"))
        );
    }

    #[test]
    fn install_root_needs_a_shims_dir() {
        let root = Path::new("root");
        assert_eq!(
            install_root(&root.join("shims").join("scoop.cmd")),
            Some(root.to_path_buf())
        );
        assert_eq!(
            install_root(&root.join("Shims").join("scoop.cmd")),
            Some(root.to_path_buf())
        );
        assert_eq!(install_root(&root.join("bin").join("scoop.cmd")), None);
    }

    #[test]
    fn config_file_prefers_portable_then_xdg_then_home() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let home = Path::new("home");
        let user_config = home.join(".config").join("scoop").join("config.json");
        assert_eq!(
            config_path(env_of(&[]), Some(root), Some(home)),
            Some(user_config)
        );
        assert_eq!(
            config_path(env_of(&[("XDG_CONFIG_HOME", "xdg")]), Some(root), Some(home)),
            Some(Path::new("xdg").join("scoop").join("config.json"))
        );
        std::fs::write(root.join("config.json"), "{}").unwrap();
        assert_eq!(
            config_path(env_of(&[("XDG_CONFIG_HOME", "xdg")]), Some(root), Some(home)),
            Some(root.join("config.json"))
        );
    }

    #[test]
    fn config_with_bom_is_read() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.json");
        std::fs::write(&path, "\u{feff}{\"cache_path\": \"D:\\\\scoop-cache\"}").unwrap();
        let config = read_config(&path).expect("config parses");
        assert_eq!(config["cache_path"], "D:\\scoop-cache");
    }
}
