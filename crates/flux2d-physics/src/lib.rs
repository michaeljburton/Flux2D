//! Flux2D collision: a spatial hash grid for very many circles and boxes.
//!
//! The game fills a [`CollisionGrid`] with every collider once per fixed step
//! ([`clear`](CollisionGrid::clear), [`insert`](CollisionGrid::insert),
//! [`build`](CollisionGrid::build)), then asks it questions: what overlaps this shape
//! ([`query`](CollisionGrid::query)), which pairs overlap ([`pairs`](CollisionGrid::pairs)),
//! what does this bolt hit first ([`raycast`](CollisionGrid::raycast)).
//!
//! Colliders carry Godot-style collision **layers** (what the collider is) and **masks** (what
//! it looks for), as 32-bit sets. Rebuilding is a counting sort into flat arrays: no per-collider
//! allocation, nothing kept between steps but the arrays' memory. After `build` the grid is
//! read-only and `Sync`, so queries can run on many threads at once (e.g. one per bolt with
//! `rayon`).

use glam::Vec2;

/// A collider's shape, centred on its position.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Shape {
    Circle {
        radius: f32,
    },
    /// An axis-aligned box with half its width and height.
    Rect {
        half: Vec2,
    },
}

impl Shape {
    pub fn circle(radius: f32) -> Self {
        Shape::Circle { radius }
    }

    pub fn rect(width: f32, height: f32) -> Self {
        Shape::Rect { half: Vec2::new(width, height) * 0.5 }
    }

    /// Half the size of the box around the shape.
    pub fn half_extents(&self) -> Vec2 {
        match *self {
            Shape::Circle { radius } => Vec2::splat(radius),
            Shape::Rect { half } => half,
        }
    }
}

/// One thing that can collide. Plain data: copy it in every step.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Collider {
    pub pos: Vec2,
    pub shape: Shape,
    /// The layers this collider is on (Godot's `collision_layer`).
    pub layer: u32,
    /// The layers this collider looks for (Godot's `collision_mask`).
    pub mask: u32,
    /// The game's own id for it: an entity index, an array slot...
    pub user: u64,
}

impl Collider {
    pub fn new(pos: Vec2, shape: Shape, layer: u32, mask: u32, user: u64) -> Self {
        Self { pos, shape, layer, mask, user }
    }
}

/// Whether two shapes at two positions overlap (touching counts).
pub fn overlaps(a_pos: Vec2, a: Shape, b_pos: Vec2, b: Shape) -> bool {
    match (a, b) {
        (Shape::Circle { radius: ra }, Shape::Circle { radius: rb }) => {
            a_pos.distance_squared(b_pos) <= (ra + rb) * (ra + rb)
        }
        (Shape::Rect { half: ha }, Shape::Rect { half: hb }) => {
            let d = (a_pos - b_pos).abs();
            d.x <= ha.x + hb.x && d.y <= ha.y + hb.y
        }
        (Shape::Circle { radius }, Shape::Rect { half }) => circle_rect(a_pos, radius, b_pos, half),
        (Shape::Rect { half }, Shape::Circle { radius }) => circle_rect(b_pos, radius, a_pos, half),
    }
}

fn circle_rect(c: Vec2, r: f32, b: Vec2, half: Vec2) -> bool {
    let closest = c.clamp(b - half, b + half);
    c.distance_squared(closest) <= r * r
}

/// The first thing a ray hit.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RayHit {
    /// Index of the collider in the grid ([`CollisionGrid::get`]).
    pub index: u32,
    /// How far along the ray, 0 (start) to 1 (end).
    pub t: f32,
    /// Where the ray (its centre line, for a thick ray) was when it touched.
    pub point: Vec2,
    /// Surface normal at the hit, pointing back towards the ray. Zero if the ray started inside.
    pub normal: Vec2,
}

#[derive(Debug, Clone, Copy)]
struct Entry {
    index: u32,
    cx: i32,
    cy: i32,
}

/// A spatial hash grid of [`Collider`]s, rebuilt every step.
#[derive(Debug, Clone)]
pub struct CollisionGrid {
    cell: f32,
    inv_cell: f32,
    colliders: Vec<Collider>,
    /// Cell range of each collider: `[min_x, min_y, max_x, max_y]`, inclusive.
    ranges: Vec<[i32; 4]>,
    /// Bucket `b`'s entries are `entries[starts[b]..starts[b + 1]]`.
    starts: Vec<u32>,
    entries: Vec<Entry>,
    table_mask: usize,
    built: bool,
}

/// Cells one collider may cover before it is clamped (a huge collider would otherwise fill the
/// table with entries). Colliders bigger than `64 x 64` cells should use a coarser grid.
const MAX_CELLS_PER_SIDE: i32 = 64;

impl CollisionGrid {
    /// A grid with square cells of `cell_size` world units. A good size is about the diameter
    /// of the common colliders (ships, not the largest capital ship).
    pub fn new(cell_size: f32) -> Self {
        assert!(cell_size > 0.0, "cell size must be positive");
        Self {
            cell: cell_size,
            inv_cell: 1.0 / cell_size,
            colliders: Vec::new(),
            ranges: Vec::new(),
            starts: Vec::new(),
            entries: Vec::new(),
            table_mask: 0,
            built: false,
        }
    }

    pub fn cell_size(&self) -> f32 {
        self.cell
    }

    /// Removes every collider, keeping the memory.
    pub fn clear(&mut self) {
        self.colliders.clear();
        self.ranges.clear();
        self.entries.clear();
        self.built = false;
    }

    /// Adds a collider and returns its index. Call [`build`](Self::build) before querying.
    pub fn insert(&mut self, collider: Collider) -> u32 {
        self.built = false;
        let half = collider.shape.half_extents();
        let lo = self.cell_of(collider.pos - half);
        let mut hi = self.cell_of(collider.pos + half);
        hi.0 = hi.0.min(lo.0 + MAX_CELLS_PER_SIDE - 1);
        hi.1 = hi.1.min(lo.1 + MAX_CELLS_PER_SIDE - 1);
        self.colliders.push(collider);
        self.ranges.push([lo.0, lo.1, hi.0, hi.1]);
        self.colliders.len() as u32 - 1
    }

    pub fn len(&self) -> usize {
        self.colliders.len()
    }

    pub fn is_empty(&self) -> bool {
        self.colliders.is_empty()
    }

    /// The collider at `index`.
    pub fn get(&self, index: u32) -> &Collider {
        &self.colliders[index as usize]
    }

    pub fn colliders(&self) -> &[Collider] {
        &self.colliders
    }

    /// Sorts the colliders into cells. Queries need this after the last insert.
    pub fn build(&mut self) {
        let cells: usize = self.ranges.iter().map(|r| ((r[2] - r[0] + 1) * (r[3] - r[1] + 1)) as usize).sum();
        let table = (cells * 2).next_power_of_two().max(64);
        self.table_mask = table - 1;
        self.starts.clear();
        self.starts.resize(table + 1, 0);
        // Count, prefix-sum, fill: a counting sort by bucket.
        for r in &self.ranges {
            for cy in r[1]..=r[3] {
                for cx in r[0]..=r[2] {
                    self.starts[hash(cx, cy, self.table_mask) + 1] += 1;
                }
            }
        }
        for b in 0..table {
            self.starts[b + 1] += self.starts[b];
        }
        self.entries.clear();
        self.entries.resize(cells, Entry { index: 0, cx: 0, cy: 0 });
        // `cursor[b]` reuses the space after the counts: write position per bucket.
        let mut cursor: Vec<u32> = self.starts[..table].to_vec();
        for (i, r) in self.ranges.iter().enumerate() {
            for cy in r[1]..=r[3] {
                for cx in r[0]..=r[2] {
                    let b = hash(cx, cy, self.table_mask);
                    self.entries[cursor[b] as usize] = Entry { index: i as u32, cx, cy };
                    cursor[b] += 1;
                }
            }
        }
        self.built = true;
    }

    #[inline]
    fn cell_of(&self, p: Vec2) -> (i32, i32) {
        ((p.x * self.inv_cell).floor() as i32, (p.y * self.inv_cell).floor() as i32)
    }

    /// The entries stored for cell `(cx, cy)`.
    #[inline]
    fn cell_entries(&self, cx: i32, cy: i32) -> impl Iterator<Item = u32> + '_ {
        let b = hash(cx, cy, self.table_mask);
        self.entries[self.starts[b] as usize..self.starts[b + 1] as usize]
            .iter()
            .filter(move |e| e.cx == cx && e.cy == cy)
            .map(|e| e.index)
    }

    fn assert_built(&self) {
        assert!(self.built, "CollisionGrid: call build() after inserting, before querying");
    }

    /// Calls `found(index, collider)` once for every collider on a layer in `mask` that
    /// overlaps `shape` at `pos`.
    pub fn query(&self, pos: Vec2, shape: Shape, mask: u32, mut found: impl FnMut(u32, &Collider)) {
        self.assert_built();
        let half = shape.half_extents();
        let (lx, ly) = self.cell_of(pos - half);
        let (hx, hy) = self.cell_of(pos + half);
        for cy in ly..=hy {
            for cx in lx..=hx {
                for i in self.cell_entries(cx, cy) {
                    let c = &self.colliders[i as usize];
                    if c.layer & mask == 0 {
                        continue;
                    }
                    // Report each collider only from the first cell it shares with the query.
                    let r = self.ranges[i as usize];
                    if cx != r[0].max(lx) || cy != r[1].max(ly) {
                        continue;
                    }
                    if overlaps(pos, shape, c.pos, c.shape) {
                        found(i, c);
                    }
                }
            }
        }
    }

    /// Whether anything on a layer in `mask` overlaps `shape` at `pos`.
    pub fn any(&self, pos: Vec2, shape: Shape, mask: u32) -> bool {
        let mut hit = false;
        self.query(pos, shape, mask, |_, _| hit = true);
        hit
    }

    /// Every overlapping pair `(a, b)` with `a < b` where either one's mask includes the other's
    /// layer, written to `out` (cleared first).
    pub fn pairs(&self, out: &mut Vec<(u32, u32)>) {
        self.assert_built();
        out.clear();
        for (a, ca) in self.colliders.iter().enumerate() {
            let a = a as u32;
            let ra = self.ranges[a as usize];
            for cy in ra[1]..=ra[3] {
                for cx in ra[0]..=ra[2] {
                    for b in self.cell_entries(cx, cy) {
                        if b <= a {
                            continue;
                        }
                        let cb = &self.colliders[b as usize];
                        if ca.mask & cb.layer == 0 && cb.mask & ca.layer == 0 {
                            continue;
                        }
                        let rb = self.ranges[b as usize];
                        if cx != ra[0].max(rb[0]) || cy != ra[1].max(rb[1]) {
                            continue;
                        }
                        if overlaps(ca.pos, ca.shape, cb.pos, cb.shape) {
                            out.push((a, b));
                        }
                    }
                }
            }
        }
    }

    /// The first collider on a layer in `mask` that a ray from `from` to `to` touches.
    /// `radius` > 0 makes it a thick ray (a bolt with a size). A ray starting inside a collider
    /// hits it at `t = 0`.
    pub fn raycast(&self, from: Vec2, to: Vec2, radius: f32, mask: u32) -> Option<RayHit> {
        self.raycast_filtered(from, to, radius, mask, |_, _| true)
    }

    /// [`raycast`](Self::raycast) that skips colliders `accept(index, collider)` turns down
    /// (the shooter itself, things already hit...).
    pub fn raycast_filtered(
        &self,
        from: Vec2,
        to: Vec2,
        radius: f32,
        mask: u32,
        mut accept: impl FnMut(u32, &Collider) -> bool,
    ) -> Option<RayHit> {
        self.assert_built();
        let delta = to - from;
        let len = delta.length();
        let mut best: Option<RayHit> = None;
        let mut test_cell = |cx: i32, cy: i32, best: &mut Option<RayHit>| {
            for i in self.cell_entries(cx, cy) {
                let c = &self.colliders[i as usize];
                if c.layer & mask == 0 {
                    continue;
                }
                if let Some((t, normal)) = ray_shape(from, delta, radius, c.pos, c.shape)
                    && best.is_none_or(|b| t < b.t)
                    && accept(i, c)
                {
                    *best = Some(RayHit { index: i, t, point: from + delta * t, normal });
                }
            }
        };
        // Neighbouring cells a thick ray can reach.
        let reach = (radius * self.inv_cell).ceil() as i32;
        let mut visit = |cx: i32, cy: i32, best: &mut Option<RayHit>| {
            for oy in -reach..=reach {
                for ox in -reach..=reach {
                    test_cell(cx + ox, cy + oy, best);
                }
            }
        };

        // Walk the cells the centre line crosses (Amanatides & Woo).
        let (mut cx, mut cy) = self.cell_of(from);
        let (ex, ey) = self.cell_of(to);
        let step_x = if delta.x > 0.0 { 1 } else { -1 };
        let step_y = if delta.y > 0.0 { 1 } else { -1 };
        let next_boundary = |c: i32, step: i32| (c + i32::from(step > 0)) as f32 * self.cell;
        let t_delta_x = if delta.x != 0.0 { self.cell / delta.x.abs() } else { f32::INFINITY };
        let t_delta_y = if delta.y != 0.0 { self.cell / delta.y.abs() } else { f32::INFINITY };
        let mut t_max_x =
            if delta.x != 0.0 { (next_boundary(cx, step_x) - from.x) / delta.x } else { f32::INFINITY };
        let mut t_max_y =
            if delta.y != 0.0 { (next_boundary(cy, step_y) - from.y) / delta.y } else { f32::INFINITY };
        let margin = if len > 0.0 { radius / len } else { 0.0 };
        let mut t_enter = 0.0f32;
        let max_steps = (ex - cx).unsigned_abs() + (ey - cy).unsigned_abs() + 1;
        for _ in 0..max_steps {
            if best.is_some_and(|b| t_enter > b.t + margin) {
                break;
            }
            visit(cx, cy, &mut best);
            if cx == ex && cy == ey {
                break;
            }
            if t_max_x < t_max_y {
                t_enter = t_max_x;
                t_max_x += t_delta_x;
                cx += step_x;
            } else {
                t_enter = t_max_y;
                t_max_y += t_delta_y;
                cy += step_y;
            }
        }
        best
    }
}

#[inline]
fn hash(cx: i32, cy: i32, mask: usize) -> usize {
    ((cx as u32).wrapping_mul(0x9E37_79B1) ^ (cy as u32).wrapping_mul(0x85EB_CA77)) as usize & mask
}

/// Where along `from + delta * t` (t in 0..1) a ray of `radius` first touches a shape.
fn ray_shape(from: Vec2, delta: Vec2, radius: f32, pos: Vec2, shape: Shape) -> Option<(f32, Vec2)> {
    match shape {
        Shape::Circle { radius: r } => {
            let r = r + radius;
            let m = from - pos;
            let c = m.length_squared() - r * r;
            if c <= 0.0 {
                return Some((0.0, Vec2::ZERO));
            }
            let a = delta.length_squared();
            if a == 0.0 {
                return None;
            }
            let b = m.dot(delta);
            if b >= 0.0 {
                return None; // moving away
            }
            let disc = b * b - a * c;
            if disc < 0.0 {
                return None;
            }
            let t = (-b - disc.sqrt()) / a;
            (t <= 1.0).then(|| (t, (from + delta * t - pos).normalize_or_zero()))
        }
        Shape::Rect { half } => {
            // Slab test against the box grown by the ray's radius.
            let lo = pos - half - Vec2::splat(radius);
            let hi = pos + half + Vec2::splat(radius);
            if from.cmpge(lo).all() && from.cmple(hi).all() {
                return Some((0.0, Vec2::ZERO));
            }
            let mut t_in = 0.0f32;
            let mut t_out = 1.0f32;
            let mut normal = Vec2::ZERO;
            for axis in 0..2 {
                let (o, d, l, h) = (from[axis], delta[axis], lo[axis], hi[axis]);
                if d == 0.0 {
                    if o < l || o > h {
                        return None;
                    }
                    continue;
                }
                let (mut t0, mut t1) = ((l - o) / d, (h - o) / d);
                let n = -d.signum();
                if t0 > t1 {
                    std::mem::swap(&mut t0, &mut t1);
                }
                if t0 > t_in {
                    t_in = t0;
                    normal = Vec2::ZERO;
                    normal[axis] = n;
                }
                t_out = t_out.min(t1);
                if t_in > t_out {
                    return None;
                }
            }
            Some((t_in, normal))
        }
    }
}

/// Named collision layers, so game code reads `layers::ENEMY` rather than `1 << 2`.
///
/// These are only a suggestion; any `u32` bit set works.
pub mod layers {
    pub const PLAYER: u32 = 1 << 0;
    pub const ALLY: u32 = 1 << 1;
    pub const ENEMY: u32 = 1 << 2;
    pub const PLAYER_SHOT: u32 = 1 << 3;
    pub const ENEMY_SHOT: u32 = 1 << 4;
    pub const PICKUP: u32 = 1 << 5;
    pub const WORLD: u32 = 1 << 6;
    pub const ALL: u32 = u32::MAX;
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Rng(u64);
    impl Rng {
        fn f(&mut self) -> f32 {
            self.0 ^= self.0 << 13;
            self.0 ^= self.0 >> 7;
            self.0 ^= self.0 << 17;
            (self.0 >> 40) as f32 / (1u64 << 24) as f32
        }
        fn range(&mut self, lo: f32, hi: f32) -> f32 {
            lo + (hi - lo) * self.f()
        }
    }

    fn random_world(n: usize, seed: u64) -> CollisionGrid {
        let mut rng = Rng(seed);
        let mut g = CollisionGrid::new(32.0);
        for i in 0..n {
            let pos = Vec2::new(rng.range(-500.0, 500.0), rng.range(-500.0, 500.0));
            let shape = if rng.f() < 0.7 {
                Shape::circle(rng.range(2.0, 40.0))
            } else {
                Shape::rect(rng.range(4.0, 150.0), rng.range(4.0, 60.0))
            };
            let layer = 1 << (i % 4);
            let mask = if rng.f() < 0.5 { 0b0101 } else { 0b1010 };
            g.insert(Collider::new(pos, shape, layer, mask, i as u64));
        }
        g.build();
        g
    }

    #[test]
    fn shape_overlaps() {
        let c = Shape::circle(10.0);
        let r = Shape::rect(20.0, 10.0);
        assert!(overlaps(Vec2::ZERO, c, Vec2::new(19.0, 0.0), c));
        assert!(!overlaps(Vec2::ZERO, c, Vec2::new(21.0, 0.0), c));
        assert!(overlaps(Vec2::ZERO, r, Vec2::new(19.0, 0.0), r));
        assert!(!overlaps(Vec2::ZERO, r, Vec2::new(0.0, 11.0), r));
        // Circle near a box corner: inside the bounding boxes but not touching.
        assert!(!overlaps(Vec2::new(18.0, 13.0), c, Vec2::ZERO, r));
        assert!(overlaps(Vec2::new(15.0, 9.0), c, Vec2::ZERO, r));
    }

    #[test]
    fn query_matches_brute_force() {
        let g = random_world(600, 0x1234_5678);
        let mut rng = Rng(99);
        for _ in 0..200 {
            let pos = Vec2::new(rng.range(-550.0, 550.0), rng.range(-550.0, 550.0));
            let shape =
                if rng.f() < 0.5 { Shape::circle(rng.range(1.0, 80.0)) } else { Shape::rect(50.0, 120.0) };
            let mask = 0b0110;
            let mut found = Vec::new();
            g.query(pos, shape, mask, |i, _| found.push(i));
            let mut sorted = found.clone();
            sorted.sort_unstable();
            sorted.dedup();
            assert_eq!(sorted.len(), found.len(), "reported a collider twice");
            let brute: Vec<u32> = (0..g.len() as u32)
                .filter(|&i| {
                    let c = g.get(i);
                    c.layer & mask != 0 && overlaps(pos, shape, c.pos, c.shape)
                })
                .collect();
            assert_eq!(sorted, brute);
        }
    }

    #[test]
    fn pairs_match_brute_force() {
        let g = random_world(400, 0xDEAD_BEEF);
        let mut pairs = Vec::new();
        g.pairs(&mut pairs);
        let mut sorted = pairs.clone();
        sorted.sort_unstable();
        sorted.dedup();
        assert_eq!(sorted.len(), pairs.len(), "reported a pair twice");
        let mut brute = Vec::new();
        for a in 0..g.len() as u32 {
            for b in a + 1..g.len() as u32 {
                let (ca, cb) = (g.get(a), g.get(b));
                let interested = ca.mask & cb.layer != 0 || cb.mask & ca.layer != 0;
                if interested && overlaps(ca.pos, ca.shape, cb.pos, cb.shape) {
                    brute.push((a, b));
                }
            }
        }
        assert_eq!(sorted, brute);
        assert!(!brute.is_empty());
    }

    #[test]
    fn raycast_matches_brute_force() {
        let g = random_world(300, 0xC0FFEE);
        let mut rng = Rng(7);
        let mut hits = 0;
        for _ in 0..300 {
            let from = Vec2::new(rng.range(-600.0, 600.0), rng.range(-600.0, 600.0));
            let to = from + Vec2::from_angle(rng.range(0.0, 6.3)) * rng.range(10.0, 700.0);
            let radius = if rng.f() < 0.5 { 0.0 } else { rng.range(1.0, 50.0) };
            let mask = 0b0011;
            let got = g.raycast(from, to, radius, mask);
            let brute = (0..g.len() as u32)
                .filter(|&i| g.get(i).layer & mask != 0)
                .filter_map(|i| {
                    ray_shape(from, to - from, radius, g.get(i).pos, g.get(i).shape).map(|h| (i, h.0))
                })
                .min_by(|a, b| a.1.total_cmp(&b.1));
            match (got, brute) {
                (None, None) => {}
                (Some(h), Some((_, t))) => {
                    hits += 1;
                    assert!((h.t - t).abs() < 1e-4, "t {} vs {t}", h.t);
                }
                other => panic!("grid {:?} vs brute {:?} for {from} -> {to} r {radius}", other.0, other.1),
            }
        }
        assert!(hits > 30, "only {hits} hits; test is too easy");
    }

    #[test]
    fn ray_normals() {
        let mut g = CollisionGrid::new(16.0);
        g.insert(Collider::new(Vec2::new(100.0, 0.0), Shape::circle(10.0), 1, 0, 0));
        g.insert(Collider::new(Vec2::new(0.0, 100.0), Shape::rect(20.0, 20.0), 1, 0, 1));
        g.build();
        let h = g.raycast(Vec2::ZERO, Vec2::new(200.0, 0.0), 0.0, 1).unwrap();
        assert_eq!(h.index, 0);
        assert!((h.point.x - 90.0).abs() < 1e-3);
        assert!((h.normal - Vec2::new(-1.0, 0.0)).length() < 1e-4);
        let h = g.raycast(Vec2::ZERO, Vec2::new(0.0, 200.0), 0.0, 1).unwrap();
        assert_eq!(h.index, 1);
        assert!((h.point.y - 90.0).abs() < 1e-3);
        assert_eq!(h.normal, Vec2::new(0.0, -1.0));
        assert!(g.raycast(Vec2::ZERO, Vec2::new(0.0, 200.0), 0.0, 2).is_none(), "masked out");
        let skip_first = g.raycast_filtered(Vec2::ZERO, Vec2::new(200.0, 0.0), 0.0, 1, |i, _| i != 0);
        assert!(skip_first.is_none());
    }

    #[test]
    fn rebuild_reuses_memory() {
        let mut g = CollisionGrid::new(10.0);
        for step in 0..3 {
            g.clear();
            for i in 0..100 {
                g.insert(Collider::new(
                    Vec2::new(i as f32 * 3.0 + step as f32, 0.0),
                    Shape::circle(2.0),
                    1,
                    1,
                    i,
                ));
            }
            g.build();
            assert!(g.any(Vec2::new(30.0 + step as f32, 0.0), Shape::circle(0.5), 1));
        }
    }
}
