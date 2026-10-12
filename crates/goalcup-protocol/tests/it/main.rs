//! Fast tier: the protocol crate's integration tests, folded into one binary.
//! `export_roundtrip` (process umask) and `capture_reserved_storage` (re-executes itself by
//! test name) stay separate binaries.

mod addenda;
mod capture_storage;
mod capture_terminals;
mod chain_golden;
mod connector_pins;
mod control_consumer;
mod control_shapes;
mod harness_conformance;
mod harness_loader;
mod harness_schema;
mod m2_contract;
mod manifest_load;
mod no_family_literals;
mod pins_build_fails;
mod provenance_wire;
mod ps_env;
mod roundtrip;
mod submission_contract;
