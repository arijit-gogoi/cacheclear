//! Status board, summary, and JSON emitter.
//!
//! Default UI is the per-manager `indicatif` board described in `spec/SPEC.md`.
//! `--json` swaps the board for a single JSON document on stdout.

use crate::manager::ManagerResult;
use serde::Serialize;

/// One line of the final summary table.
#[derive(Debug, Serialize)]
pub struct ReportLine {
    pub manager: String,
    pub outcome: String,
    pub freed_bytes: u64,
    pub took_ms: u128,
    pub note: Option<String>,
}

/// Convert raw manager results into rows; pure function, easy to unit-test.
pub fn build_report(_results: &[(String, ManagerResult)]) -> Vec<ReportLine> {
    Vec::new()
}
