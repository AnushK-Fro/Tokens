//! Database access. `ingest` and `events` write; `summary` is read-only.

pub mod events;
pub mod ingest;
pub mod summary;

pub use events::{record_events, UsageEvent};
pub use ingest::record_report;
