//! Status board, summary, and JSON emitter.
//!
//! Four emitters live here:
//! - `emit_run_report` / `emit_dry_run_plan`: human-readable, used by default.
//! - `emit_json_run` / `emit_json_dry_run`: machine-readable, used by `--json`.
//!
//! All four consume the same `ReportLine` / `RunSummary` data shapes, so
//! changing `Manager::execute` only requires touching `build_report` once.
//!
//! ## JSON schema (version 1)
//!
//! Run mode (`--json` without `--dry-run`):
//! ```json
//! {
//!   "version": 1,
//!   "mode": "run",
//!   "results": [
//!     { "manager": "cargo", "outcome": "done",
//!       "freed_bytes": 491279791, "took_ms": 4200, "note": null }
//!   ],
//!   "summary": {
//!     "total_freed_bytes": 491279791, "total_took_ms": 4200,
//!     "ran": 1, "skipped": 0, "failed": 0, "total": 1
//!   }
//! }
//! ```
//!
//! Dry-run mode (`--json --dry-run`):
//! ```json
//! {
//!   "version": 1,
//!   "mode": "dry_run",
//!   "plan": [
//!     { "manager": "cargo", "detected": true,
//!       "description": "...", "paths": ["..."],
//!       "would_free_bytes": 491279791 }
//!   ],
//!   "total_would_free_bytes": 491279791,
//!   "any_size_unknown": false
//! }
//! ```

use crate::manager::{ManagerResult, Plan};
use serde::Serialize;
use std::time::Duration;

/// One line of the final summary table — the data shape that both
/// the human and JSON emitters consume.
#[derive(Debug, Serialize)]
pub struct ReportLine {
    pub manager: String,
    pub outcome: String,
    pub freed_bytes: u64,
    pub took_ms: u128,
    pub note: Option<String>,
}

/// Aggregate stats for the bottom-line summary.
pub struct RunSummary {
    pub total_freed: u64,
    pub total_took: Duration,
    pub ran: usize,
    pub skipped: usize,
    pub failed: usize,
}

impl RunSummary {
    pub fn from_results(results: &[(String, ManagerResult)]) -> Self {
        let mut s = Self {
            total_freed: 0,
            total_took: Duration::ZERO,
            ran: 0,
            skipped: 0,
            failed: 0,
        };
        for (_, r) in results {
            match r {
                ManagerResult::Done { freed, took } => {
                    s.ran += 1;
                    s.total_freed = s.total_freed.saturating_add(*freed);
                    if *took > s.total_took {
                        s.total_took = *took;
                    }
                }
                ManagerResult::SkippedMissing | ManagerResult::SkippedBusy { .. } => {
                    s.skipped += 1;
                }
                ManagerResult::Failed { .. } => {
                    s.failed += 1;
                }
            }
        }
        s
    }
}

/// Convert raw manager results into structured rows.
/// Pure function, easy to unit-test.
pub fn build_report(results: &[(String, ManagerResult)]) -> Vec<ReportLine> {
    results
        .iter()
        .map(|(name, r)| match r {
            ManagerResult::Done { freed, took } => ReportLine {
                manager: name.clone(),
                outcome: "done".into(),
                freed_bytes: *freed,
                took_ms: took.as_millis(),
                note: None,
            },
            ManagerResult::SkippedMissing => ReportLine {
                manager: name.clone(),
                outcome: "skipped".into(),
                freed_bytes: 0,
                took_ms: 0,
                note: Some("not installed".into()),
            },
            ManagerResult::SkippedBusy { reason } => ReportLine {
                manager: name.clone(),
                outcome: "skipped".into(),
                freed_bytes: 0,
                took_ms: 0,
                note: Some(format!("busy: {reason}")),
            },
            ManagerResult::Failed { error } => ReportLine {
                manager: name.clone(),
                outcome: "failed".into(),
                freed_bytes: 0,
                took_ms: 0,
                note: Some(error.clone()),
            },
        })
        .collect()
}

/// Print the per-manager status board + bottom-line summary for a real run.
///
/// Output matches `spec/SPEC.md` §"Example output (default run)":
///
/// ```text
///   ✓ uv        done        312 MB freed     (0.8s)
///   ⊘ pnpm      skipped     not installed
///   ✗ npm       failed      <error>
///
/// Total: 10.15 GB freed in 3.4s (6/7 ran, 1 skipped, 0 failed)
/// ```
pub fn emit_run_report(results: &[(String, ManagerResult)]) {
    for (name, r) in results {
        match r {
            ManagerResult::Done { freed, took } => {
                println!(
                    "  ✓ {name:<8} done       {:>10}   ({})",
                    format_bytes(*freed),
                    format_duration(*took),
                );
            }
            ManagerResult::SkippedMissing => {
                println!("  ⊘ {name:<8} skipped    not installed");
            }
            ManagerResult::SkippedBusy { reason } => {
                println!("  ⊘ {name:<8} skipped    busy: {reason}");
            }
            ManagerResult::Failed { error } => {
                println!("  ✗ {name:<8} failed     {error}");
            }
        }
    }

    let s = RunSummary::from_results(results);
    let total = results.len();
    println!();
    println!(
        "Total: {} freed in {} ({}/{} ran, {} skipped, {} failed)",
        format_bytes(s.total_freed),
        format_duration(s.total_took),
        s.ran,
        total,
        s.skipped,
        s.failed,
    );
}

/// Print the per-manager plan when `--dry-run` is set.
///
/// `entries` carries (name, detected, plan, would_free):
/// - `detected`: did `Manager::detect()` succeed? If false, we skip with a
///   "not installed" note.
/// - `plan`: from `Manager::plan()` — description + concrete paths.
/// - `would_free`: bytes that would be freed (where computable). For
///   subprocess managers we don't know without running the before-probe,
///   so this is `None` and we omit the size column.
pub fn emit_dry_run_plan(entries: &[(String, bool, Plan, Option<u64>)]) {
    println!("DRY RUN — no caches will be modified.\n");

    for (name, detected, plan, would_free) in entries {
        if !*detected {
            println!("  ⊘ {name:<8} skipped    not installed");
            continue;
        }
        match would_free {
            Some(bytes) => println!(
                "  • {name:<8} {:<10}  {}",
                format_bytes(*bytes),
                plan.description
            ),
            None => println!("  • {name:<8} {:<10}  {}", "?", plan.description),
        }
        for p in &plan.paths {
            println!("      └─ {}", crate::fs_util::display_path(p).display());
        }
    }

    let total: u64 = entries
        .iter()
        .filter_map(|(_, detected, _, w)| if *detected { *w } else { None })
        .sum();
    let any_unknown = entries
        .iter()
        .any(|(_, detected, _, w)| *detected && w.is_none());
    println!();
    if any_unknown {
        println!(
            "Would free: at least {} (some sizes unknown until executed)",
            format_bytes(total)
        );
    } else {
        println!("Would free: {}", format_bytes(total));
    }
}

/// Format a byte count human-readably: B / KB / MB / GB / TB.
/// Uses SI base-1000 to match `du -h` and the spec's example output.
pub fn format_bytes(n: u64) -> String {
    const UNITS: &[&str] = &["B", "KB", "MB", "GB", "TB", "PB"];
    let mut value = n as f64;
    let mut unit_idx = 0;
    while value >= 1000.0 && unit_idx < UNITS.len() - 1 {
        value /= 1000.0;
        unit_idx += 1;
    }
    if unit_idx == 0 {
        format!("{n} B")
    } else {
        format!("{value:.1} {}", UNITS[unit_idx])
    }
}

/// Format a Duration as a short human string: `850ms`, `1.2s`, `1m 23s`.
pub fn format_duration(d: Duration) -> String {
    let total_ms = d.as_millis();
    if total_ms < 1000 {
        return format!("{total_ms}ms");
    }
    let secs = d.as_secs_f64();
    if secs < 60.0 {
        return format!("{secs:.1}s");
    }
    let mins = (secs / 60.0).floor() as u64;
    let rem = (secs - (mins as f64) * 60.0).round() as u64;
    format!("{mins}m {rem}s")
}

// ========================================================================
//                              JSON emitters
// ========================================================================

/// Top-level JSON document for a real run (`--json`).
#[derive(Debug, Serialize)]
struct JsonRunDoc {
    version: u32,
    mode: &'static str,
    results: Vec<ReportLine>,
    summary: JsonRunSummary,
}

#[derive(Debug, Serialize)]
struct JsonRunSummary {
    total_freed_bytes: u64,
    total_took_ms: u128,
    ran: usize,
    skipped: usize,
    failed: usize,
    total: usize,
}

/// Top-level JSON document for `--json --dry-run`.
#[derive(Debug, Serialize)]
struct JsonDryRunDoc {
    version: u32,
    mode: &'static str,
    plan: Vec<JsonPlanEntry>,
    total_would_free_bytes: u64,
    any_size_unknown: bool,
}

#[derive(Debug, Serialize)]
struct JsonPlanEntry {
    manager: String,
    detected: bool,
    description: String,
    paths: Vec<String>,
    /// `null` when size is unknown (subprocess manager whose cache_dir()
    /// probe failed) or when the manager wasn't detected.
    would_free_bytes: Option<u64>,
}

/// Emit a JSON document on stdout describing the run results.
pub fn emit_json_run(results: &[(String, ManagerResult)]) {
    let summary = RunSummary::from_results(results);
    let doc = JsonRunDoc {
        version: 1,
        mode: "run",
        results: build_report(results),
        summary: JsonRunSummary {
            total_freed_bytes: summary.total_freed,
            total_took_ms: summary.total_took.as_millis(),
            ran: summary.ran,
            skipped: summary.skipped,
            failed: summary.failed,
            total: results.len(),
        },
    };
    // Pretty-print so humans can also read the output; piping through `jq`
    // doesn't care, and one-off scripting consumers won't either.
    match serde_json::to_string_pretty(&doc) {
        Ok(s) => println!("{s}"),
        Err(e) => eprintln!("(internal) failed to serialize JSON: {e}"),
    }
}

/// Emit a JSON document on stdout describing the dry-run plan.
pub fn emit_json_dry_run(entries: &[(String, bool, Plan, Option<u64>)]) {
    let plan: Vec<JsonPlanEntry> = entries
        .iter()
        .map(|(name, detected, plan, would_free)| JsonPlanEntry {
            manager: name.clone(),
            detected: *detected,
            description: plan.description.clone(),
            paths: plan
                .paths
                .iter()
                .map(|p| crate::fs_util::display_path(p).display().to_string())
                .collect(),
            would_free_bytes: if *detected { *would_free } else { None },
        })
        .collect();

    let total_would_free_bytes: u64 = entries
        .iter()
        .filter_map(|(_, detected, _, w)| if *detected { *w } else { None })
        .sum();
    let any_size_unknown = entries
        .iter()
        .any(|(_, detected, _, w)| *detected && w.is_none());

    let doc = JsonDryRunDoc {
        version: 1,
        mode: "dry_run",
        plan,
        total_would_free_bytes,
        any_size_unknown,
    };

    match serde_json::to_string_pretty(&doc) {
        Ok(s) => println!("{s}"),
        Err(e) => eprintln!("(internal) failed to serialize JSON: {e}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bytes_formatting() {
        assert_eq!(format_bytes(0), "0 B");
        assert_eq!(format_bytes(999), "999 B");
        assert_eq!(format_bytes(1000), "1.0 KB");
        assert_eq!(format_bytes(1_500_000), "1.5 MB");
        assert_eq!(format_bytes(491_279_791), "491.3 MB");
        assert_eq!(format_bytes(10_150_000_000), "10.2 GB");
    }

    #[test]
    fn duration_formatting() {
        assert_eq!(format_duration(Duration::from_millis(850)), "850ms");
        assert_eq!(format_duration(Duration::from_millis(1_200)), "1.2s");
        assert_eq!(format_duration(Duration::from_secs(83)), "1m 23s");
    }
}
