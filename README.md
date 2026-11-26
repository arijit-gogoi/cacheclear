# cacheclear

Clear the download/build caches of common package managers in parallel — **never uninstalls anything**.

```text
$ cacheclear
  ✓ uv       done              0 B   (5.4s)
  ✓ pip      done         160.1 MB   (11.7s)
  ✓ pipx     done              0 B   (5.4s)
  ✓ cargo    done         167.1 MB   (10.3s)
  ✓ npm      done           2.2 GB   (23.6s)
  ✓ pnpm     done         197.5 MB   (14.9s)
  ✓ bun      done           2.3 GB   (1m 10s)

Total: 5.1 GB freed in 1m 10s (7/7 ran, 0 skipped, 0 failed)
```

(Real output from a Windows 11 dev machine. uv shows 0 B because `cacheclear --only uv` had already cleared its 5.9 GB cache a minute earlier.)

## Why

Every package manager you use silently piles download and build artifacts into its cache: cargo's `~/.cargo/registry/src`, npm's `_cacache`, pnpm's content-addressable store, pip's wheel cache, and so on. On a polyglot dev machine these add up to **tens of gigabytes** that nobody asked for.

`cacheclear` runs the documented cache-clearing command for each manager **in parallel**, reports what was freed, and is paranoid about one specific thing: it will **never** remove anything you'd consider installed. No binaries from `~/.cargo/bin`, no pipx venvs, no npm/pnpm/bun globally installed packages, no system package managers. The only thing it touches is cache.

## Install

Not yet published to crates.io, and no release binaries yet. Build from source:

```sh
git clone https://github.com/arijit-gogoi/cacheclear
cd cacheclear
cargo build --release
./target/release/cacheclear --help
```

Requires Rust 1.85+ (edition 2024).

## Usage

```sh
cacheclear                   # detect what's installed, clear it all in parallel
cacheclear --dry-run         # show what would be freed, touch nothing
cacheclear --only cargo,npm  # only these managers
cacheclear --skip bun        # everything except these
cacheclear -v                # stream child process output
cacheclear --json            # machine-readable output
cacheclear --timeout 60      # stop any single manager after 60 s (default 120)
```

Manager names for `--only` / `--skip`: `uv`, `pip`, `pipx`, `cargo`, `npm`, `pnpm`, `bun`. An unknown name is a usage error (exit 2), so a typo never widens a run. If both flags are given, `--only` applies first, then `--skip`. Managers filtered out are left out of the report.

`--dry-run` prints a per-manager plan with concrete paths and a total:

```text
DRY RUN — no caches will be modified.

  • uv       0 B         run `uv cache clean`
  • pip      160.1 MB    run `pip cache purge`
  • pipx     0 B         delete contents of pipx venv cache dirs (NEVER `venvs/`)
      └─ C:\Users\you\pipx\.cache
  • cargo    167.1 MB    delete contents of cargo registry/cache, registry/src, git/db, git/checkouts
      └─ C:\Users\you\.cargo\registry/cache
      └─ C:\Users\you\.cargo\registry/src
      └─ C:\Users\you\.cargo\git/db
      └─ C:\Users\you\.cargo\git/checkouts
  • npm      3.4 GB      run `npm cache clean --force`
  • pnpm     484.3 MB    run `pnpm store prune`
  • bun      2.3 GB      run `bun pm --global cache rm`

Would free: 6.6 GB
```

Dry-run sizes are upper bounds for npm and pnpm: they measure the whole cache dir, but `npm cache clean` and `pnpm store prune` keep some of it.

`--json` emits one JSON document (version 1) on stdout. Its shape is defined by [`schema/output.schema.json`](schema/output.schema.json), and the tests validate real output against it.

## What it touches (the whole table)

| Manager   | Action | What's safe about it |
| --------- | ------ | -------------------- |
| **uv**    | `uv cache clean` | Documented cache-only command. |
| **pip**   | `pip cache purge` | Documented cache-only command. Tries `pip3` fallback. |
| **pipx**  | Deletes contents of every pipx venv cache dir that exists: the one pipx reports (`PIPX_VENV_CACHEDIR`), `$PIPX_HOME/.cache`, and pre-1.3 leftovers such as `~/.local/pipx/.cache`. | **Never touches `venvs/`** — those are installed tools. |
| **cargo** | Deletes contents of `registry/cache`, `registry/src`, `git/db`, `git/checkouts` under `$CARGO_HOME`. | **Never touches `bin/` or `registry/index/`.** Each of these four dirs is documented as safe to delete; cargo re-downloads as needed. |
| **npm**   | `npm cache clean --force` | The `--force` flag is required by npm itself — it refuses without it. |
| **pnpm**  | `pnpm store prune` | Removes only unreferenced packages. **Never wipes the store dir** — every project on the machine links into it. |
| **bun**   | `bun pm --global cache rm` | Clears `~/.bun/install/cache`. Doesn't uninstall anything. |

## Safety guarantees

These aren't aspirational. They're tested on Linux, macOS, and Windows in CI.

1. **No uninstall, ever.** The integration tests (`tests/no_uninstall.rs`) build a fake home containing both cache files (which we expect to delete) and "installed" files (`cargo/bin/`, `pipx/venvs/`, `~/.bashrc`, user data). After each run, every installed file must be **byte-identical** to before.

2. **`--dry-run` touches nothing.** A test snapshots every file in the fake home before and after `--dry-run` and requires them to be identical.

3. **Symlinks below a cache root are never followed.** Pipx ephemeral venvs symlink to the system Python; naive recursion through them could `rm -rf` into `/usr/lib/python3.x/`. The walker uses `WalkDir::new(canonical_root).follow_links(false).same_file_system(true)`. Symlink entries get the link itself removed, never traversed.

4. **System paths are refused.** After canonicalizing a cache root, if it resolves to `/`, `$HOME`, `/etc`, `/usr`, `/var`, `C:\`, or `C:\Users` (or above one of them), the manager refuses to clear it and is reported as failed. A "cache" resolving to a system root means something is misconfigured.

5. **Concurrent installs are detected, not raced.** `cacheclear` takes a **non-blocking** exclusive lock on cargo's `$CARGO_HOME/.package-cache` and pnpm's store lock. If another process holds it (an active `cargo build`, say), that manager is reported **skipped (busy)** and the run continues with exit 0. It never waits and never retries.

6. **Windows long paths are handled.** Cache trees routinely pass Windows' 260-char `MAX_PATH`. `cacheclear` works through the `\\?\` extended-length prefix on Windows and does not depend on the OS `LongPathsEnabled` setting. A test clears a cache path over 300 characters long.

7. **Timeouts are enforced.** When `--timeout` fires, subprocess managers are killed, and cargo/pipx deletion stops at the next file.

## Exit codes

| Code | Meaning |
| ---- | ------- |
| `0`  | All managers ran, or were skipped (busy or not installed) |
| `1`  | One or more managers failed (including timeouts) |
| `2`  | Usage / argument error |

## Status & roadmap

- **Phase 1 — MVP (done):** parallel clear for all 7 managers, auto-detection, final status table, size reporting, `--only`/`--skip`/`--dry-run`/`-v`/`--timeout`, exit codes, sandboxed-home tests, CI on three OSes.
- **Phase 2 — polish & distribution (in progress):** done: `--json` output with schema, live status board, `cargo-dist` release config (Linux/macOS x86_64 + aarch64, Windows x86_64; shell and PowerShell installers). Not yet: first tagged release, Homebrew formula, crates.io publish.
- **Phase 3 — more managers:** poetry, yarn (classic + berry), deno, go, gem, nuget, composer, maven, gradle.

See [`spec/SPEC.md`](spec/SPEC.md) for the full design rationale.

## License

Dual-licensed under [MIT](LICENSE-MIT) or [Apache-2.0](LICENSE-APACHE), at your option.
