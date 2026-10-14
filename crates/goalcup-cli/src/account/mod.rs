//! The participant's accounts for a run: today the GitHub repository the agent pushes to.
//!
//! [`github`] checks the repository `goalcup prep` was given, clones it as the agent's work
//! folder, and answers git's credential protocol for it, so the token that can push there
//! never sits in a clone's configuration, a remote URL, a command line or the agent's
//! environment.

pub mod github;
