//! The one synthetic provider key every secret-handling test uses.
//!
//! It has the shape DeepSeek issues, `sk-` and thirty-two hex digits, and is made up: it is never
//! a real credential, and nothing here reads one. A test that needs the key takes it from here, so
//! one string stands for it everywhere and a test that greps for its fragments greps for the same
//! bytes the others plant.

/// A synthetic key of the shape DeepSeek issues. Never a real credential.
pub const KEY: &str = "sk-3f9a1c7e5b2d4086a1f3c5e7d9b0a2c4";
