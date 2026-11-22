//! Flux2D effects: particles and trails, kept as plain data and drawn as sprites.
//!
//! - [`Particles`]: one pool per look (sparks, smoke, flashes...), each particle a 52-byte
//!   `Copy` struct in a `Vec`. Updating and drawing run on every core once a pool is big
//!   enough; dead particles are swapped out, so nothing allocates in a steady state.
//! - [`Trail`]: a fixed ring of recent positions (engine exhaust, bolt streaks) drawn as a
//!   tapering, fading ribbon of stretched sprites.
//!
//! Both draw with any [`SpriteRegion`]: a soft glow for additive sparks, the atlas's white
//! sprite for hard lines.

use flux2d_core::Rng;
use flux2d_render::{Blend, Space, SpriteBatch, SpriteInstance, SpriteRegion};
use glam::Vec2;
use rayon::prelude::*;

/// Pools at least this big update and draw in parallel.
const PARALLEL_FROM: usize = 4096;

/// One particle. Colours and sizes run from start to end over its life.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Particle {
    pub pos: Vec2,
    pub vel: Vec2,
    pub rotation: f32,
    pub spin: f32,
    /// Seconds lived so far.
    pub age: f32,
    /// Seconds it lives.
    pub life: f32,
    pub size_start: f32,
    pub size_end: f32,
    /// RGBA, straight alpha.
    pub color_start: [u8; 4],
    pub color_end: [u8; 4],
    /// Width / height of the drawn sprite (1 = square, 3 = a streak along the velocity).
    pub stretch: f32,
}

/// What a burst of particles looks like. Ranges are `(min, max)`, picked per particle.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Emitter {
    pub count: u32,
    /// Speed outwards, world units per second.
    pub speed: (f32, f32),
    /// Direction (radians, 0 = +x) and spread round it (radians; TAU = all round).
    pub angle: f32,
    pub spread: f32,
    /// How much of the source's velocity the particles keep (0..1).
    pub inherit: f32,
    pub life: (f32, f32),
    pub size_start: (f32, f32),
    pub size_end: (f32, f32),
    pub spin: (f32, f32),
    pub color_start: [u8; 4],
    pub color_end: [u8; 4],
    /// Particles start this far from the centre, at most.
    pub radius: f32,
    pub stretch: f32,
}

impl Default for Emitter {
    fn default() -> Self {
        Self {
            count: 16,
            speed: (50.0, 150.0),
            angle: 0.0,
            spread: std::f32::consts::TAU,
            inherit: 0.0,
            life: (0.4, 0.8),
            size_start: (8.0, 12.0),
            size_end: (0.0, 0.0),
            spin: (0.0, 0.0),
            color_start: [255; 4],
            color_end: [255, 255, 255, 0],
            radius: 0.0,
            stretch: 1.0,
        }
    }
}

/// A pool of particles that share one sprite, draw layer and blend mode.
#[derive(Debug, Clone)]
pub struct Particles {
    pub region: SpriteRegion,
    pub layer: u8,
    pub blend: Blend,
    /// Velocity lost per second, as a share (0 = none, 2 = slows quickly).
    pub drag: f32,
    /// Acceleration on every particle (world units / s²).
    pub gravity: Vec2,
    /// Most particles alive at once; extra spawns are dropped.
    pub max: usize,
    items: Vec<Particle>,
    rng: Rng,
}

impl Particles {
    pub fn new(region: SpriteRegion, layer: u8, blend: Blend) -> Self {
        Self {
            region,
            layer,
            blend,
            drag: 0.0,
            gravity: Vec2::ZERO,
            max: 200_000,
            items: Vec::new(),
            rng: Rng::new(0xF1A5),
        }
    }

    pub fn with_drag(mut self, drag: f32) -> Self {
        self.drag = drag;
        self
    }

    pub fn with_max(mut self, max: usize) -> Self {
        self.max = max;
        self
    }

    pub fn len(&self) -> usize {
        self.items.len()
    }

    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    pub fn clear(&mut self) {
        self.items.clear();
    }

    pub fn particles(&self) -> &[Particle] {
        &self.items
    }

    /// Adds one particle (dropped if the pool is full).
    #[inline]
    pub fn spawn(&mut self, p: Particle) {
        if self.items.len() < self.max {
            self.items.push(p);
        }
    }

    /// Spawns a burst at `pos`, for a source moving at `source_vel`.
    pub fn emit(&mut self, e: &Emitter, pos: Vec2, source_vel: Vec2) {
        let room = self.max.saturating_sub(self.items.len());
        let n = (e.count as usize).min(room);
        self.items.reserve(n);
        let rng = &mut self.rng;
        for _ in 0..n {
            let dir = Vec2::from_angle(e.angle + rng.range(-0.5, 0.5) * e.spread);
            let speed = rng.range(e.speed.0, e.speed.1);
            let offset = if e.radius > 0.0 { rng.direction() * rng.range(0.0, e.radius) } else { Vec2::ZERO };
            let vel = dir * speed + source_vel * e.inherit;
            self.items.push(Particle {
                pos: pos + offset,
                vel,
                rotation: if e.stretch != 1.0 {
                    vel.to_angle()
                } else {
                    rng.range(0.0, std::f32::consts::TAU)
                },
                spin: rng.range(e.spin.0, e.spin.1),
                age: 0.0,
                life: rng.range(e.life.0, e.life.1).max(1e-3),
                size_start: rng.range(e.size_start.0, e.size_start.1),
                size_end: rng.range(e.size_end.0, e.size_end.1),
                color_start: e.color_start,
                color_end: e.color_end,
                stretch: e.stretch,
            });
        }
    }

    /// Moves every particle on by `dt` seconds and removes the dead.
    pub fn update(&mut self, dt: f32) {
        let damp = (1.0 - self.drag * dt).clamp(0.0, 1.0);
        let gravity = self.gravity * dt;
        let step = |p: &mut Particle| {
            p.vel = p.vel * damp + gravity;
            p.pos += p.vel * dt;
            p.rotation += p.spin * dt;
            p.age += dt;
        };
        if self.items.len() >= PARALLEL_FROM {
            self.items.par_iter_mut().for_each(step);
        } else {
            self.items.iter_mut().for_each(step);
        }
        // Swap-remove the dead: order inside a pool doesn't matter.
        let mut i = 0;
        while i < self.items.len() {
            if self.items[i].age >= self.items[i].life {
                self.items.swap_remove(i);
            } else {
                i += 1;
            }
        }
    }

    /// Draws every particle into `batch` (world space). `lag` is how far behind the latest
    /// update to draw, in seconds: `(1 - alpha) * dt` matches interpolated game objects.
    pub fn draw(&self, batch: &mut SpriteBatch, lag: f32) {
        self.draw_in(batch, Space::World, lag);
    }

    pub fn draw_in(&self, batch: &mut SpriteBatch, space: Space, lag: f32) {
        if self.items.is_empty() {
            return;
        }
        let region = self.region;
        let out = batch.alloc_in(space, self.layer, self.blend, self.items.len());
        let fill = |(slot, p): (&mut SpriteInstance, &Particle)| {
            let t = (p.age / p.life).clamp(0.0, 1.0);
            let size = p.size_start + (p.size_end - p.size_start) * t;
            *slot = SpriteInstance::sized(
                &region,
                p.pos - p.vel * lag,
                Vec2::new(size * p.stretch, size),
                p.rotation,
            )
            .with_color(lerp_color(p.color_start, p.color_end, t));
        };
        if self.items.len() >= PARALLEL_FROM {
            out.par_iter_mut().zip(self.items.par_iter()).for_each(fill);
        } else {
            out.iter_mut().zip(self.items.iter()).for_each(fill);
        }
    }
}

/// Blends two RGBA colours, `t` = 0 gives `a`.
#[inline]
pub fn lerp_color(a: [u8; 4], b: [u8; 4], t: f32) -> [u8; 4] {
    let t = t.clamp(0.0, 1.0);
    std::array::from_fn(|i| (f32::from(a[i]) + (f32::from(b[i]) - f32::from(a[i])) * t).round() as u8)
}

/// Points a trail remembers.
pub const TRAIL_POINTS: usize = 24;

/// A ribbon following something: the last [`TRAIL_POINTS`] positions, oldest fading out.
///
/// Plain `Copy` data (about 200 bytes), so a component or an array of them costs no heap.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Trail {
    points: [Vec2; TRAIL_POINTS],
    /// Index of the newest point.
    head: usize,
    len: usize,
    /// A new point is kept only after moving this far.
    pub spacing: f32,
    /// Width at the head; the tail tapers to `width * taper`.
    pub width: f32,
    pub taper: f32,
    pub color_head: [u8; 4],
    pub color_tail: [u8; 4],
}

impl Trail {
    pub fn new(width: f32, color: [u8; 4]) -> Self {
        Self {
            points: [Vec2::ZERO; TRAIL_POINTS],
            head: 0,
            len: 0,
            spacing: 6.0,
            width,
            taper: 0.2,
            color_head: color,
            color_tail: [color[0], color[1], color[2], 0],
        }
    }

    pub fn with_spacing(mut self, spacing: f32) -> Self {
        self.spacing = spacing;
        self
    }

    pub fn len(&self) -> usize {
        self.len
    }

    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// Forgets every point (after a teleport or respawn).
    pub fn clear(&mut self) {
        self.len = 0;
    }

    /// The newest point first.
    pub fn points(&self) -> impl Iterator<Item = Vec2> + '_ {
        (0..self.len).map(move |i| self.points[(self.head + TRAIL_POINTS - i) % TRAIL_POINTS])
    }

    /// Records where the thing is now (call once per update). Moves the newest point instead
    /// of adding one until it is `spacing` away from the last kept point, so the trail's
    /// length in the world doesn't depend on speed.
    pub fn push(&mut self, pos: Vec2) {
        if self.len >= 2 {
            let prev = self.points[(self.head + TRAIL_POINTS - 1) % TRAIL_POINTS];
            if prev.distance_squared(pos) < self.spacing * self.spacing {
                self.points[self.head] = pos;
                return;
            }
        }
        if self.len > 0 {
            self.head = (self.head + 1) % TRAIL_POINTS;
        }
        self.points[self.head] = pos;
        self.len = (self.len + 1).min(TRAIL_POINTS);
    }

    /// Draws the ribbon from `head_pos` (where the thing is drawn this frame, so the trail
    /// stays attached between updates) back along the stored points.
    pub fn draw(
        &self,
        batch: &mut SpriteBatch,
        region: &SpriteRegion,
        layer: u8,
        blend: Blend,
        head_pos: Vec2,
    ) {
        if self.len < 2 {
            return;
        }
        let segments = self.len - 1;
        let mut prev = head_pos;
        for (i, p) in self.points().skip(1).enumerate() {
            let t = (i as f32 + 0.5) / segments as f32;
            let d = p - prev;
            let length = d.length();
            if length > 1e-3 {
                let width = self.width * (1.0 - t * (1.0 - self.taper));
                let mid = (p + prev) * 0.5;
                // Overlap segments by half a width so joints don't gap.
                let sprite =
                    SpriteInstance::sized(region, mid, Vec2::new(length + width * 0.5, width), d.to_angle())
                        .with_color(lerp_color(self.color_head, self.color_tail, t));
                batch.push(layer, blend, sprite);
            }
            prev = p;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn region() -> SpriteRegion {
        SpriteRegion { layer: 0, uv: [0.0, 0.0, 1.0, 1.0], size: Vec2::ONE }
    }

    #[test]
    fn particles_live_and_die() {
        let mut p = Particles::new(region(), 3, Blend::Additive).with_drag(1.0);
        let e = Emitter { count: 100, life: (0.5, 1.0), ..Default::default() };
        p.emit(&e, Vec2::new(10.0, 0.0), Vec2::ZERO);
        assert_eq!(p.len(), 100);
        let speed0: f32 = p.particles().iter().map(|q| q.vel.length()).sum();
        p.update(0.25);
        let speed1: f32 = p.particles().iter().map(|q| q.vel.length()).sum();
        assert!(speed1 < speed0, "drag slows them");
        let mut batch = SpriteBatch::new();
        p.draw(&mut batch, 0.0);
        assert_eq!(batch.len(), 100);
        for _ in 0..5 {
            p.update(0.25);
        }
        assert!(p.is_empty(), "all past their life");
    }

    #[test]
    fn pool_is_capped_and_big_pools_go_parallel() {
        let mut p = Particles::new(region(), 0, Blend::Alpha).with_max(10_000);
        let e = Emitter { count: 6_000, life: (1.0, 1.0), ..Default::default() };
        p.emit(&e, Vec2::ZERO, Vec2::ZERO);
        p.emit(&e, Vec2::ZERO, Vec2::ZERO);
        assert_eq!(p.len(), 10_000);
        p.update(0.5);
        let mut batch = SpriteBatch::new();
        p.draw(&mut batch, 0.0);
        let (_, _, sprites) = batch.buckets().next().unwrap();
        assert_eq!(sprites.len(), 10_000);
        // Half way through life: colour half way from opaque to clear.
        assert!((i32::from(sprites[0].color[3]) - 128).abs() <= 1);
    }

    #[test]
    fn trail_keeps_spacing_and_draws_segments() {
        let mut t = Trail::new(4.0, [255, 200, 100, 255]).with_spacing(10.0);
        for i in 0..100 {
            t.push(Vec2::new(i as f32, 0.0)); // 1 unit per step
        }
        // About one kept point per 10 units travelled, plus the moving head.
        let pts: Vec<Vec2> = t.points().collect();
        assert_eq!(t.len(), 12, "{pts:?}");
        assert!(pts.windows(2).all(|w| w[0].distance(w[1]) <= 10.0), "{pts:?}");
        assert_eq!(t.points().next(), Some(Vec2::new(99.0, 0.0)));
        for i in 100..1000 {
            t.push(Vec2::new(i as f32 * 5.0, 0.0));
        }
        assert_eq!(t.len(), TRAIL_POINTS);
        let mut batch = SpriteBatch::new();
        t.draw(&mut batch, &region(), 0, Blend::Additive, Vec2::new(5000.0, 0.0));
        assert_eq!(batch.len(), TRAIL_POINTS - 1);
        t.clear();
        assert!(t.is_empty());
    }

    #[test]
    fn colour_blend() {
        assert_eq!(lerp_color([0, 0, 0, 0], [255, 255, 255, 255], 0.5), [128; 4]);
        assert_eq!(lerp_color([10, 20, 30, 40], [0; 4], 0.0), [10, 20, 30, 40]);
    }
}
