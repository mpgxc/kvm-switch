//! Interface em GPUI.

pub mod overlay;
pub mod settings;

/// Cores do tema escuro, em RGB.
mod theme {
    pub const BACKGROUND: u32 = 0x1e1f22;
    pub const FIELD: u32 = 0x2b2d31;
    pub const HOVER: u32 = 0x35373c;
    pub const BORDER: u32 = 0x3f4147;
    pub const TEXT: u32 = 0xe6e6e6;
    pub const MUTED: u32 = 0x9aa0a6;
    pub const ACCENT: u32 = 0x4f8cff;
    pub const ERROR: u32 = 0xff6b6b;
}
