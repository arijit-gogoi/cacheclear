//! Per-manager modules. Each implements `crate::manager::Manager`.
//!
//! Subprocess managers (shell out to a documented cache command):
//!   uv, pip, npm, pnpm, bun, scoop
//! Direct-deletion managers (walk and remove cache dirs under `fs_util` rules):
//!   cargo, pipx

pub mod bun;
pub mod cargo;
pub mod npm;
pub mod pip;
pub mod pipx;
pub mod pnpm;
pub mod scoop;
pub mod uv;
