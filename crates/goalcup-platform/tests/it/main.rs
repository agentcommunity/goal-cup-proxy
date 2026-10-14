//! Fast tier: the platform crate's integration tests, folded into one binary. All of them
//! exercise Unix descriptor primitives.

#[cfg(unix)]
mod artifact_root_unix;
#[cfg(unix)]
mod awake;
#[cfg(unix)]
mod machine;
#[cfg(unix)]
mod readcap_unix;
#[cfg(unix)]
mod unix_snapshot_primitives;
#[cfg(unix)]
mod unix_sourcecap;
#[cfg(unix)]
mod unix_tracecap;
