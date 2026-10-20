//! One green theme. No global terminal palette or background is changed.

pub const GREEN: &str = "\x1b[32m";
pub const BRIGHT: &str = "\x1b[92m";
pub const RESET: &str = "\x1b[0m";

/// Capabilities supplied by the terminal adapter; the renderer reads no environment.
#[derive(Debug, Clone, Copy)]
pub struct Style {
    pub width: usize,
    pub height: usize,
    pub color: bool,
    pub unicode: bool,
    pub animate: bool,
}

impl Style {
    pub fn plain(width: usize, height: usize) -> Self {
        Self {
            width,
            height,
            color: false,
            unicode: false,
            animate: false,
        }
    }
}
