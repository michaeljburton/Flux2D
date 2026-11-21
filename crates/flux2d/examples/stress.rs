//! Flux2D stress test and benchmark.
//!
//! Hundreds of thousands of moving, spinning, tinted sprites: ships (alpha blended) plus laser
//! bolts and glows (additive), using Cosmic Commando's art when it can find it.
//!
//! ```text
//! cargo run --release --example stress                      # interactive, 100,000 sprites
//! cargo run --release --example stress -- --sprites=500000
//! cargo run --release --example stress -- --bench --sprites=200000
//! cargo run --release --example stress -- --sweep           # 10k .. 1M, prints a results table
//! ```
//!
//! Options: `--sprites=N`, `--bench`, `--sweep[=N,N,...]`, `--frames=N` (measured frames, 600),
//! `--warmup=N` (120), `--vsync=on|off` (benchmarks default off), `--size=WxH`, `--static`
//! (sprites don't move: measures drawing only), `--zoom=Z` (start zoom, 2 = closer),
//! `--shot=out.png [--shot-frame=N]` (save frame N, default 30, then quit), `--art=DIR` (Cosmic Commando's
//! `cosmic-commando-2d/images` folder; also found next to this repo or via the
//! `COSMIC_COMMANDO_IMAGES` environment variable; without it, simple shapes are drawn instead).
//!
//! Keys: Up / Down = +/- 10,000 sprites, PageUp / PageDown = +/- 100,000, Space = pause,
//! V = vsync, mouse wheel = zoom, right or middle drag = pan, Home = reset camera, Esc = quit.

use flux2d::prelude::*;
use image::{Rgba, RgbaImage};
use rayon::prelude::*;
use std::path::{Path, PathBuf};
use std::time::Instant;

// ---------------------------------------------------------------------------------------------
// Options

#[derive(Debug, Clone)]
struct Options {
    sprites: usize,
    bench: bool,
    sweep: Vec<usize>,
    frames: usize,
    warmup: usize,
    vsync: Option<bool>,
    width: u32,
    height: u32,
    still: bool,
    art: Option<PathBuf>,
    shot: Option<PathBuf>,
    shot_frame: usize,
    zoom: f32,
}

impl Options {
    fn parse() -> Self {
        let mut o = Options {
            sprites: 100_000,
            bench: false,
            sweep: Vec::new(),
            frames: 600,
            warmup: 120,
            vsync: None,
            width: 1920,
            height: 1080,
            still: false,
            shot: None,
            shot_frame: 30,
            zoom: 1.0,
            art: std::env::var_os("COSMIC_COMMANDO_IMAGES").map(PathBuf::from),
        };
        for arg in std::env::args().skip(1) {
            let (key, value) = arg.split_once('=').unwrap_or((arg.as_str(), ""));
            let num = |v: &str| {
                v.replace(['_', ','], "").parse::<usize>().unwrap_or_else(|_| panic!("bad number in {arg}"))
            };
            match key {
                "--sprites" => o.sprites = num(value),
                "--bench" => o.bench = true,
                "--sweep" => {
                    o.sweep = if value.is_empty() {
                        vec![10_000, 50_000, 100_000, 200_000, 500_000, 1_000_000]
                    } else {
                        value.split(',').map(|v| v.trim().parse().expect("bad --sweep count")).collect()
                    }
                }
                "--frames" => o.frames = num(value),
                "--warmup" => o.warmup = num(value),
                "--vsync" => o.vsync = Some(matches!(value, "on" | "true" | "1" | "")),
                "--size" => {
                    let (w, h) = value.split_once('x').expect("--size=WxH");
                    o.width = w.parse().expect("--size width");
                    o.height = h.parse().expect("--size height");
                }
                "--static" => o.still = true,
                "--art" => o.art = Some(PathBuf::from(value)),
                "--shot" => o.shot = Some(PathBuf::from(value)),
                "--shot-frame" => o.shot_frame = num(value),
                "--zoom" => o.zoom = value.parse().expect("--zoom=Z"),
                "--help" | "-h" => {
                    println!(
                        "{}",
                        include_str!("stress.rs")
                            .lines()
                            .take_while(|l| l.starts_with("//!"))
                            .map(|l| l.trim_start_matches("//!").trim_start_matches(' '))
                            .collect::<Vec<_>>()
                            .join("\n")
                    );
                    std::process::exit(0);
                }
                _ => panic!("unknown option {arg} (try --help)"),
            }
        }
        o
    }

    fn benchmarking(&self) -> bool {
        self.bench || !self.sweep.is_empty()
    }
}

// ---------------------------------------------------------------------------------------------
// Art

/// The sprite art the stress test draws: ship kinds (alpha) plus a bolt and a glow (additive).
struct Art {
    ships: Vec<(SpriteRegion, f32, u32)>, // region, world size in px, spawn weight
    bolt: SpriteRegion,
    glow: SpriteRegion,
    source: String,
}

const SHIP_FILES: [(&str, f32, u32); 6] = [
    ("eniemes/fighter-eniemes-1.png", 44.0, 35),
    ("eniemes/drone-eniemes-1.png", 28.0, 35),
    ("eniemes/fighter-heavy-eniemes-1.png", 60.0, 15),
    ("escorts/fighter-human.png", 40.0, 10),
    ("player/commander-player-1.png", 64.0, 4),
    ("eniemes/destoryer-eniemes-1.png", 180.0, 1),
];

fn find_art_dir(explicit: Option<&Path>) -> Option<PathBuf> {
    if let Some(p) = explicit {
        return Some(p.to_path_buf());
    }
    let here = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../..");
    let roots = [PathBuf::from(".."), here];
    for root in roots {
        for name in ["cosmic-commando", "Cosmic-Commando"] {
            let dir = root.join(name).join("cosmic-commando-2d/images");
            if dir.join(SHIP_FILES[0].0).exists() {
                return Some(dir);
            }
        }
    }
    None
}

fn load_art(ctx: &mut Context, art_dir: Option<&Path>) -> (Art, Atlas) {
    let mut b = AtlasBuilder::new();
    let mut ships = Vec::new();
    let mut source = "built-in shapes".to_string();
    if let Some(dir) = find_art_dir(art_dir) {
        let mut ok = true;
        for (file, size, weight) in SHIP_FILES {
            match b.add_file(dir.join(file), Some(256)) {
                Ok(id) => ships.push((id, size, weight)),
                Err(e) => {
                    eprintln!("{e}; falling back to built-in shapes");
                    ok = false;
                    break;
                }
            }
        }
        if ok {
            source = format!("Cosmic Commando art from {}", dir.display());
        } else {
            b = AtlasBuilder::new();
            ships.clear();
        }
    }
    if ships.is_empty() {
        for (i, (size, weight)) in [(44.0, 40), (28.0, 40), (60.0, 20)].into_iter().enumerate() {
            ships.push((b.add(format!("ship {i}"), ship_shape(64, i)), size, weight));
        }
    }
    let bolt = b.add("bolt", glow_image(64, 16));
    let glow = b.add("glow", glow_image(64, 64));
    let atlas = b.build(&ctx.gpu).expect("atlas");
    let (layer, layers, mips) = atlas.dimensions();
    println!(
        "atlas: {} sprites in {layers} layer(s) of {layer}x{layer}, {mips} mip levels; {source}",
        atlas.len()
    );
    let art = Art {
        ships: ships.into_iter().map(|(id, s, w)| (atlas.region(id), s, w)).collect(),
        bolt: atlas.region(bolt),
        glow: atlas.region(glow),
        source,
    };
    (art, atlas)
}

/// A soft white glow: an ellipse filling the image, bright in the middle.
fn glow_image(w: u32, h: u32) -> RgbaImage {
    RgbaImage::from_fn(w, h, |x, y| {
        let dx = (x as f32 + 0.5) / w as f32 * 2.0 - 1.0;
        let dy = (y as f32 + 0.5) / h as f32 * 2.0 - 1.0;
        let d = (dx * dx + dy * dy).sqrt();
        let a = (1.0 - d).clamp(0.0, 1.0).powf(1.6);
        Rgba([255, 255, 255, (a * 255.0) as u8])
    })
}

/// A plain arrowhead ship, for when Cosmic Commando's art isn't around.
fn ship_shape(size: u32, kind: usize) -> RgbaImage {
    let colors = [[200, 60, 60], [200, 200, 80], [120, 160, 220]];
    let c = colors[kind % colors.len()];
    RgbaImage::from_fn(size, size, |x, y| {
        let u = (x as f32 + 0.5) / size as f32; // 0 = tail, 1 = nose (points right)
        let v = ((y as f32 + 0.5) / size as f32 - 0.5).abs() * 2.0;
        if v < 1.0 - u && u > 0.1 { Rgba([c[0], c[1], c[2], 255]) } else { Rgba([0, 0, 0, 0]) }
    })
}

// ---------------------------------------------------------------------------------------------
// Sprites

/// Small fast random numbers (xorshift64*); the same seed gives the same scene every run.
struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }
    fn f(&mut self) -> f32 {
        (self.next() >> 40) as f32 / (1u64 << 24) as f32
    }
    fn range(&mut self, lo: f32, hi: f32) -> f32 {
        lo + (hi - lo) * self.f()
    }
}

/// One kind of sprite, stored column by column so updates stream through memory.
#[derive(Default)]
struct Group {
    pos: Vec<Vec2>,
    vel: Vec<Vec2>,
    rot: Vec<f32>,
    spin: Vec<f32>,
    size: Vec<Vec2>,
    region: Vec<u16>,
    color: Vec<[u8; 4]>,
}

impl Group {
    fn len(&self) -> usize {
        self.pos.len()
    }
}

struct Scene {
    ships: Group,
    bolts: Group,
    glows: Group,
    regions: Vec<SpriteRegion>, // 0..ships = ship art, then bolt, glow
    half_world: Vec2,
}

const SHIP_SHARE: f32 = 0.50;
const BOLT_SHARE: f32 = 0.35;

impl Scene {
    fn new(art: &Art, count: usize, half_world: Vec2) -> Self {
        let mut rng = Rng(0x9E37_79B9_7F4A_7C15);
        let mut regions: Vec<SpriteRegion> = art.ships.iter().map(|s| s.0).collect();
        let bolt_idx = regions.len() as u16;
        regions.push(art.bolt);
        let glow_idx = regions.len() as u16;
        regions.push(art.glow);

        let n_ships = (count as f32 * SHIP_SHARE) as usize;
        let n_bolts = (count as f32 * BOLT_SHARE) as usize;
        let n_glows = count - n_ships - n_bolts;
        let total_weight: u32 = art.ships.iter().map(|s| s.2).sum();
        let pos = |rng: &mut Rng| {
            Vec2::new(rng.range(-half_world.x, half_world.x), rng.range(-half_world.y, half_world.y))
        };

        let mut ships = Group::default();
        for _ in 0..n_ships {
            let mut pick = (rng.next() % u64::from(total_weight)) as u32;
            let kind = art.ships.iter().position(|s| {
                if pick < s.2 {
                    true
                } else {
                    pick -= s.2;
                    false
                }
            });
            let kind = kind.unwrap_or(0);
            let (region, size, _) = art.ships[kind];
            let speed = rng.range(20.0, 140.0) * (44.0 / size).sqrt();
            let heading = rng.range(0.0, std::f32::consts::TAU);
            ships.pos.push(pos(&mut rng));
            ships.vel.push(Vec2::from_angle(heading) * speed);
            // The art's nose points up (-y): turn it a quarter so it leads along `heading`.
            ships.rot.push(heading + std::f32::consts::FRAC_PI_2);
            ships.spin.push(rng.range(-0.5, 0.5)); // turn rate: ships fly in arcs
            let aspect = region.size.y / region.size.x.max(1.0);
            ships.size.push(Vec2::new(size, size * aspect));
            ships.region.push(kind as u16);
            ships.color.push([255, 255, 255, 255]);
        }

        let palette = [[255, 70, 60, 255], [80, 170, 255, 255], [90, 255, 140, 255], [255, 200, 80, 255]];
        let mut bolts = Group::default();
        for _ in 0..n_bolts {
            let heading = rng.range(0.0, std::f32::consts::TAU);
            bolts.pos.push(pos(&mut rng));
            bolts.vel.push(Vec2::from_angle(heading) * rng.range(500.0, 1100.0));
            bolts.rot.push(heading);
            bolts.spin.push(0.0);
            bolts.size.push(Vec2::new(rng.range(22.0, 34.0), rng.range(5.0, 8.0)));
            bolts.region.push(bolt_idx);
            bolts.color.push(palette[(rng.next() % palette.len() as u64) as usize]);
        }

        let mut glows = Group::default();
        for _ in 0..n_glows {
            let d = rng.range(8.0, 40.0);
            glows.pos.push(pos(&mut rng));
            glows.vel.push(Vec2::new(rng.range(-30.0, 30.0), rng.range(-30.0, 30.0)));
            glows.rot.push(0.0);
            glows.spin.push(rng.range(1.0, 4.0)); // pulse speed
            glows.size.push(Vec2::splat(d));
            glows.region.push(glow_idx);
            let c = palette[(rng.next() % palette.len() as u64) as usize];
            glows.color.push([c[0], c[1], c[2], 150]);
        }
        Self { ships, bolts, glows, regions, half_world }
    }

    fn len(&self) -> usize {
        self.ships.len() + self.bolts.len() + self.glows.len()
    }

    fn update(&mut self, dt: f32) {
        let hw = self.half_world;
        let wrap = |p: &mut Vec2| {
            if p.x < -hw.x {
                p.x += 2.0 * hw.x
            } else if p.x > hw.x {
                p.x -= 2.0 * hw.x
            }
            if p.y < -hw.y {
                p.y += 2.0 * hw.y
            } else if p.y > hw.y {
                p.y -= 2.0 * hw.y
            }
        };
        for g in [&mut self.ships, &mut self.bolts, &mut self.glows] {
            g.pos.par_iter_mut().zip(g.vel.par_iter()).with_min_len(4096).for_each(|(p, v)| {
                *p += *v * dt;
                wrap(p);
            });
        }
        // Ships turn as they fly: heading and velocity rotate together.
        let ships = &mut self.ships;
        ships
            .rot
            .par_iter_mut()
            .zip(ships.vel.par_iter_mut())
            .zip(ships.spin.par_iter())
            .with_min_len(4096)
            .for_each(|((r, v), s)| {
                *r += s * dt;
                *v = Vec2::from_angle(s * dt).rotate(*v);
            });
    }

    fn draw(&self, batch: &mut SpriteBatch, back: f32, time: f32) {
        // `back` = seconds to step positions back for interpolation: drawing between updates.
        let regions = &self.regions;
        let fill = |g: &Group, out: &mut [SpriteInstance], pulse: bool| {
            out.par_iter_mut().enumerate().with_min_len(4096).for_each(|(i, s)| {
                let r = &regions[g.region[i] as usize];
                let mut size = g.size[i];
                let mut rot = g.rot[i];
                if pulse {
                    size *= 0.8 + 0.2 * (time * g.spin[i] + i as f32).sin();
                    rot = 0.0;
                }
                *s = SpriteInstance {
                    position: (g.pos[i] - g.vel[i] * back).into(),
                    size: size.into(),
                    uv: r.uv,
                    rotation: rot,
                    layer: r.layer,
                    color: g.color[i],
                    param: 0.0,
                };
            });
        };
        fill(&self.ships, batch.alloc(1, Blend::Alpha, self.ships.len()), false);
        fill(&self.bolts, batch.alloc(2, Blend::Additive, self.bolts.len()), false);
        fill(&self.glows, batch.alloc(2, Blend::Additive, self.glows.len()), true);
    }
}

// ---------------------------------------------------------------------------------------------
// The game

#[derive(Default)]
struct Totals {
    stats: FrameStats,
    update: f64,
    draw: f64,
    upload: f64,
    present: f64,
    draw_calls: u32,
    upload_bytes: u64,
    steps: u64,
}

impl Totals {
    /// Milliseconds per fixed update (a slow frame runs several updates, so per frame misleads).
    fn update_per_step(&self) -> f64 {
        if self.steps == 0 { 0.0 } else { self.update / self.steps as f64 }
    }
}

struct Stress {
    opts: Options,
    art: Art,
    scene: Scene,
    paused: bool,
    time: f32,
    // Benchmark state: which count of the sweep, frames seen at this count, results so far.
    counts: Vec<usize>,
    count_idx: usize,
    frames_at_count: usize,
    totals: Totals,
    results: Vec<(usize, Summary, Totals)>,
    // Title readout
    title_timer: Instant,
    title_stats: FrameStats,
    title_cpu: [f32; 4],
    dragging: Option<Vec2>,
}

impl Stress {
    fn new(ctx: &mut Context, opts: Options) -> Self {
        let (art, atlas) = load_art(ctx, opts.art.as_deref());
        ctx.set_atlas(&atlas);
        // `atlas` can be dropped: the renderer's bind group keeps the texture alive.
        let counts = if !opts.sweep.is_empty() { opts.sweep.clone() } else { vec![opts.sprites] };
        let half_world = ctx.window_size() * 0.5;
        let scene = Scene::new(&art, counts[0], half_world);
        println!("GPU: {}", ctx.gpu.adapter_name());
        ctx.camera.zoom = opts.zoom;
        Self {
            opts,
            art,
            scene,
            paused: false,
            time: 0.0,
            counts,
            count_idx: 0,
            frames_at_count: 0,
            totals: Totals::default(),
            results: Vec::new(),
            title_timer: Instant::now(),
            title_stats: FrameStats::new(),
            title_cpu: [0.0; 4],
            dragging: None,
        }
    }

    fn set_count(&mut self, ctx: &Context, count: usize) {
        let count = count.clamp(1_000, 5_000_000);
        self.scene = Scene::new(&self.art, count, ctx.window_size() * 0.5);
    }

    fn controls(&mut self, ctx: &mut Context) {
        let mut count = self.scene.len() as i64;
        if ctx.input.key_pressed(KeyCode::ArrowUp) {
            count += 10_000
        }
        if ctx.input.key_pressed(KeyCode::ArrowDown) {
            count -= 10_000
        }
        if ctx.input.key_pressed(KeyCode::PageUp) {
            count += 100_000
        }
        if ctx.input.key_pressed(KeyCode::PageDown) {
            count -= 100_000
        }
        if count != self.scene.len() as i64 {
            self.set_count(ctx, count.max(0) as usize);
        }
        if ctx.input.key_pressed(KeyCode::Space) {
            self.paused = !self.paused
        }
        if ctx.input.key_pressed(KeyCode::KeyV) {
            let on = !ctx.vsync();
            ctx.set_vsync(on);
        }
        if ctx.input.key_pressed(KeyCode::Escape) {
            ctx.exit()
        }
        if ctx.input.key_pressed(KeyCode::Home) {
            ctx.camera.position = Vec2::ZERO;
            ctx.camera.zoom = 1.0;
        }
        let wheel = ctx.input.wheel();
        if wheel != 0.0 {
            // Zoom about the pointer.
            let mouse = ctx.input.mouse_position();
            let before = ctx.camera.screen_to_world(mouse);
            ctx.camera.zoom = (ctx.camera.zoom * 1.15f32.powf(wheel)).clamp(0.05, 20.0);
            let after = ctx.camera.screen_to_world(mouse);
            ctx.camera.position += before - after;
        }
        let drag = ctx.input.mouse_down(MouseButton::Right) || ctx.input.mouse_down(MouseButton::Middle);
        if drag {
            let delta = ctx.input.mouse_delta();
            if self.dragging.is_some() {
                ctx.camera.position -= delta / ctx.camera.zoom;
            }
            self.dragging = Some(ctx.input.mouse_position());
        } else {
            self.dragging = None;
        }
    }

    fn bench_frame(&mut self, ctx: &mut Context) {
        let t = ctx.timings;
        self.frames_at_count += 1;
        if self.frames_at_count <= self.opts.warmup {
            return;
        }
        let tot = &mut self.totals;
        tot.stats.push(t.frame_ms);
        tot.update += f64::from(t.update_ms);
        tot.steps += u64::from(t.updates);
        tot.draw += f64::from(t.draw_ms);
        tot.upload += f64::from(t.upload_ms);
        tot.present += f64::from(t.present_ms);
        tot.draw_calls = t.upload.draw_calls;
        tot.upload_bytes = t.upload.bytes;
        if tot.stats.len() < self.opts.frames {
            return;
        }
        let totals = std::mem::take(&mut self.totals);
        let summary = totals.stats.summary().expect("frames measured");
        let n = self.scene.len();
        print_result(n, &summary, &totals);
        self.results.push((n, summary, totals));
        self.count_idx += 1;
        self.frames_at_count = 0;
        if let Some(&next) = self.counts.get(self.count_idx) {
            self.set_count(ctx, next);
        } else {
            self.print_table(ctx);
            ctx.exit();
        }
    }

    fn print_table(&self, ctx: &Context) {
        let size = ctx.window_size();
        println!();
        println!(
            "GPU: {} | {}x{} | vsync {} | {}",
            ctx.gpu.adapter_name(),
            size.x,
            size.y,
            if ctx.vsync() { "on" } else { "off" },
            self.art.source
        );
        println!();
        println!(
            "| Sprites | Avg frame (ms) | p95 (ms) | p99 (ms) | FPS | One update (ms) | Draw (ms) | Upload (ms) | Draw calls |"
        );
        println!("| ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: |");
        for (n, s, t) in &self.results {
            let f = s.frames as f64;
            println!(
                "| {} | {:.2} | {:.2} | {:.2} | {:.0} | {:.2} | {:.2} | {:.2} | {} |",
                thousands(*n),
                s.avg_ms,
                s.p95_ms,
                s.p99_ms,
                s.fps(),
                t.update_per_step(),
                t.draw / f,
                t.upload / f,
                t.draw_calls
            );
        }
    }
}

fn print_result(n: usize, s: &Summary, t: &Totals) {
    let f = s.frames as f64;
    println!(
        "FLUX2D SUMMARY sprites={n} frames={} avg_ms={:.2} p50_ms={:.2} p95_ms={:.2} p99_ms={:.2} max_ms={:.2} over_16.7ms={:.1}% fps={:.0} \
         update_ms={:.2} draw_ms={:.2} upload_ms={:.2} present_ms={:.2} draw_calls={} upload_mb={:.1}",
        s.frames,
        s.avg_ms,
        s.p50_ms,
        s.p95_ms,
        s.p99_ms,
        s.max_ms,
        s.over_60hz * 100.0,
        s.fps(),
        t.update_per_step(),
        t.draw / f,
        t.upload / f,
        t.present / f,
        t.draw_calls,
        t.upload_bytes as f64 / 1e6
    );
}

fn thousands(n: usize) -> String {
    let s = n.to_string();
    let mut out = String::new();
    for (i, c) in s.chars().enumerate() {
        if i > 0 && (s.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    out
}

impl Game for Stress {
    fn update(&mut self, _ctx: &mut Context, dt: f32) {
        if !self.paused && !self.opts.still {
            self.scene.update(dt);
            self.time += dt;
        }
    }

    fn draw(&mut self, ctx: &mut Context, batch: &mut SpriteBatch, alpha: f32) {
        // Controls run once per displayed frame: key presses are per-frame edges.
        if !self.opts.benchmarking() {
            self.controls(ctx);
        }
        let back = if self.paused || self.opts.still { 0.0 } else { (1.0 - alpha) / 60.0 };
        self.scene.draw(batch, back, self.time);
    }

    fn frame_end(&mut self, ctx: &mut Context) {
        if let Some(path) = self.opts.shot.clone() {
            self.frames_at_count += 1;
            if self.frames_at_count == self.opts.shot_frame {
                ctx.request_screenshot(path);
            } else if self.frames_at_count > self.opts.shot_frame {
                ctx.exit();
            }
            return;
        }
        if self.opts.benchmarking() {
            self.bench_frame(ctx);
            return;
        }
        let t = ctx.timings;
        self.title_stats.push(t.frame_ms);
        for (acc, v) in self.title_cpu.iter_mut().zip([t.update_ms, t.draw_ms, t.upload_ms, t.present_ms]) {
            *acc += v;
        }
        if self.title_timer.elapsed().as_secs_f32() >= 0.5 {
            let n = self.title_stats.len().max(1) as f32;
            let s = self.title_stats.summary().unwrap();
            let [u, d, up, p] = self.title_cpu.map(|v| v / n);
            ctx.window.set_title(&format!(
                "Flux2D stress | {} sprites | {:.2} ms ({:.0} fps) | update {u:.2} draw {d:.2} upload {up:.2} present {p:.2} | {} draw calls | vsync {} | {}",
                thousands(self.scene.len()), s.avg_ms, s.fps(), t.upload.draw_calls, if ctx.vsync() { "on" } else { "off" }, ctx.gpu.adapter_name()
            ));
            self.title_stats.clear();
            self.title_cpu = [0.0; 4];
            self.title_timer = Instant::now();
        }
    }
}

fn main() {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("warn")).init();
    let opts = Options::parse();
    let config = Config {
        title: "Flux2D stress".into(),
        width: opts.width,
        height: opts.height,
        vsync: opts.vsync.unwrap_or(!opts.benchmarking()),
        ..Default::default()
    };
    if opts.benchmarking() {
        println!("benchmark: {} warm-up + {} measured frames per count", opts.warmup, opts.frames);
    }
    if let Err(e) = flux2d::run(config, move |ctx| Stress::new(ctx, opts)) {
        eprintln!("error: {e}");
        std::process::exit(1);
    }
}
