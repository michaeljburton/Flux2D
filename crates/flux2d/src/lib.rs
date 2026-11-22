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
//!
//! The parts, all re-exported here:
//!
//! | Module | What |
//! | --- | --- |
//! | [`core`] | fixed timestep, keyboard / mouse / gamepad input, frame stats, `Rng`, profiling |
//! | [`render`] | sprites, the texture array atlas, text, the camera |
//! | [`physics`] | the collision grid: circles and boxes, layers and masks, raycasts |
//! | [`fx`] | particles and trails |
//! | [`audio`] | sounds and music on master / music / SFX / voice buses (kira) |
//! | [`ecs`] | bevy_ecs on its own, for game objects (`ecs` feature, on by default) |
//! | [`assets`] | finding the game's folder, loading art, sounds, fonts and JSON |

mod app;
pub mod assets;
mod overlay;
mod screenshot;

pub use app::{Config, Context, FrameTimings, Game, run};
#[cfg(feature = "ecs")]
pub use bevy_ecs as ecs;
pub use flux2d_audio as audio;
pub use flux2d_core as core;
pub use flux2d_core::{profile_frame, profile_scope};
pub use flux2d_fx as fx;
pub use flux2d_physics as physics;
pub use flux2d_render as render;
pub use glam;
pub use overlay::DevOverlay;
pub use winit;

pub mod prelude {
    pub use crate::assets::Assets;
    pub use crate::{Config, Context, DevOverlay, FrameTimings, Game};
    pub use flux2d_audio::{Audio, Bus, PlayParams, Sound};
    pub use flux2d_core::{
        FixedTimestep, FrameStats, GamepadAxis, GamepadButton, Input, KeyCode, MouseButton, Rng, Summary,
    };
    pub use flux2d_fx::{Emitter, Particles, Trail};
    pub use flux2d_physics::{Collider, CollisionGrid, RayHit, Shape};
    pub use flux2d_render::{
        Align, Atlas, AtlasBuilder, Blend, Camera2D, Font, FontId, Space, SpriteBatch, SpriteId,
        SpriteInstance, SpriteRegion, TextStyle,
    };
    pub use glam::{Vec2, vec2};
}
