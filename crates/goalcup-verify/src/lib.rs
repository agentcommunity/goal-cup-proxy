//! `goalcup-verify` (PRD-08): reads GCR1 records, proves them whole and names every loss.
//! [`input`] resolves entry, rec, run, fetched and live inputs; [`integrity`] runs PRD-01's 14
//! invariants and the record checks in one streaming pass; [`server`] proves a fetched server
//! copy (seam S13); [`report`] builds the `check --json` report (S6), its exit code and the text
//! form; [`usage`] is PRD-09's usage reader.

pub mod input;
pub mod integrity;
pub mod report;
pub mod server;
pub mod usage;
