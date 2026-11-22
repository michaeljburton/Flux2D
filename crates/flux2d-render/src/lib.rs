//! Flux2D renderer: draws very large numbers of sprites with a handful of draw calls.
//!
//! How it stays fast:
//! - Every sprite is one 48-byte [`SpriteInstance`]; there are no per-sprite objects on the GPU.
//! - All sprite art lives in one texture array ([`Atlas`]), so ships, shots and effects share a
//!   bind group and can be drawn together.
//! - A [`SpriteBatch`] buckets sprites by draw layer and blend mode as they are submitted
//!   (no sorting), and [`SpriteRenderer`] uploads the whole frame in one buffer write and issues
//!   one instanced draw per run of buckets with the same blend mode.
//! - The quad is generated in the vertex shader from the vertex index; there is no vertex buffer.
//! - Text is sprites too: fonts are baked into the atlas ([`AtlasBuilder::add_font`]) and every
//!   glyph is one instance, so HUD text shares the frame's draw calls.
//! - [`Space::Screen`] sprites (HUD, overlay) are drawn in window pixels after the world.

mod atlas;
mod batch;
mod camera;
mod gpu;
mod renderer;
pub mod text;

pub use atlas::{Atlas, AtlasBuilder, AtlasError, IMAGE_EXTENSIONS, SpriteId, SpriteRegion};
pub use batch::{Blend, DrawRun, Space, SpriteBatch, SpriteInstance};
pub use camera::Camera2D;
pub use gpu::{Gpu, GpuError};
pub use renderer::{FrameUpload, SpriteRenderer};
pub use text::{Align, Font, FontId, Glyph, TextStyle};
pub use wgpu;
