//! Flux2D core: the pieces of the game loop that don't touch the GPU.
//!
//! - [`FixedTimestep`]: a 60 Hz (or any rate) simulation clock with an interpolation factor
//!   for drawing between steps.
//! - [`Input`]: keyboard and mouse state, with "pressed this frame" edges.
//! - [`FrameStats`]: frame time history with average, p95, p99 and max, the same fields the
//!   Cosmic Commando `PERF SUMMARY` prints, so the two can be compared.

mod input;
mod stats;
mod time;

pub use input::{Input, KeyCode, MouseButton};
pub use stats::{FrameStats, Summary};
pub use time::FixedTimestep;
