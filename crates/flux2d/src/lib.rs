//! # Flux2D
//!
//! A 2D game engine built to draw very large numbers of sprites.
//!
//! Implement [`Game`], then call [`run`]:
//!
//! ```no_run
//! use flux2d::prelude::*;
//!
//! struct Hello;
//!
//! impl Game for Hello {
//!     fn update(&mut self, _ctx: &mut Context, _dt: f32) {}
//!     fn draw(&mut self, _ctx: &mut Context, _batch: &mut SpriteBatch, _alpha: f32) {}
//! }
//!
//! fn main() {
//!     flux2d::run(Config::default(), |_ctx| Hello).unwrap();
//! }
//! ```
//!
//! [`Game::update`] runs at a fixed rate (60 Hz by default); [`Game::draw`] runs once per
//! displayed frame with `alpha`, how far the clock is between two updates, for smooth motion
//! at any refresh rate.

mod app;
mod screenshot;

pub use app::{Config, Context, FrameTimings, Game, run};
pub use flux2d_core as core;
pub use flux2d_render as render;
pub use glam;
pub use winit;

pub mod prelude {
    pub use crate::{Config, Context, FrameTimings, Game};
    pub use flux2d_core::{FixedTimestep, FrameStats, Input, KeyCode, MouseButton, Summary};
    pub use flux2d_render::{
        Atlas, AtlasBuilder, Blend, Camera2D, SpriteBatch, SpriteId, SpriteInstance, SpriteRegion,
    };
    pub use glam::{Vec2, vec2};
}
