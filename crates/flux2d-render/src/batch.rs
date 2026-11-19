use crate::SpriteRegion;
use bytemuck::{Pod, Zeroable};
use glam::Vec2;

/// How a sprite combines with what is already drawn.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum Blend {
    /// Normal transparency (premultiplied alpha).
    Alpha = 0,
    /// Light adds up: glows, lasers, explosions, engine flames.
    Additive = 1,
}

/// One sprite as the GPU sees it: 48 bytes, no pointers, no per-sprite objects.
#[repr(C)]
#[derive(Debug, Clone, Copy, Default, PartialEq, Pod, Zeroable)]
pub struct SpriteInstance {
    /// Centre of the sprite in world units.
    pub position: [f32; 2],
    /// Width and height in world units; a negative side flips the art.
    pub size: [f32; 2],
    /// `[u_min, v_min, u_max, v_max]` of the art in the atlas ([`SpriteRegion::uv`]).
    pub uv: [f32; 4],
    /// Radians, clockwise on screen.
    pub rotation: f32,
    /// Atlas texture array layer ([`SpriteRegion::layer`]).
    pub layer: u32,
    /// RGBA tint, straight alpha. `[255; 4]` draws the art as is.
    pub color: [u8; 4],
    /// Free per-sprite value for effect shaders (progress, seed...), unused by the default one.
    pub param: f32,
}

const _: () = assert!(std::mem::size_of::<SpriteInstance>() == 48);

impl SpriteInstance {
    /// The art in `region`, centred on `position`, at `scale` times its pixel size.
    #[inline]
    pub fn new(region: &SpriteRegion, position: Vec2, scale: f32, rotation: f32) -> Self {
        Self {
            position: position.into(),
            size: (region.size * scale).into(),
            uv: region.uv,
            rotation,
            layer: region.layer,
            color: [255; 4],
            param: 0.0,
        }
    }

    /// Same, with an explicit size in world units.
    #[inline]
    pub fn sized(region: &SpriteRegion, position: Vec2, size: Vec2, rotation: f32) -> Self {
        Self {
            position: position.into(),
            size: size.into(),
            uv: region.uv,
            rotation,
            layer: region.layer,
            color: [255; 4],
            param: 0.0,
        }
    }

    #[inline]
    pub fn with_color(mut self, rgba: [u8; 4]) -> Self {
        self.color = rgba;
        self
    }
}

/// Number of draw layers. Layer 0 is drawn first (furthest back).
pub const LAYERS: usize = 256;
const BUCKETS: usize = LAYERS * 2;

/// A frame's worth of sprites, kept in order without sorting.
///
/// Sprites go into one bucket per (draw layer, blend mode). Buckets are drawn layer by layer,
/// alpha before additive within a layer, and within a bucket in the order they were added.
/// Buckets keep their memory between frames, so a steady scene allocates nothing.
pub struct SpriteBatch {
    buckets: Vec<Vec<SpriteInstance>>,
    used: [u64; BUCKETS / 64],
}

impl Default for SpriteBatch {
    fn default() -> Self {
        Self::new()
    }
}

/// A contiguous range of the uploaded instances drawn with one blend mode: one draw call.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DrawRun {
    pub blend: Blend,
    pub start: u32,
    pub count: u32,
}

impl SpriteBatch {
    pub fn new() -> Self {
        Self { buckets: (0..BUCKETS).map(|_| Vec::new()).collect(), used: [0; BUCKETS / 64] }
    }

    #[inline]
    fn bucket(&mut self, layer: u8, blend: Blend) -> &mut Vec<SpriteInstance> {
        let idx = usize::from(layer) * 2 + blend as usize;
        self.used[idx / 64] |= 1 << (idx % 64);
        &mut self.buckets[idx]
    }

    /// Adds one sprite.
    #[inline]
    pub fn push(&mut self, layer: u8, blend: Blend, sprite: SpriteInstance) {
        self.bucket(layer, blend).push(sprite);
    }

    /// Adds many sprites.
    pub fn extend(&mut self, layer: u8, blend: Blend, sprites: &[SpriteInstance]) {
        self.bucket(layer, blend).extend_from_slice(sprites);
    }

    /// Makes room for `count` sprites and returns them to fill in, e.g. from several threads
    /// with `rayon`'s `par_iter_mut`. Fill every one: they start zeroed (invisible).
    pub fn alloc(&mut self, layer: u8, blend: Blend, count: usize) -> &mut [SpriteInstance] {
        let bucket = self.bucket(layer, blend);
        let start = bucket.len();
        bucket.resize(start + count, SpriteInstance::default());
        &mut bucket[start..]
    }

    /// Empties every bucket, keeping the memory.
    pub fn clear(&mut self) {
        for (word, bits) in self.used.iter().enumerate() {
            let mut b = *bits;
            while b != 0 {
                self.buckets[word * 64 + b.trailing_zeros() as usize].clear();
                b &= b - 1;
            }
        }
        self.used = [0; BUCKETS / 64];
    }

    /// Sprites in the batch.
    pub fn len(&self) -> usize {
        self.used_indices().map(|i| self.buckets[i].len()).sum()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    fn used_indices(&self) -> impl Iterator<Item = usize> + '_ {
        self.used.iter().enumerate().flat_map(|(word, &bits)| {
            let mut b = bits;
            std::iter::from_fn(move || {
                if b == 0 {
                    return None;
                }
                let tz = b.trailing_zeros() as usize;
                b &= b - 1;
                Some(word * 64 + tz)
            })
        })
    }

    /// The non-empty buckets in draw order, with their blend mode.
    pub fn buckets(&self) -> impl Iterator<Item = (Blend, &[SpriteInstance])> + '_ {
        self.used_indices().filter(|&i| !self.buckets[i].is_empty()).map(|i| {
            let blend = if i % 2 == 0 { Blend::Alpha } else { Blend::Additive };
            (blend, self.buckets[i].as_slice())
        })
    }

    /// The draw calls for this batch: buckets laid end to end, merging neighbours that share
    /// a blend mode.
    pub fn runs(&self, out: &mut Vec<DrawRun>) {
        out.clear();
        let mut start = 0u32;
        for (blend, sprites) in self.buckets() {
            let count = sprites.len() as u32;
            match out.last_mut() {
                Some(run) if run.blend == blend => run.count += count,
                _ => out.push(DrawRun { blend, start, count }),
            }
            start += count;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(x: f32) -> SpriteInstance {
        SpriteInstance { position: [x, 0.0], ..Default::default() }
    }

    #[test]
    fn draw_order_is_layer_then_alpha_then_additive() {
        let mut b = SpriteBatch::new();
        b.push(5, Blend::Additive, s(1.0));
        b.push(5, Blend::Alpha, s(2.0));
        b.push(0, Blend::Alpha, s(3.0));
        b.push(200, Blend::Alpha, s(4.0));
        let order: Vec<f32> = b.buckets().flat_map(|(_, v)| v.iter().map(|i| i.position[0])).collect();
        assert_eq!(order, vec![3.0, 2.0, 1.0, 4.0]);
    }

    #[test]
    fn runs_merge_same_blend_neighbours() {
        let mut b = SpriteBatch::new();
        b.extend(1, Blend::Alpha, &[s(0.0); 10]);
        b.extend(2, Blend::Alpha, &[s(0.0); 5]);
        b.extend(2, Blend::Additive, &[s(0.0); 7]);
        b.extend(3, Blend::Additive, &[s(0.0); 3]);
        b.extend(4, Blend::Alpha, &[s(0.0); 1]);
        let mut runs = Vec::new();
        b.runs(&mut runs);
        assert_eq!(
            runs,
            vec![
                DrawRun { blend: Blend::Alpha, start: 0, count: 15 },
                DrawRun { blend: Blend::Additive, start: 15, count: 10 },
                DrawRun { blend: Blend::Alpha, start: 25, count: 1 },
            ]
        );
        assert_eq!(b.len(), 26);
    }

    #[test]
    fn clear_keeps_capacity_and_alloc_fills() {
        let mut b = SpriteBatch::new();
        let slots = b.alloc(7, Blend::Alpha, 1000);
        slots.iter_mut().enumerate().for_each(|(i, x)| x.position[0] = i as f32);
        assert_eq!(b.len(), 1000);
        b.clear();
        assert!(b.is_empty());
        assert!(b.buckets[14].capacity() >= 1000);
        assert_eq!(b.buckets().count(), 0);
    }
}
