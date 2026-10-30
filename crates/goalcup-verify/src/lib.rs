//! `goalcup-verify` (PRD-08): reads GCR1 records, proves them whole and names every loss.
//!
//! - [`input`]: entry, rec, run, fetched and live inputs, part file order.
//! - [`integrity`]: PRD-01's 14 invariants and the record checks, in one streaming pass.
//! - [`report`]: the `check --json` report (seam S6), its exit code and the text form.
//! - [`usage`]: PRD-09's usage reader.

pub mod input;
pub mod integrity;
pub mod report;
pub mod usage;
