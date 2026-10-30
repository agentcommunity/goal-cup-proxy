//! goalcup 1.0: one recorder process per entry (PRD-00).
//!
//! The module list is frozen after the PRD-02 scaffold. Each module has one owner:
//!
//! - [`pipeline`] (PRD-02): the recorder handle (seam S1), guard, mask worker, writer, counts and
//!   `status.json` (seam S4), seal, boot recovery.
//! - [`watchdog`] (PRD-02): the restart loop and null mode, control lines (seam S8), start facts
//!   (seam S17).
//! - [`proxy`] (PRD-03): the local recording proxy.
//! - [`native`] (PRD-04): the session-file reader.
//! - [`mask`] (PRD-05): masking (seam S2).
//! - [`upload`] (PRD-06): the uploader.
//! - [`connector`] (PRD-11): connectors and host lists (seam S3).
//! - [`cli`] (PRD-10): the participant commands.

pub mod cli;
pub mod connector;
pub mod mask;
pub mod native;
pub mod pipeline;
pub mod proxy;
pub mod upload;
pub mod watchdog;
