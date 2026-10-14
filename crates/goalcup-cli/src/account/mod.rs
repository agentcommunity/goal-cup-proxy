//! The participant's accounts for a run: the Goal Cup account that approves it, and the GitHub
//! repository the agent pushes to.
//!
//! [`device`] asks the upload server for a device code and waits while the participant approves
//! the run (or a login) in the browser; [`credential`] keeps the run-bound credential an
//! approval returns, at mode 0600 in the run's own directory.
//!
//! [`github`] checks the repository `goalcup prep` was given, clones it as the agent's work
//! folder, and answers git's credential protocol for it, so the token that can push there
//! never sits in a clone's configuration, a remote URL, a command line or the agent's
//! environment.

pub mod credential;
pub mod device;
pub mod github;
