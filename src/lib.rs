//! repoforge as a library: the audit engine, GitHub client, rubric config, fix generators, and
//! report formatters. The `repoforge` binary is a thin CLI on top of these modules; exposing them
//! as a library is what lets the integration tests in `tests/` exercise the logic directly.

pub mod audit;
pub mod config;
pub mod github;
pub mod remediate;
pub mod report;
