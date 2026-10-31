//! `goalcup-verify` (PRD-08): reads GCR1 records, proves them whole and names every loss.
//! [`input`] resolves entry, rec, run, fetched and live inputs; [`integrity`] runs PRD-01's 14
//! invariants and the record checks in one streaming pass and feeds [`honesty`] and [`legs`] (the
//! checks that fail run 1's shape); [`server`] proves a server copy (S13); [`secrets`] is the scan
//! (S12); [`report`] builds the `check --json` report (S6), exit code and text; [`usage`] is U1.

pub mod honesty;
pub mod input;
pub mod integrity;
pub mod legs;
pub mod report;
pub mod secrets;
pub mod server;
pub mod usage;
