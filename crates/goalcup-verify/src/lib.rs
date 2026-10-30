//! `goalcup-verify` (PRD-08): reads GCR1 records, proves them whole and names every loss. [`input`]
//! resolves the input; [`integrity`] runs PRD-01's 14 invariants in one streaming pass and feeds
//! [`honesty`] (the checks that fail run 1's shape); [`server`] and [`secrets`] are S13 and S12;
//! [`report`] builds the S6 report; [`usage`] is PRD-09's.

pub mod honesty;
pub mod input;
pub mod integrity;
pub mod report;
pub mod secrets;
pub mod server;
pub mod usage;
