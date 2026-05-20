//! Library crate for the Pixel Agents TUI client.
//!
//! Exists so integration tests (`tests/*.rs`) can exercise the pure simulation
//! engine (`office`) and wire types directly. The binary (`src/main.rs`) is a
//! thin wrapper over [`app::run`].

pub mod agents;
pub mod app;
pub mod assets;
pub mod caps;
pub mod chrome;
pub mod daemon;
pub mod focus;
pub mod input_queue;
pub mod keymap;
pub mod office;
pub mod raw_mode;
pub mod reconnect;
pub mod render;
pub mod tui;
