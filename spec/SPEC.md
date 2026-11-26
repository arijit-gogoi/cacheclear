# `cacheclear` — SPEC
_Last updated: 2026-11-26_

A Rust CLI that clears the download/build caches of common package managers in parallel, **without ever uninstalling anything**.

---

## 🎯 Goals & Vision

A single command that reclaims disk space from the accumulated caches of every language/package manager you have installed, run safely in parallel with live progress output. The user runs `cacheclear`, sees a per-manager status board, and ends up with no installed packages touched — only their caches emptied. Target user: developers with polyglot machines (Rust + Node + Python + newer runtimes) who notice tens of GB bleeding into `~/.cache`, `~/.cargo`, `~/.npm`, etc.

---

## 🚫 Constraints, Non-Goals & Anti-Features

| Type | Item | Reason |
|---|---|---|
| **Anti-feature** | **Must NEVER uninstall or remove installed packages/binaries** | Core safety invariant. The tool only ever touches caches. |
| Anti-feature | Must not touch `$CARGO_HOME/bin`, `pipx venvs/`, npm/pnpm/bun global install dirs, uv's tool installs | These contain installed software, not cache. |
| Anti-feature | Must not delete cargo's `registry/index/` by default | It's metadata, not download cache; deleting forces a full re-index on next build. |
| Anti-feature | Run as sudo / modify system-level package caches (apt, brew, pacman, dnf) | Out of scope. Those have security implications and different semantics. |
| Non-goal (v1) | GUI, TUI menu, interactive selection | CLI-first; keep MVP scope tight. |
| Non-goal (v1) | Scheduled/cron-mode, daemon | User-invoked only. |
| Non-goal (v1) | Config file | All behavior expressible through flags in v1. |
| Constraint | Cross-platform (Linux, macOS, Windows) | These are where all seven package managers live. |
| Constraint | Zero-sudo, zero-elevation | Tool operates entirely on user-owned caches. |
| Constraint | Per-manager guardrail: only use that manager's documented cache command (or documented cache directory) | No clever tricks. If the manager says "this is the cache dir", we trust it. |

---

## 📋 Feature Roadmap

| # | Feature | Status | Priority | Notes |
|---|---|---|---|---|
| 1 | Parallel cache clear for 7 managers (uv, pip, pipx, cargo, npm, pnpm, bun) | 📋 Planned | High | Core MVP |
| 2 | Auto-detect which managers are installed (skip missing) | 📋 Planned | High | Prevents spurious errors |
| 3 | Per-manager live status board (pending / running / done / failed / skipped) | 📋 Planned (Phase 2) | High | UX backbone. Phase 1 ships a final status table instead. |
| 4 | Size reporting: bytes freed per manager + total | 📋 Planned | High | The headline result |
| 5 | `--dry-run` — show what would run, don't execute | 📋 Planned | High | Safety |
| 6 | `--only <list>` and `--skip <list>` filters | 📋 Planned | High | Surgical runs |
| 7 | `-v/--verbose` — stream child stdout/stderr | 📋 Planned | Medium | Debugging |
| 8 | `--json` output for scripting | 📋 Planned | Medium | CI integrations |
| 9 | `--timeout <secs>` per manager | 📋 Planned | Medium | Don't hang forever |
| 10 | Exit codes: 0 ok, 1 one or more failed, 2 usage error | 📋 Planned | High | Scriptable |
| 11 | Extra managers (poetry, yarn, deno, go, gem, maven, gradle, nuget) | 📋 Planned | Low | Post-v1 |
| 12 | Config file `~/.config/cacheclear/config.toml` for defaults | 📋 Planned | Low | Post-v1 |
| 13 | `--aggressive` vs `--safe` profiles | ⏸ Deferred | Low | Unclear it adds value; revisit after real use |

---

## 🧰 Cache-Clear Matrix (the core technical spec)

This is the contract. For each supported manager, here is the exact action taken and why it is safe.

| Manager | Detection | Action | Safety note |
|---|---|---|---|
| **uv** | `uv --version` | `uv cache clean` | Clears download/build cache; does not remove installed tools/projects. |
| **pip** | `pip --version` (and try `pip3` as fallback) | `pip cache purge` | Documented cache-only command; never uninstalls. |
| **pipx** | `pipx --version` | Delete contents of every pipx venv cache dir that exists: `$PIPX_HOME/.cache` if set; pipx's own answer (`pipx environment --value PIPX_VENV_CACHEDIR`, plus `<PIPX_HOME>/.cache` left over from pre-1.3 layouts); else defaults `%LOCALAPPDATA%\pipx\pipx\Cache` (Windows), `~/Library/Caches/pipx` (macOS), `$XDG_CACHE_HOME/pipx` or `~/.cache/pipx` (Linux), and legacy `~/.local/pipx/.cache` / `~\pipx\.cache`. **Do NOT touch `venvs/`.** | pipx has no `cache clean` command ([pypa/pipx#1681](https://github.com/pypa/pipx/issues/1681)). Cache dir stores only ephemeral `pipx run` envs. |
| **cargo** | `cargo --version` | Delete contents of these dirs under `$CARGO_HOME` (default `~/.cargo`): `registry/cache/*`, `registry/src/*`, `git/db/*`, `git/checkouts/*`. **Do NOT touch `bin/` or `registry/index/`.** | Cargo has no cache command. These four dirs are documented as "safe to delete, will be re-downloaded." |
| **npm** | `npm --version` | `npm cache clean --force` | `--force` is required by npm; command only removes cache, not installed packages. |
| **pnpm** | `pnpm --version` | `pnpm store prune` | Removes unreferenced packages from the content-addressable store. Do NOT delete the store dir wholesale — that would break every pnpm project symlinking into it. |
| **bun** | `bun --version` | `bun pm --global cache rm` | Clears `~/.bun/install/cache`. Does not uninstall anything. The `--global` flag is required: `bun pm cache rm` without it operates on the *local* project's `node_modules` cache (errors when no project context exists), which is not what we want. |

For each manager, if detection fails → mark as **Skipped** and continue. Never error out the whole run because one manager is missing.

---

## 🛡️ Symlink Policy

Multiple cache trees contain or are themselves symlinks (pipx's `.cache/` ephemeral venvs symlink to the system Python interpreter; users frequently relocate large caches off their main disk via root-level symlinks). Naive recursive deletion through these would violate the no-uninstall invariant catastrophically — e.g. `rm -rf` through a venv symlink could chew into `/usr/lib/python3.x/`.

**Two rules govern all cache-tree deletion (`fs_util::safe_rm_contents`):**

1. **Cache root:** if the configured cache root is itself a symlink, resolve it once via `fs::canonicalize` and operate inside the resolved target. (Respects users who intentionally relocated a cache to another disk; we don't clobber the symlink itself.)
2. **Below the root:** never follow symlinks. The walker uses `WalkDir::new(canonical_root).follow_links(false).same_file_system(true)`. Any symlink entry encountered is removed via `fs::remove_file` on the link itself — never traversed into.

**Sanity check:** after canonicalizing the cache root, refuse to operate if the resolved path is at or above any of: `/`, `$HOME`, `/etc`, `/usr`, `/var`, `C:\`, `C:\Users`. A manager's "cache" resolving to a system root indicates a broken config, not a legitimate clear target — bail loudly and skip that manager.

This policy applies uniformly to direct-deletion managers (cargo, pipx) and is irrelevant for the subprocess managers (uv, pip, npm, pnpm, bun) since they handle their own filesystem traversal.

---

## 🔒 Concurrent-Install Handling

Two managers in the matrix can be actively writing to their cache from another shell when `cacheclear` runs:

- **cargo** holds an exclusive flock on `$CARGO_HOME/.package-cache` during downloads and extraction. Direct directory removal (which is how we clear cargo) bypasses this lock — so racing a `cargo build` could leave a half-extracted `.crate` and break the in-progress install.
- **pnpm** holds a lock in its content-addressable store during writes. `pnpm store prune` blocks on it and would eventually succeed, but a long stall during a parallel run is bad UX.

**Policy:** for cargo and pnpm, attempt a **non-blocking exclusive flock** on the manager's lock file before acting:

| Manager | Lock check |
|---|---|
| cargo | `fs2::FileExt::try_lock_exclusive` on `$CARGO_HOME/.package-cache` |
| pnpm | non-blocking lock on the store's lock file (`<store>/v3/.lock` or equivalent — discover via `pnpm store path`) |

If the lock is held by another process → mark as **Skipped (busy)** and continue. Do **not** wait or retry — a user with an active build doesn't want a background cache clear silently delaying it. They can re-run once builds finish.

```
  ⊘ cargo     skipped     another cargo process is using the cache
  ✓ pnpm      done        1.2 GB freed   (0.4s)
```

A "skipped (busy)" run does **not** count as a failure — exit code stays 0.

For the other five managers (uv, pip, pipx, npm, bun) we rely on either the manager's own subcommand handling concurrency, or on content-addressed caches being race-tolerant. No lock check needed.

---

## 🪟 Windows Long-Path Handling

Windows caps paths at 260 chars (`MAX_PATH`) at the API level. Cache trees blow past this routinely — cargo's `registry/src/index.crates.io-<hash>/<crate>-<version>/<deep>/<modules>/...` is the worst offender, but pnpm/npm/bun all hit it with deep enough projects. Without handling this, `walkdir` and `std::fs::remove_*` return `ERROR_PATH_NOT_FOUND` on entries that exist — silent partial cleanup, exactly the trust-eroding failure mode the tool exists to prevent.

**Policy:** on Windows, always operate through the `\\?\` extended-length path prefix. We don't depend on the OS-level `LongPathsEnabled` opt-in (users may not have set it; it's not universally available; assuming it makes the tool fragile).

The fix collapses cleanly into the same `fs_util::prepare_root()` helper that handles symlinks:

```rust
#[cfg(windows)]
fn prepare_root(p: &Path) -> io::Result<PathBuf> {
    let canonical = fs::canonicalize(p)?;
    let s = canonical.as_os_str().to_string_lossy();
    if s.starts_with(r"\\?\") {
        Ok(canonical)
    } else {
        Ok(PathBuf::from(format!(r"\\?\{}", s)))
    }
}

#[cfg(not(windows))]
fn prepare_root(p: &Path) -> io::Result<PathBuf> {
    fs::canonicalize(p)
}
```

Because `walkdir` constructs child paths by appending to the parent, prefixing once at the root means every descendant inherits the prefix automatically. Deletions and size counting are then long-path-safe with no further special handling.

**Display rule:** the `\\?\` prefix is hideous in user-facing output. When emitting paths in verbose mode, dry-run plans, or error messages, strip the prefix via `dunce::simplified`. **Internal filesystem operations keep the prefix; only display strings drop it.** Mixing these up reintroduces the original bug, so this distinction lives as a comment on `prepare_root` and a clippy-flagged convention in `fs_util`.

**Test coverage:** one integration test creates a tempdir tree nested past 300 chars and asserts cleanup runs to completion with zero leftover entries. Runs on Windows CI.

---

## 🖥️ CLI Surface (v1)

```
cacheclear [OPTIONS]

OPTIONS:
  --only <list>        Comma-separated managers to include (default: all detected)
                       e.g. --only cargo,npm
  --skip <list>        Comma-separated managers to exclude
                       e.g. --skip bun
  --dry-run            Print what would be done, don't execute
  --json               Emit machine-readable result to stdout
  -v, --verbose        Stream child process output
  --timeout <secs>     Kill any single manager that runs longer (default: 120)
  --no-color           Disable color output
  -h, --help           Show help
  -V, --version        Show version

MANAGERS (v1):
  uv  pip  pipx  cargo  npm  pnpm  bun
```

### Example output (default run)

```
cacheclear 0.1.0 — clearing 7 manager caches in parallel

  ✓ uv        done        312 MB freed     (0.8s)
  ✓ pip       done        1.4 GB freed     (1.2s)
  ✓ pipx      done        48 MB freed      (0.1s)
  ✓ cargo     done        7.2 GB freed     (3.4s)
  ✓ npm       done        980 MB freed     (1.9s)
  ⊘ pnpm      skipped     not installed
  ✓ bun       done        210 MB freed     (0.3s)

Total: 10.15 GB freed in 3.4s (6/7 ran, 1 skipped, 0 failed)
```

### Exit codes

| Code | Meaning |
|---|---|
| 0 | All runs succeeded (or all were skipped) |
| 1 | One or more managers failed |
| 2 | Usage / argument error |

---

## ⚙️ Tech Stack

| Concern | Choice | Why |
|---|---|---|
| Language | Rust (2024 edition, MSRV 1.85+) | User's preference; ideal for a small, fast, portable CLI. Edition 2024 was stabilized in Rust 1.85 (Feb 2025) and is the source of truth — bump in lockstep if `edition` is changed in `Cargo.toml`. |
| Arg parsing | `clap` (derive feature) | De-facto standard, great UX out of the box |
| Async runtime | `tokio` (multi-thread) | Native support for concurrent subprocesses + non-blocking I/O for streaming stdout/stderr |
| Process spawn | `tokio::process::Command` | Async wait + per-process timeout via `tokio::time::timeout` |
| Concurrency primitive | `futures::future::join_all` over one task per manager | Simplest model that fits ≤ ~15 concurrent tasks |
| Progress UI | `indicatif` with `MultiProgress` | Standard in Rust CLI land; integrates cleanly with tokio |
| Filesystem ops | `std::fs` + `walkdir` for size counting; `tokio::fs` where it helps | Size-before/size-after needs a quick walk |
| File locking | `fs2` crate (`try_lock_exclusive`) | Cross-platform non-blocking flock for cargo / pnpm busy-detection |
| Windows path display | `dunce` crate | Strip `\\?\` prefix from paths shown to user; internal ops keep the prefix |
| Home/config dirs | `dirs` crate | Correct cross-platform resolution of `$HOME`, `$LOCALAPPDATA`, `$CARGO_HOME`, etc. |
| Binary detection | `which` crate | Clean cross-platform PATH lookup |
| JSON output | `serde` + `serde_json` | Standard |
| Error handling | `anyhow` at the CLI boundary, `thiserror` for library-style errors | Idiomatic split |
| Logging (verbose mode) | `tracing` + `tracing-subscriber` | Structured, plays well with async |
| Testing | `assert_cmd` + `predicates` for CLI integration tests, unit tests inline | Standard |
| Packaging | `cargo-dist` for multi-platform release binaries | Less boilerplate than hand-rolling CI |

---

## 🏗️ Architecture Sketch

```
src/
├── main.rs             // parse args, construct runtime, dispatch
├── cli.rs              // clap definitions
├── manager.rs          // trait Manager { detect, plan, execute }
├── managers/
│   ├── uv.rs
│   ├── pip.rs
│   ├── pipx.rs         // direct dir removal
│   ├── cargo.rs        // direct dir removal
│   ├── npm.rs
│   ├── pnpm.rs
│   └── bun.rs
├── runner.rs           // orchestrates parallel execution + timeouts
├── report.rs           // status board + summary + JSON emitter
└── fs_util.rs          // prepare_root (canonicalize + \\?\ on Windows),
                        // dir_size, safe_rm_contents (never follows symlinks
                        // below root, refuses system paths)
```

Each manager module implements a tiny trait:

```rust
#[async_trait]
trait Manager {
    fn name(&self) -> &'static str;
    async fn detect(&self) -> bool;
    fn plan(&self) -> Plan;                    // for --dry-run
    async fn execute(&self, ctx: &RunCtx) -> ManagerResult;
}
```

This way "shell out to a subcommand" and "delete a directory" are both just implementations of the same trait, and adding a new manager post-v1 (poetry, yarn, etc.) is one new file.

---

## 🔀 Key Decisions

| Decision | Chosen Path | Rationale |
|---|---|---|
| Parallelism model | `tokio` + `join_all`, one task per manager | Subprocesses + streaming child output are a natural fit for async; `indicatif` progress bars update cleanly from multiple tasks |
| cargo clearing | Direct directory removal (no dep on `cargo-cache`) | `cargo-cache` isn't ubiquitous; the four dirs are documented-safe; keeps us dependency-free at runtime |
| pipx clearing | Direct cache-dir removal, explicitly scoped to pipx's venv cache dirs (current + legacy) | No official command exists; scoping to the cache dirs (never `venvs/`) preserves the uninstall-nothing invariant. pipx 1.3 moved the cache, so ask pipx first and also clear the leftover legacy dir. |
| pnpm clearing | `pnpm store prune` (not a full store wipe) | Wiping pnpm's content-addressable store breaks every project symlinking into it — that would violate the "don't uninstall" contract in spirit |
| Unknown / missing manager | Skip silently (still listed in output as "skipped") | Opinionated default: most users don't have all seven installed |
| Default verbosity | Summarized status board; child output hidden unless `-v` | The board is the product; raw npm/cargo output is noise by default |
| Symlinks | Canonicalize the cache root once; never follow links below it; refuse to operate if the resolved root is at or above a system path | Pipx venvs symlink to system Python — naive recursion could uninstall the interpreter. Two-rule policy preserves user-relocated caches while making interior link traversal impossible. |
| Concurrent installs | Non-blocking flock on cargo's `.package-cache` and pnpm's store lock; skip-if-busy, don't wait | Direct dir removal can corrupt an in-progress cargo extraction. Skipping is more honest than silently delaying a user's active build. |
| `--only` / `--skip` | Names validated by clap (unknown name = usage error, exit 2). `--only` applies first, then `--skip`. Filtered-out managers are left out of the report. | Typos must not silently widen a run. A filtered-out manager was not asked about, so listing it as "skipped" is noise. |
| Home directory | On Windows, honour `USERPROFILE` before the shell profile folder | Matches `std::env::home_dir` (Rust 1.86+) and makes the sandboxed-home tests hermetic. |
| Windows long paths | Always `\\?\`-prefix the canonicalized cache root on Windows; do not rely on OS `LongPathsEnabled` opt-in | Cargo / pnpm / npm cache trees routinely exceed 260 chars. Relying on user OS configuration makes the tool fragile. Single helper covers all entry points. |

---

## 🗑️ Discarded Options

| Item | Reason |
|---|---|
| Using `std::thread::spawn` per manager instead of tokio | Works, but makes streaming child stdout + timeouts + a shared progress renderer noticeably clunkier. Tokio is a small tax for a cleaner impl. |
| Depending on `cargo-cache` for cargo | Adds a hidden install requirement; defeats the point of a single binary that Just Works. |
| Clearing `cargo/registry/index/` | It's metadata, not download cache. Removing it forces a slow re-index without meaningfully bigger space savings. |
| Aggressive mode that wipes pnpm store entirely | Would break installed pnpm projects → violates the "never uninstall" guarantee. |
| Including system package managers (apt, brew, pacman, dnf) | Requires sudo, has security-update implications, and caches behave differently (e.g. brew's cache is already pruned on upgrade). Out of scope. |
| Config file in v1 | Flags cover every v1 use case; premature abstraction. Revisit once real usage surfaces repeated patterns. |

---

## 🗓️ Timeline & Phases

### 🏁 Phase 1 — MVP (~1 week) ⭐ THIS IS THE MVP
**Goal:** A working `cacheclear` that, on a polyglot dev box, clears all seven caches in parallel, reports what happened, and never violates the no-uninstall invariant.

**Features included:**
- Parallel clear for all 7 managers (matrix above)
- Auto-detection + skip-if-missing
- Final per-manager status table (the live `indicatif` board moved to Phase 2)
- Size-freed reporting
- `--only`, `--skip`, `--dry-run`, `-v`, `--timeout`
- Exit codes 0/1/2

**Exit criteria:**
- Runs cleanly on Linux, macOS, and Windows for at least 3 managers installed
- `--dry-run` prints the exact plan and touches nothing (verified via filesystem diff in tests)
- Integration test: create fake caches in a sandboxed `$HOME`, run, assert only cache paths change and no "installed" paths (bin dirs, venvs) are touched
- End-to-end run on a real dev machine with all 7 installed

**Effort estimate:** S (approx. 1 week)

---

### 🚀 Phase 2 — Polish & Distribution (~1 week)
**Goal:** Make it trivially installable.

**Features included:**
- `--json` output
- Live status board via `indicatif` (moved from Phase 1: it must not garble `-v` child output or `--json`)
- `cargo-dist` multi-platform release binaries (x86_64 + aarch64 for Linux/macOS, x86_64 Windows)
- Homebrew formula
- README with benchmark numbers
- Optional publish to `cargo install cacheclear` via crates.io

**Exit criteria:**
- One-line install on all three platforms
- JSON output validated against a schema in tests

**Effort estimate:** S (approx. 1 week)

---

### 🌟 Phase 3 — Manager Expansion (opportunistic)
**Goal:** Cover more of the ecosystem.

**Candidates:** poetry, yarn (classic + berry), deno, go (module cache), gem, nuget, composer, maven (`~/.m2/repository`), gradle (`~/.gradle/caches`).

**Exit criteria:**
- Each added manager has: detection, one clear action, one integration test, entry in the cache-clear matrix doc.

---

## Open Questions

_All initial open questions have been resolved into the policy sections above (Symlink Policy, Concurrent-Install Handling, Windows Long-Path Handling). New questions surfaced during implementation should be appended here._
