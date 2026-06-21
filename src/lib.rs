//! tmux-palette — a fast, scriptable Raycast-style command palette for tmux.
//!
//! Rust port of https://github.com/eduwass/tmux-palette (originally TypeScript/Bun).
//! The crate is split into the same modules as the original so behaviour maps 1:1.

pub mod cli;
pub mod dispatch;
pub mod fuzzy;
pub mod palette;
pub mod palettes;
pub mod raw;
pub mod render;
pub mod text;
pub mod theme;
pub mod themes_bundled;
pub mod tmux;
pub mod types;
pub mod user_config;
