//! Database access. `ingest` is the only writer; `summary` is read-only.

pub mod ingest;
pub mod summary;

pub use ingest::record_report;
