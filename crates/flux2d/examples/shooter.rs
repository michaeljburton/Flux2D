//! Milestone 2's gate: a ship that flies, shoots and blows things up.
//!
//! Everything the engine grew in M2 in one small game: enemies are bevy_ecs entities moved by
//! systems, bolts are a packed array tested against a collision grid (on every core), and
//! explosions are particle bursts with synthesised sound. A TrueType HUD sits on top, and F3
//! shows the dev overlay. Uses Cosmic Commando's ship art when it can find it, plain shapes
//! otherwise.
//!
//! ```text
//! cargo run --release --example shooter
//! cargo run --release --example shooter -- --enemies=400          # a bigger fight
//! cargo run --release --example shooter -- --bench --enemies=2000  # autopilot, prints a summary
//! ```
//!
//! Options: `--enemies=N` (how many attack at once at wave 1, default 24), `--bench`
//! (autopilot, vsync off, 120 warm-up and 600 measured frames, then a `FLUX2D SUMMARY`),
//! `--frames=N`, `--autopilot`, `--overlay` (dev overlay from the start), `--no-audio`,
//! `--size=WxH`, `--shot=out.png [--shot-frame=N]` (autopilot, save frame N (300) and quit),
//! `--art=DIR` (Cosmic Commando's `cosmic-commando-2d/images`; also found next to this repo or
//! via `COSMIC_COMMANDO_IMAGES`), `--font=FILE` (a .ttf for the HUD; a system font otherwise).
//!
//! Controls: WASD / arrows / left stick to fly, mouse / right stick to aim, left mouse / Space /
//! right trigger to fire, wheel to zoom, F3 dev overlay, F1 collision shapes, P pause, Esc quit.

use flux2d::assets::{Assets, system_font};
use flux2d::ecs::prelude::*;
use flux2d::fx::Emitter;
use flux2d::physics::layers;
use flux2d::prelude::*;
use flux2d::render::text::LATIN;
use image::{Rgba, RgbaImage};
use rayon::prelude::*;
use std::f32::consts::{FRAC_PI_2, TAU};
use std::fmt::Write as _;
use std::path::PathBuf;

// ---------------------------------------------------------------------------------------------
// Options

#[derive(Debug, Clone)]
struct Options {
    enemies: usize,
    bench: bool,
    frames: usize,
    warmup: usize,
    autopilot: bool,
    overlay: bool,
    audio: bool,
    width: u32,
    height: u32,
    shot: Option<PathBuf>,
    shot_frame: usize,
    art: Option<PathBuf>,
    font: Option<PathBuf>,
}

impl Options {
    fn parse() -> Self {
        let mut o = Options {
            enemies: 24,
            bench: false,
            frames: 600,
            warmup: 120,
            autopilot: false,
            overlay: false,
            audio: true,
            width: 1600,
            height: 900,
            shot: None,
            shot_frame: 300,
            art: None,
            font: None,
        };
        for arg in std::env::args().skip(1) {
            let (key, value) = arg.split_once('=').unwrap_or((arg.as_str(), ""));
            let num = |v: &str| {
                v.replace(['_', ','], "").parse::<usize>().unwrap_or_else(|_| panic!("bad number in {arg}"))
            };
            match key {
                "--enemies" => o.enemies = num(value).max(1),
                "--bench" => o.bench = true,
                "--frames" => o.frames = num(value),
                "--warmup" => o.warmup = num(value),
                "--autopilot" => o.autopilot = true,
                "--overlay" => o.overlay = true,
                "--no-audio" => o.audio = false,
                "--size" => {
                    let (w, h) = value.split_once('x').expect("--size=WxH");
                    o.width = w.parse().expect("--size width");
                    o.height = h.parse().expect("--size height");
                }
                "--shot" => o.shot = Some(PathBuf::from(value)),
                "--shot-frame" => o.shot_frame = num(value),
                "--art" => o.art = Some(PathBuf::from(value)),
                "--font" => o.font = Some(PathBuf::from(value)),
                "--help" | "-h" => {
                    let doc: Vec<&str> = include_str!("shooter.rs")
                        .lines()
                        .take_while(|l| l.starts_with("//!"))
                        .map(|l| l.trim_start_matches("//!").trim_start_matches(' '))
                        .collect();
                    println!("{}", doc.join("\n"));
                    std::process::exit(0);
                }
                _ => panic!("unknown option {arg} (try --help)"),
            }
        }
        if o.bench || o.shot.is_some() {
            o.autopilot = true;
        }
        o
    }
}

// ---------------------------------------------------------------------------------------------
// Art

/// Enemy kinds: art, size in world units, hit points, top speed, score, shoots back.
#[derive(Debug, Clone, Copy)]
struct Kind {
    file: &'static str,
    size: f32,
    hp: f32,
    speed: f32,
    score: u32,
    shoots: bool,
    color: [u8; 3],
}

const KINDS: [Kind; 3] = [
    Kind {
        file: "eniemes/drone-eniemes-1.png",
        size: 34.0,
        hp: 1.0,
        speed: 260.0,
        score: 10,
        shoots: false,
        color: [230, 200, 80],
    },
    Kind {
        file: "eniemes/fighter-eniemes-1.png",
        size: 52.0,
        hp: 3.0,
        speed: 200.0,
        score: 25,
        shoots: true,
        color: [220, 70, 60],
    },
    Kind {
        file: "eniemes/fighter-heavy-eniemes-1.png",
        size: 78.0,
        hp: 10.0,
        speed: 120.0,
        score: 80,
        shoots: true,
        color: [170, 90, 220],
    },
];
const PLAYER_FILE: &str = "player/commander-player-1.png";
const PLAYER_SIZE: f32 = 64.0;

struct Art {
    player: SpriteRegion,
    enemies: [SpriteRegion; 3],
    bolt: SpriteRegion,
    glow: SpriteRegion,
    smoke: SpriteRegion,
    white: SpriteRegion,
    hud: Font,
    small: Font,
    source: String,
}

fn load_art(ctx: &mut Context, o: &Options) -> (Art, Atlas) {
    let mut b = AtlasBuilder::new();
    let images = match &o.art {
        Some(dir) => Some(Assets::new(dir)),
        None => Assets::locate(
            Some("COSMIC_COMMANDO_IMAGES"),
            &["cosmic-commando/cosmic-commando-2d/images", "Cosmic-Commando/cosmic-commando-2d/images"],
            PLAYER_FILE,
        ),
    };
    let mut ids = None;
    let mut source = "built-in shapes".to_string();
    if let Some(images) = &images {
        let files: Vec<PathBuf> = std::iter::once(PLAYER_FILE)
            .chain(KINDS.iter().map(|k| k.file))
            .map(|f| images.path(f))
            .collect();
        let loaded: Result<Vec<SpriteId>, _> = b.add_files(&files, Some(256)).into_iter().collect();
        match loaded {
            Ok(v) => {
                source = format!("Cosmic Commando art from {}", images.root().display());
                ids = Some(v);
            }
            Err(e) => eprintln!("{e}; using built-in shapes"),
        }
    }
    let ids = ids.unwrap_or_else(|| {
        let mut v = vec![b.add("player", ship_shape(96, [90, 200, 255], true))];
        v.extend(KINDS.iter().map(|k| b.add(k.file, ship_shape(96, k.color, false))));
        v
    });
    let bolt = b.add("bolt", glow_image(64, 16, 1.2));
    let glow = b.add("glow", glow_image(64, 64, 1.6));
    let smoke = b.add("smoke", glow_image(64, 64, 0.8));

    let font_path = o.font.clone().or_else(system_font);
    let fonts = font_path.as_ref().and_then(|p| {
        let big = b.add_font_file(p, 40.0, LATIN).map_err(|e| eprintln!("{e}")).ok()?;
        let small = b.add_font_file(p, 20.0, LATIN).ok()?;
        Some((big, small))
    });
    let atlas = b.build(&ctx.gpu).expect("atlas");
    ctx.set_atlas(&atlas);
    let (hud, small) = match fonts {
        Some((big, small)) => (atlas.font(big).clone(), atlas.font(small).clone()),
        None => (atlas.debug_font().clone(), atlas.debug_font().clone()),
    };
    println!(
        "art: {source}; HUD font: {}",
        font_path.as_deref().map_or("built-in 8x8".into(), |p| p.display().to_string())
    );
    let art = Art {
        player: atlas.region(ids[0]),
        enemies: [atlas.region(ids[1]), atlas.region(ids[2]), atlas.region(ids[3])],
        bolt: atlas.region(bolt),
        glow: atlas.region(glow),
        smoke: atlas.region(smoke),
        white: atlas.white(),
        hud,
        small,
        source,
    };
    (art, atlas)
}

/// A soft white ellipse filling the image.
fn glow_image(w: u32, h: u32, falloff: f32) -> RgbaImage {
    RgbaImage::from_fn(w, h, |x, y| {
        let dx = (x as f32 + 0.5) / w as f32 * 2.0 - 1.0;
        let dy = (y as f32 + 0.5) / h as f32 * 2.0 - 1.0;
        let a = (1.0 - (dx * dx + dy * dy).sqrt()).clamp(0.0, 1.0).powf(falloff);
        Rgba([255, 255, 255, (a * 255.0) as u8])
    })
}

/// A plain ship pointing up (-y), like Cosmic Commando's art.
fn ship_shape(size: u32, c: [u8; 3], player: bool) -> RgbaImage {
    RgbaImage::from_fn(size, size, |x, y| {
        let u = ((x as f32 + 0.5) / size as f32 - 0.5).abs() * 2.0; // 0 centre .. 1 edge
        let v = (y as f32 + 0.5) / size as f32; // 0 nose .. 1 tail
        let body = u < v * 0.9 && v < 0.92;
        let notch = v > 0.7 && u < (v - 0.7) * 1.2;
        let cockpit = player && u < 0.12 && (0.3..0.5).contains(&v);
        if cockpit {
            Rgba([230, 250, 255, 255])
        } else if body && !notch {
            let shade = 1.0 - u * 0.35;
            Rgba([
                (f32::from(c[0]) * shade) as u8,
                (f32::from(c[1]) * shade) as u8,
                (f32::from(c[2]) * shade) as u8,
                255,
            ])
        } else {
            Rgba([0, 0, 0, 0])
        }
    })
}

// ---------------------------------------------------------------------------------------------
// ECS: enemies are entities, moved by systems

#[derive(Component, Clone, Copy)]
struct Body {
    pos: Vec2,
    prev: Vec2,
    vel: Vec2,
    rot: f32,
    prev_rot: f32,
}

#[derive(Component)]
struct Enemy {
    kind: usize,
    hp: f32,
    cooldown: f32,
    /// Strafing side and preferred range, so fighters don't all stack on one point.
    side: f32,
    range: f32,
}

#[derive(Component)]
struct Exhaust(Trail);

/// What the systems need to know this step.
#[derive(Resource, Default)]
struct Step {
    dt: f32,
    player: Vec2,
    player_alive: bool,
}

/// Drones dive straight at the player; fighters and heavies circle at range.
fn steer(mut q: Query<(&mut Body, &Enemy)>, step: Res<Step>) {
    let dt = step.dt;
    for (mut body, enemy) in &mut q {
        let kind = &KINDS[enemy.kind];
        let to = step.player - body.pos;
        let dist = to.length().max(1.0);
        let dir = to / dist;
        let desired = if !step.player_alive {
            Vec2::from_angle(body.rot - FRAC_PI_2) * kind.speed * 0.5
        } else if kind.shoots {
            let tangent = dir.perp() * enemy.side;
            let radial = dir * ((dist - enemy.range) / 200.0).clamp(-1.0, 1.0);
            (radial + tangent * 0.8).normalize_or_zero() * kind.speed
        } else {
            dir * kind.speed
        };
        let turn = 2.5 * dt;
        body.vel = body.vel + (desired - body.vel) * turn;
    }
}

fn integrate(mut q: Query<&mut Body>, step: Res<Step>) {
    for mut b in &mut q {
        b.prev = b.pos;
        b.prev_rot = b.rot;
        let v = b.vel;
        b.pos += v * step.dt;
        if v.length_squared() > 1.0 {
            // The art's nose points up: face along the velocity.
            let target = v.to_angle() + FRAC_PI_2;
            b.rot = turn_towards(b.rot, target, 6.0 * step.dt);
        }
    }
}

fn exhaust(mut q: Query<(&Body, &mut Exhaust)>) {
    for (b, mut e) in &mut q {
        let tail = b.pos - Vec2::from_angle(b.rot - FRAC_PI_2) * 14.0;
        e.0.push(tail);
    }
}

fn turn_towards(from: f32, to: f32, max: f32) -> f32 {
    let d = (to - from + std::f32::consts::PI).rem_euclid(TAU) - std::f32::consts::PI;
    from + d.clamp(-max, max)
}

fn lerp_angle(a: f32, b: f32, t: f32) -> f32 {
    let d = (b - a + std::f32::consts::PI).rem_euclid(TAU) - std::f32::consts::PI;
    a + d * t
}

// ---------------------------------------------------------------------------------------------
// Packed data: bolts, the player, effects

#[derive(Clone, Copy)]
struct Bolt {
    pos: Vec2,
    vel: Vec2,
    life: f32,
    damage: f32,
    friendly: bool,
}

struct Player {
    body: Body,
    hull: f32,
    alive: bool,
    respawn: f32,
    invulnerable: f32,
    cooldown: f32,
    trail: Trail,
    aim: Vec2,
}

impl Player {
    fn new() -> Self {
        Self {
            body: Body { pos: Vec2::ZERO, prev: Vec2::ZERO, vel: Vec2::ZERO, rot: 0.0, prev_rot: 0.0 },
            hull: 100.0,
            alive: true,
            respawn: 0.0,
            invulnerable: 2.0,
            cooldown: 0.0,
            trail: Trail::new(10.0, [90, 190, 255, 200]).with_spacing(8.0),
            aim: Vec2::NEG_Y,
        }
    }
}

struct Effects {
    sparks: Particles,
    flashes: Particles,
    smoke: Particles,
    embers: Particles,
}

impl Effects {
    fn new(art: &Art) -> Self {
        Self {
            sparks: Particles::new(art.bolt, 60, Blend::Additive).with_drag(2.5),
            flashes: Particles::new(art.glow, 61, Blend::Additive),
            smoke: Particles::new(art.smoke, 20, Blend::Alpha).with_drag(1.5),
            embers: Particles::new(art.glow, 59, Blend::Additive).with_drag(1.2),
        }
    }

    fn len(&self) -> usize {
        self.sparks.len() + self.flashes.len() + self.smoke.len() + self.embers.len()
    }

    fn update(&mut self, dt: f32) {
        self.sparks.update(dt);
        self.flashes.update(dt);
        self.smoke.update(dt);
        self.embers.update(dt);
    }

    fn draw(&self, batch: &mut SpriteBatch, lag: f32) {
        self.smoke.draw(batch, lag);
        self.embers.draw(batch, lag);
        self.sparks.draw(batch, lag);
        self.flashes.draw(batch, lag);
    }

    /// A ship blowing up: a flash, a ring of streaking sparks, embers and a smoke cloud.
    fn explosion(&mut self, pos: Vec2, vel: Vec2, size: f32) {
        let s = size / 50.0;
        self.flashes.emit(
            &Emitter {
                count: 2,
                speed: (0.0, 10.0),
                life: (0.18, 0.28),
                size_start: (size * 2.5, size * 3.2),
                size_end: (size * 0.5, size * 0.8),
                color_start: [255, 230, 180, 255],
                color_end: [255, 120, 40, 0],
                ..Default::default()
            },
            pos,
            vel,
        );
        self.sparks.emit(
            &Emitter {
                count: (30.0 * s) as u32 + 10,
                speed: (200.0 * s.sqrt(), 650.0 * s.sqrt()),
                inherit: 0.4,
                life: (0.25, 0.6),
                size_start: (10.0, 16.0),
                size_end: (4.0, 6.0),
                color_start: [255, 220, 140, 255],
                color_end: [255, 80, 20, 0],
                stretch: 3.5,
                ..Default::default()
            },
            pos,
            vel,
        );
        self.embers.emit(
            &Emitter {
                count: (14.0 * s) as u32 + 4,
                speed: (40.0, 180.0 * s.sqrt()),
                inherit: 0.6,
                life: (0.6, 1.4),
                size_start: (6.0, 14.0),
                size_end: (0.0, 2.0),
                color_start: [255, 170, 60, 255],
                color_end: [200, 40, 10, 0],
                radius: size * 0.3,
                ..Default::default()
            },
            pos,
            vel,
        );
        self.smoke.emit(
            &Emitter {
                count: (10.0 * s) as u32 + 4,
                speed: (10.0, 70.0 * s.sqrt()),
                inherit: 0.5,
                life: (0.9, 1.8),
                size_start: (size * 0.6, size),
                size_end: (size * 1.6, size * 2.4),
                spin: (-1.0, 1.0),
                color_start: [90, 80, 80, 150],
                color_end: [40, 40, 50, 0],
                radius: size * 0.3,
                ..Default::default()
            },
            pos,
            vel,
        );
    }

    /// A bolt hitting something: a small spray back along the surface normal.
    fn hit(&mut self, pos: Vec2, normal: Vec2, friendly: bool) {
        let color = if friendly { [140, 255, 200, 255] } else { [255, 140, 120, 255] };
        let angle = if normal == Vec2::ZERO { 0.0 } else { normal.to_angle() };
        self.sparks.emit(
            &Emitter {
                count: 6,
                speed: (120.0, 380.0),
                angle,
                spread: 2.2,
                life: (0.1, 0.25),
                size_start: (6.0, 10.0),
                size_end: (2.0, 3.0),
                color_start: color,
                color_end: [color[0], color[1], color[2], 0],
                stretch: 3.0,
                ..Default::default()
            },
            pos,
            Vec2::ZERO,
        );
        self.flashes.emit(
            &Emitter {
                count: 1,
                speed: (0.0, 0.0),
                life: (0.08, 0.1),
                size_start: (40.0, 50.0),
                size_end: (10.0, 10.0),
                color_start: color,
                color_end: [color[0], color[1], color[2], 0],
                ..Default::default()
            },
            pos,
            Vec2::ZERO,
        );
    }
}

struct Sounds {
    laser: Sound,
    enemy_laser: Sound,
    booms: [Sound; 3],
    hit: Sound,
}

impl Sounds {
    fn new() -> Self {
        use flux2d::audio::synth;
        Self {
            laser: synth::laser(1500.0, 420.0, 0.12),
            enemy_laser: synth::laser(700.0, 240.0, 0.16),
            booms: [synth::explosion(0.7, 11), synth::explosion(0.9, 23), synth::explosion(1.2, 37)],
            hit: synth::hit(0.06),
        }
    }
}

/// Background stars in three parallax layers, tiled over a square.
struct Stars {
    stars: Vec<(Vec2, f32, f32, u8)>, // position in tile, size, parallax, brightness
}

const STAR_TILE: f32 = 2048.0;

impl Stars {
    fn new(rng: &mut Rng) -> Self {
        let stars = (0..900)
            .map(|i| {
                let depth = [0.15, 0.35, 0.7][i % 3];
                let p = Vec2::new(rng.range(0.0, STAR_TILE), rng.range(0.0, STAR_TILE));
                let size = rng.range(1.0, 2.5) * (0.6 + depth);
                (p, size, depth, (rng.range(90.0, 255.0) * (0.5 + depth * 0.7)).min(255.0) as u8)
            })
            .collect();
        Self { stars }
    }

    fn draw(&self, batch: &mut SpriteBatch, art: &Art, camera: &Camera2D) {
        let view = camera.visible_size();
        for &(p, size, depth, light) in &self.stars {
            // Each star repeats every STAR_TILE in "parallax space"; pick the copy nearest the
            // camera so the field covers any view.
            let cam = camera.position * depth;
            let rel = (p - cam).rem_euclid(Vec2::splat(STAR_TILE)) - Vec2::splat(STAR_TILE * 0.5);
            if rel.x.abs() > view.x * 0.5 + 8.0 || rel.y.abs() > view.y * 0.5 + 8.0 {
                continue;
            }
            let pos = camera.position + rel;
            let s = size / camera.zoom.max(0.25);
            batch.push(
                0,
                Blend::Additive,
                SpriteInstance::sized(&art.glow, pos, Vec2::splat(s * 2.5), 0.0)
                    .with_color([180, 200, 255, light]),
            );
        }
    }
}

// ---------------------------------------------------------------------------------------------
// The game

struct Shooter {
    o: Options,
    art: Art,
    world: World,
    schedule: Schedule,
    player: Player,
    bolts: Vec<Bolt>,
    hits: Vec<Option<RayHit>>,
    grid: CollisionGrid,
    fx: Effects,
    sounds: Sounds,
    stars: Stars,
    rng: Rng,
    score: u32,
    kills: u32,
    wave: u32,
    target: usize,
    time: f32,
    shake: f32,
    paused: bool,
    show_shapes: bool,
    frame: usize,
    stats: FrameStats,
    alive: usize,
    line: String,
    scratch_users: Vec<u64>,
    scratch_dead: Vec<(Entity, Body, usize, bool)>,
    _atlas: Atlas,
}

const PLAYER_SPEED: f32 = 520.0;
const FIRE_INTERVAL: f32 = 0.085;
const BOLT_SPEED: f32 = 1500.0;
const ENEMY_BOLT_SPEED: f32 = 700.0;
const PLAYER_RADIUS: f32 = 22.0;

impl Shooter {
    fn new(ctx: &mut Context, o: Options) -> Self {
        let (art, atlas) = load_art(ctx, &o);
        let mut world = World::new();
        world.insert_resource(Step::default());
        let mut schedule = Schedule::default();
        schedule.add_systems((steer, integrate, exhaust).chain());
        let mut rng = Rng::new(0xC05C0);
        let stars = Stars::new(&mut rng);
        ctx.overlay.visible = o.overlay || o.bench;
        let target = o.enemies;
        let fx = Effects::new(&art);
        let mut game = Self {
            art,
            world,
            schedule,
            player: Player::new(),
            bolts: Vec::new(),
            hits: Vec::new(),
            grid: CollisionGrid::new(64.0),
            fx,
            sounds: Sounds::new(),
            stars,
            rng,
            score: 0,
            kills: 0,
            wave: 1,
            target,
            time: 0.0,
            shake: 0.0,
            paused: false,
            show_shapes: false,
            frame: 0,
            stats: FrameStats::new(),
            alive: 0,
            line: String::with_capacity(128),
            scratch_users: Vec::new(),
            scratch_dead: Vec::new(),
            _atlas: atlas,
            o,
        };
        println!("art: {}", game.art.source);
        game.spawn_enemies(ctx);
        game
    }

    fn enemy_count(&mut self) -> usize {
        self.world.query::<&Enemy>().iter(&self.world).len()
    }

    /// Tops the attackers up to the wave's target, just outside the view.
    fn spawn_enemies(&mut self, ctx: &Context) {
        let missing = self.target.saturating_sub(self.enemy_count());
        let reach = ctx.camera.visible_size().length() * 0.55 + 100.0;
        for _ in 0..missing {
            let roll = self.rng.f32();
            let heavy_share = (0.04 * self.wave as f32).min(0.25);
            let kind = if roll < heavy_share {
                2
            } else if roll < 0.45 {
                1
            } else {
                0
            };
            let pos = self.player.body.pos + self.rng.direction() * self.rng.range(reach, reach * 1.5);
            let to_player = (self.player.body.pos - pos).normalize_or_zero();
            let vel = to_player * KINDS[kind].speed * 0.5;
            let rot = vel.to_angle() + FRAC_PI_2;
            let trail_color = match kind {
                0 => [255, 210, 90, 160],
                1 => [255, 110, 70, 170],
                _ => [200, 120, 255, 170],
            };
            self.world.spawn((
                Body { pos, prev: pos, vel, rot, prev_rot: rot },
                Enemy {
                    kind,
                    hp: KINDS[kind].hp,
                    cooldown: self.rng.range(0.5, 3.0),
                    side: if self.rng.chance(0.5) { 1.0 } else { -1.0 },
                    range: self.rng.range(260.0, 480.0),
                },
                Exhaust(Trail::new(KINDS[kind].size * 0.22, trail_color).with_spacing(7.0)),
            ));
        }
    }

    fn pan(&self, ctx: &Context, pos: Vec2) -> f32 {
        let screen = ctx.camera.world_to_screen(pos);
        (screen.x / ctx.camera.viewport.x * 2.0 - 1.0).clamp(-1.0, 1.0) * 0.7
    }

    fn update_player(&mut self, ctx: &mut Context, dt: f32) {
        let p = &mut self.player;
        p.body.prev = p.body.pos;
        p.body.prev_rot = p.body.rot;
        if !p.alive {
            p.respawn -= dt;
            if p.respawn <= 0.0 {
                let keep = p.body.pos;
                *p = Player::new();
                p.body.pos = keep;
                p.body.prev = keep;
            }
            return;
        }
        p.invulnerable = (p.invulnerable - dt).max(0.0);
        p.cooldown -= dt;

        let input = &ctx.input;
        let (mut thrust, mut aim, firing);
        if self.o.autopilot {
            // Circle the origin, aim at the nearest enemy, always fire.
            let t = self.time * 0.35;
            let goal = Vec2::new(t.cos(), (t * 2.0).sin() * 0.5) * 380.0;
            thrust = ((goal - p.body.pos) / 150.0).clamp_length_max(1.0);
            let me = p.body.pos;
            let nearest = self
                .world
                .query::<&Body>()
                .iter(&self.world)
                .filter(|b| b.pos != me)
                .min_by(|a, b| a.pos.distance_squared(me).total_cmp(&b.pos.distance_squared(me)));
            aim = nearest.map_or(Vec2::NEG_Y, |b| (b.pos + b.vel * 0.15 - me).normalize_or_zero());
            firing = true;
        } else {
            thrust = Vec2::ZERO;
            let key = |a: KeyCode, b: KeyCode| input.key_down(a) || input.key_down(b);
            if key(KeyCode::KeyA, KeyCode::ArrowLeft) {
                thrust.x -= 1.0;
            }
            if key(KeyCode::KeyD, KeyCode::ArrowRight) {
                thrust.x += 1.0;
            }
            if key(KeyCode::KeyW, KeyCode::ArrowUp) {
                thrust.y -= 1.0;
            }
            if key(KeyCode::KeyS, KeyCode::ArrowDown) {
                thrust.y += 1.0;
            }
            thrust = (thrust + input.left_stick()).clamp_length_max(1.0);
            let mouse = ctx.camera.screen_to_world(input.mouse_position());
            aim = (mouse - p.body.pos).normalize_or(p.aim);
            let stick = input.right_stick();
            if stick.length() > 0.3 {
                aim = stick.normalize();
            }
            firing = input.mouse_down(MouseButton::Left)
                || input.key_down(KeyCode::Space)
                || input.pad_down(GamepadButton::RightTrigger)
                || input.pad_down(GamepadButton::South)
                || stick.length() > 0.6;
        }
        p.aim = aim;
        let accel = 5.0;
        p.body.vel += (thrust * PLAYER_SPEED - p.body.vel) * (accel * dt).min(1.0);
        p.body.pos += p.body.vel * dt;
        p.body.rot = turn_towards(p.body.rot, aim.to_angle() + FRAC_PI_2, 14.0 * dt);
        let back = Vec2::from_angle(p.body.rot - FRAC_PI_2);
        p.trail.push(p.body.pos - back * 24.0);

        if firing && p.cooldown <= 0.0 {
            p.cooldown += FIRE_INTERVAL;
            if p.cooldown < 0.0 {
                p.cooldown = 0.0;
            }
            let nose = Vec2::from_angle(p.body.rot - FRAC_PI_2);
            let side = nose.perp() * 11.0;
            for s in [-1.0, 1.0] {
                self.bolts.push(Bolt {
                    pos: p.body.pos + nose * 26.0 + side * s,
                    vel: nose * BOLT_SPEED + p.body.vel * 0.5,
                    life: 0.9,
                    damage: 1.0,
                    friendly: true,
                });
            }
            let pitch = self.rng.range(0.92, 1.08);
            let pan = self.pan(ctx, self.player.body.pos);
            ctx.audio.play_with(&self.sounds.laser, Bus::Sfx, PlayParams { volume: 0.35, pitch, pan });
        }
    }

    fn enemy_fire(&mut self, ctx: &mut Context, dt: f32) {
        if !self.player.alive {
            return;
        }
        let target = self.player.body.pos;
        let mut shots = 0;
        let mut q = self.world.query::<(&Body, &mut Enemy)>();
        for (body, mut enemy) in q.iter_mut(&mut self.world) {
            let kind = &KINDS[enemy.kind];
            if !kind.shoots {
                continue;
            }
            enemy.cooldown -= dt;
            if enemy.cooldown > 0.0 || body.pos.distance_squared(target) > 750.0 * 750.0 {
                continue;
            }
            enemy.cooldown = self.rng.range(1.2, 2.6) * if enemy.kind == 2 { 0.6 } else { 1.0 };
            let dir = (target - body.pos).normalize_or_zero();
            let spread = if enemy.kind == 2 { 3 } else { 1 };
            for i in 0..spread {
                let a = (i as f32 - (spread - 1) as f32 * 0.5) * 0.18;
                self.bolts.push(Bolt {
                    pos: body.pos + dir * kind.size * 0.5,
                    vel: dir.rotate(Vec2::from_angle(a)) * ENEMY_BOLT_SPEED,
                    life: 1.6,
                    damage: 6.0,
                    friendly: false,
                });
            }
            shots += 1;
        }
        if shots > 0 {
            ctx.audio.play_with(
                &self.sounds.enemy_laser,
                Bus::Sfx,
                PlayParams { volume: 0.18, pitch: 1.0, pan: 0.0 },
            );
        }
    }

    /// Rebuilds the collision grid: every enemy and the player.
    fn fill_grid(&mut self) {
        self.grid.clear();
        let mut q = self.world.query::<(Entity, &Body, &Enemy)>();
        for (entity, body, enemy) in q.iter(&self.world) {
            let r = KINDS[enemy.kind].size * 0.42;
            self.grid.insert(Collider::new(
                body.pos,
                Shape::circle(r),
                layers::ENEMY,
                layers::PLAYER | layers::PLAYER_SHOT,
                entity.to_bits(),
            ));
        }
        if self.player.alive {
            self.grid.insert(Collider::new(
                self.player.body.pos,
                Shape::circle(PLAYER_RADIUS),
                layers::PLAYER,
                layers::ENEMY | layers::ENEMY_SHOT,
                u64::MAX,
            ));
        }
        self.grid.build();
    }

    /// Moves every bolt, tests its path against the grid (in parallel), and applies the hits.
    fn update_bolts(&mut self, ctx: &mut Context, dt: f32) {
        let grid = &self.grid;
        let hit_test = |b: &Bolt| {
            let mask = if b.friendly { layers::ENEMY } else { layers::PLAYER };
            grid.raycast(b.pos, b.pos + b.vel * dt, 3.0, mask)
        };
        self.hits.clear();
        if self.bolts.len() > 512 {
            self.bolts.par_iter().map(hit_test).collect_into_vec(&mut self.hits);
        } else {
            self.hits.extend(self.bolts.iter().map(hit_test));
        }
        let mut i = 0;
        let mut hit_sound = false;
        while i < self.bolts.len() {
            let hit = self.hits[i];
            let b = &mut self.bolts[i];
            b.pos += b.vel * dt;
            b.life -= dt;
            let mut gone = b.life <= 0.0;
            if let Some(h) = hit {
                gone = true;
                let (friendly, damage) = (b.friendly, b.damage);
                self.fx.hit(h.point, h.normal, friendly);
                let user = self.grid.get(h.index).user;
                if user == u64::MAX {
                    self.damage_player(damage);
                } else if let Some(mut e) = self.world.get_mut::<Enemy>(Entity::from_bits(user)) {
                    e.hp -= damage;
                    hit_sound = true;
                }
            }
            if gone {
                self.bolts.swap_remove(i);
                self.hits.swap_remove(i);
            } else {
                i += 1;
            }
        }
        if hit_sound {
            let pitch = self.rng.range(0.9, 1.2);
            ctx.audio.play_with(&self.sounds.hit, Bus::Sfx, PlayParams { volume: 0.3, pitch, pan: 0.0 });
        }
    }

    fn damage_player(&mut self, amount: f32) {
        let p = &mut self.player;
        if !p.alive || p.invulnerable > 0.0 {
            return;
        }
        p.hull -= amount;
        self.shake = self.shake.max(6.0);
        if p.hull <= 0.0 {
            p.alive = false;
            p.respawn = 2.5;
            self.shake = 22.0;
            let (pos, vel) = (p.body.pos, p.body.vel);
            self.fx.explosion(pos, vel, 110.0);
        }
    }

    /// Ships that ran into the player blow up and hurt it; ships out of hit points blow up.
    fn resolve(&mut self, ctx: &mut Context) {
        if self.player.alive {
            let mut rammed = std::mem::take(&mut self.scratch_users);
            rammed.clear();
            self.grid.query(self.player.body.pos, Shape::circle(PLAYER_RADIUS), layers::ENEMY, |_, c| {
                rammed.push(c.user)
            });
            for &user in &rammed {
                if let Some(mut e) = self.world.get_mut::<Enemy>(Entity::from_bits(user)) {
                    e.hp = 0.0;
                    let ram = 10.0 + 10.0 * e.kind as f32;
                    self.damage_player(ram);
                }
            }
            self.scratch_users = rammed;
        }
        let mut dead = std::mem::take(&mut self.scratch_dead);
        dead.clear();
        let mut q = self.world.query::<(Entity, &Body, &Enemy)>();
        for (entity, body, enemy) in q.iter(&self.world) {
            let far = body.pos.distance(self.player.body.pos) > 4000.0;
            if enemy.hp <= 0.0 || far {
                dead.push((entity, *body, enemy.kind, !far));
            }
        }
        for &(entity, body, kind, exploded) in &dead {
            self.world.despawn(entity);
            if !exploded {
                continue;
            }
            let k = &KINDS[kind];
            self.fx.explosion(body.pos, body.vel, k.size);
            self.score += k.score;
            self.kills += 1;
            self.shake = self.shake.max(2.0 + k.size * 0.06);
            let boom = &self.sounds.booms[kind];
            let pitch = self.rng.range(0.85, 1.15);
            let pan = self.pan(ctx, body.pos);
            ctx.audio.play_with(boom, Bus::Sfx, PlayParams { volume: 0.5 + kind as f32 * 0.2, pitch, pan });
        }
        self.scratch_dead = dead;
        // Benchmarks keep one wave so every run does the same work.
        if !self.o.bench && self.kills >= self.wave * 30 {
            self.wave += 1;
            self.target = self.o.enemies + self.o.enemies * (self.wave as usize - 1) / 3;
        }
    }

    fn draw_ship(
        &self,
        batch: &mut SpriteBatch,
        region: &SpriteRegion,
        pos: Vec2,
        rot: f32,
        size: f32,
        color: [u8; 4],
    ) {
        let aspect = region.size.y / region.size.x.max(1.0);
        batch.push(
            30,
            Blend::Alpha,
            SpriteInstance::sized(region, pos, Vec2::new(size, size * aspect), rot).with_color(color),
        );
    }

    fn draw_hud(&mut self, ctx: &Context, batch: &mut SpriteBatch) {
        let view = ctx.camera.viewport;
        let hud = &self.art.hud;
        let small = &self.art.small;
        let big = TextStyle::screen(hud.raster_size()).layer(240);
        let little = TextStyle::screen(small.raster_size()).layer(240);
        let overlay_room = if ctx.overlay.visible { 260.0 } else { 0.0 };
        let mut line = std::mem::take(&mut self.line);
        line.clear();
        let _ = write!(line, "{}", self.score);
        hud.draw_aligned(batch, &line, Vec2::new(view.x - 24.0, 16.0), Align::TOP_RIGHT, big);
        let top = 16.0 + overlay_room;
        line.clear();
        let _ = write!(line, "Wave {}", self.wave);
        hud.draw_aligned(
            batch,
            &line,
            Vec2::new(view.x * 0.5, top),
            Align::TOP,
            big.color([255, 220, 120, 255]),
        );

        // Hull bar.
        let w = 260.0;
        let pos = Vec2::new(24.0, top + 8.0);
        let bar = |batch: &mut SpriteBatch, p: Vec2, size: Vec2, c: [u8; 4]| {
            batch.push_screen(
                239,
                Blend::Alpha,
                SpriteInstance::sized(&self.art.white, p + size * 0.5, size, 0.0).with_color(c),
            );
        };
        bar(batch, pos - Vec2::splat(3.0), Vec2::new(w + 6.0, 22.0), [0, 0, 0, 160]);
        let hull = self.player.hull.max(0.0) / 100.0;
        let color = if hull > 0.5 {
            [90, 200, 255, 230]
        } else if hull > 0.25 {
            [255, 200, 70, 230]
        } else {
            [255, 80, 60, 230]
        };
        bar(batch, pos, Vec2::new(w * hull, 16.0), color);
        small.draw(batch, "Hull", pos + Vec2::new(w + 14.0, -4.0), little);

        line.clear();
        let _ = write!(line, "Enemies {}   Bolts {}", self.alive, self.bolts.len());
        small.draw(batch, &line, pos + Vec2::new(0.0, 30.0), little.color([200, 210, 230, 255]));

        if !self.player.alive {
            hud.draw_aligned(batch, "Destroyed!", view * 0.5, Align::CENTER, big.color([255, 120, 90, 255]));
        } else if self.paused {
            hud.draw_aligned(batch, "Pause", view * 0.5, Align::CENTER, big);
        }
        if !self.o.autopilot {
            small.draw_aligned(
                batch,
                "WASD / stick: fly   mouse / right stick: aim   click / Space / RT: fire   F3: overlay   F1: shapes   P: pause",
                Vec2::new(view.x * 0.5, view.y - 12.0),
                Align::BOTTOM,
                little.color([170, 180, 200, 200]),
            );
        }
        self.line = line;
    }

    fn bench_frame(&mut self, ctx: &mut Context) {
        self.frame += 1;
        if let Some(path) = &self.o.shot {
            if self.frame == self.o.shot_frame {
                ctx.request_screenshot(path.clone());
            } else if self.frame > self.o.shot_frame {
                ctx.exit();
            }
        }
        if !self.o.bench {
            return;
        }
        if self.frame == self.o.warmup {
            ctx.set_vsync(false);
        }
        if self.frame > self.o.warmup + 2 {
            self.stats.push(ctx.timings.frame_ms);
        }
        if self.stats.len() >= self.o.frames {
            let s = self.stats.summary().expect("frames");
            let t = ctx.timings;
            println!(
                "FLUX2D SUMMARY example=shooter enemies={} entities={} bolts={} particles={} frames={} avg_ms={:.2} p50_ms={:.2} p95_ms={:.2} p99_ms={:.2} max_ms={:.2} over_16.7ms={:.1}% fps={:.0} update_ms={:.2} draw_ms={:.2} upload_ms={:.2} present_ms={:.2} sprites={} draw_calls={}",
                self.target,
                self.alive,
                self.bolts.len(),
                self.fx.len(),
                s.frames,
                s.avg_ms,
                s.p50_ms,
                s.p95_ms,
                s.p99_ms,
                s.max_ms,
                s.over_60hz * 100.0,
                s.fps(),
                t.update_ms / t.updates.max(1) as f32,
                t.draw_ms,
                t.upload_ms,
                t.present_ms,
                t.upload.sprites,
                t.upload.draw_calls
            );
            println!("GPU: {}", ctx.gpu.adapter_name());
            ctx.exit();
        }
    }
}

impl Game for Shooter {
    fn update(&mut self, ctx: &mut Context, dt: f32) {
        if ctx.input.key_pressed(KeyCode::Escape) {
            ctx.exit();
        }
        if ctx.input.key_pressed(KeyCode::KeyP) || ctx.input.pad_pressed(GamepadButton::Start) {
            self.paused = !self.paused;
        }
        if ctx.input.key_pressed(KeyCode::F1) {
            self.show_shapes = !self.show_shapes;
        }
        if self.paused {
            return;
        }
        self.time += dt;
        self.shake *= (1.0 - 6.0 * dt).max(0.0);

        self.update_player(ctx, dt);
        {
            let mut step = self.world.resource_mut::<Step>();
            step.dt = dt;
            step.player = self.player.body.pos;
            step.player_alive = self.player.alive;
        }
        self.schedule.run(&mut self.world);
        self.enemy_fire(ctx, dt);
        self.fill_grid();
        self.update_bolts(ctx, dt);
        self.resolve(ctx);
        self.fx.update(dt);
        self.spawn_enemies(ctx);
        self.alive = self.enemy_count();
    }

    fn draw(&mut self, ctx: &mut Context, batch: &mut SpriteBatch, alpha: f32) {
        let zoom = (ctx.camera.zoom * 1.1f32.powf(ctx.input.wheel())).clamp(0.3, 2.5);
        ctx.camera.zoom = zoom;
        let dt = 1.0 / 60.0;
        let lag = (1.0 - alpha) * dt;
        let p = &self.player;
        let player_pos = p.body.prev.lerp(p.body.pos, alpha);
        let shake = if self.shake > 0.1 { self.rng.direction() * self.shake } else { Vec2::ZERO };
        ctx.camera.position = player_pos + p.aim * 60.0 + shake;

        self.stars.draw(batch, &self.art, &ctx.camera);

        // Enemies, their exhaust and engine glow.
        let mut q = self.world.query::<(&Body, &Enemy, &Exhaust)>();
        for (b, e, trail) in q.iter(&self.world) {
            let k = &KINDS[e.kind];
            let pos = b.prev.lerp(b.pos, alpha);
            let rot = lerp_angle(b.prev_rot, b.rot, alpha);
            trail.0.draw(
                batch,
                &self.art.glow,
                25,
                Blend::Additive,
                pos - Vec2::from_angle(rot - FRAC_PI_2) * 14.0,
            );
            let flash = if e.hp < k.hp { [255, 200, 200, 255] } else { [255; 4] };
            self.draw_ship(batch, &self.art.enemies[e.kind], pos, rot, k.size, flash);
        }

        // The player.
        if p.alive {
            let rot = lerp_angle(p.body.prev_rot, p.body.rot, alpha);
            let back = Vec2::from_angle(rot - FRAC_PI_2);
            p.trail.draw(batch, &self.art.glow, 25, Blend::Additive, player_pos - back * 24.0);
            let blink = p.invulnerable > 0.0 && (self.time * 12.0).fract() < 0.5;
            let color = if blink { [255, 255, 255, 110] } else { [255; 4] };
            self.draw_ship(batch, &self.art.player, player_pos, rot, PLAYER_SIZE, color);
            batch.push(
                31,
                Blend::Additive,
                SpriteInstance::sized(&self.art.glow, player_pos - back * 26.0, Vec2::splat(26.0), 0.0)
                    .with_color([120, 200, 255, 200]),
            );
        }

        // Bolts: a streak plus a glow, drawn behind by `lag` to match interpolated ships.
        let bolt_region = self.art.bolt;
        let glow_region = self.art.glow;
        let streaks = batch.alloc(40, Blend::Additive, self.bolts.len());
        for (slot, b) in streaks.iter_mut().zip(&self.bolts) {
            let color = if b.friendly { [120, 255, 190, 255] } else { [255, 110, 90, 255] };
            *slot = SpriteInstance::sized(
                &bolt_region,
                b.pos - b.vel * lag,
                Vec2::new(34.0, 7.0),
                b.vel.to_angle(),
            )
            .with_color(color);
        }
        let glows = batch.alloc(39, Blend::Additive, self.bolts.len());
        for (slot, b) in glows.iter_mut().zip(&self.bolts) {
            let color = if b.friendly { [60, 255, 160, 90] } else { [255, 80, 60, 90] };
            *slot = SpriteInstance::sized(&glow_region, b.pos - b.vel * lag, Vec2::splat(26.0), 0.0)
                .with_color(color);
        }

        self.fx.draw(batch, lag);

        if self.show_shapes {
            for c in self.grid.colliders() {
                let r = c.shape.half_extents();
                let color =
                    if c.layer & layers::PLAYER != 0 { [80, 160, 255, 90] } else { [255, 80, 80, 90] };
                batch.push(
                    100,
                    Blend::Alpha,
                    SpriteInstance::sized(&self.art.glow, c.pos, r * 2.0, 0.0).with_color(color),
                );
            }
        }

        self.draw_hud(ctx, batch);

        let o = &mut ctx.overlay.text;
        let _ = writeln!(
            o,
            "entities {}  bolts {}  particles {}  colliders {}",
            self.alive,
            self.bolts.len(),
            self.fx.len(),
            self.grid.len()
        );
        let _ = writeln!(
            o,
            "audio: {}  sounds dropped {}",
            if ctx.audio.is_enabled() { "on" } else { "off" },
            ctx.audio.dropped()
        );
        let _ = write!(
            o,
            "{}",
            if ctx.input.first_gamepad().is_some() { "gamepad connected" } else { "no gamepad" }
        );
    }

    fn frame_end(&mut self, ctx: &mut Context) {
        self.bench_frame(ctx);
    }
}

fn main() {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("warn")).init();
    let o = Options::parse();
    let config = Config {
        title: "Flux2D shooter".into(),
        width: o.width,
        height: o.height,
        vsync: !o.bench,
        audio: o.audio && !o.bench,
        dev_overlay: o.overlay,
        ..Config::default()
    };
    flux2d::run(config, move |ctx| Shooter::new(ctx, o)).expect("flux2d");
}
