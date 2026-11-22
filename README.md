# Flux2D

A 2D game engine in Rust, built to put a very large number of sprites on screen.

Flux2D draws with [wgpu](https://wgpu.rs) (DirectX 12 or Vulkan on Windows, Metal on macOS,
Vulkan on Linux). Every sprite is a 48-byte instance in one GPU buffer, all art lives in one
texture array, and a whole frame of sprites goes out in a single buffer write and a couple of
instanced draw calls. Its test case is a port of the Godot game **Cosmic Commando**.

Status: **milestone 2**, game foundations: gamepads, audio buses, text, a collision grid,
an ECS, asset loading, particles and trails, a dev overlay and profiling, shown off by a small
shooter. See [docs/architecture.md](docs/architecture.md) for the plan.

## Quick start

Install Rust (stable, 1.95 or newer; `rustup update` if you installed it earlier) from
[rustup.rs](https://rustup.rs), then:

```sh
git clone https://github.com/michaeljburton/flux2d
cd flux2d
cargo run --release --example shooter   # milestone 2: fly, shoot, blow things up
cargo run --release --example stress    # milestone 1: hundreds of thousands of sprites
```

On Linux, building needs the udev and ALSA headers (`sudo apt install libudev-dev libasound2-dev`
on Debian and Ubuntu). Windows and macOS need nothing extra.

### Shooter

Waves of drones, fighters and heavies attack; fighters and heavies shoot back. Score and wave are
top right and centre, the hull bar top left.

| Input | Does |
| --- | --- |
| WASD / arrows / left stick | fly |
| Mouse / right stick | aim |
| Left mouse / Space / right trigger / A | fire |
| Mouse wheel | zoom |
| F3 | dev overlay: FPS, frame timings, sprites, draw calls, a frame-time graph |
| F1 | show collision shapes |
| P / Start | pause |
| Esc | quit |

Options: `--enemies=N` (attackers at once, default 24), `--bench` (autopilot, vsync off, prints
a `FLUX2D SUMMARY` line), `--overlay`, `--no-audio`, `--shot=out.png`, `--art=DIR`,
`--font=FILE.ttf`. It uses Cosmic Commando's ship art when it finds it (as the stress test does,
below) and a system font for the HUD.

The stress test uses Cosmic Commando's ship art when the game's repo sits next to this one
(`../cosmic-commando/cosmic-commando-2d/images`), or wherever `--art=DIR` or the
`COSMIC_COMMANDO_IMAGES` environment variable points. Without it, it draws plain shapes.

### Stress test controls

| Key | Does |
| --- | --- |
| Up / Down | +/- 10,000 sprites |
| Page Up / Page Down | +/- 100,000 sprites |
| Space | pause movement |
| V | vsync on / off |
| Mouse wheel | zoom about the pointer |
| Right or middle drag | pan |
| Home | reset the camera |
| Esc | quit |

The window title shows the sprite count, frame time and where the CPU time goes.

### Benchmarks

```sh
# One count: 120 warm-up frames, then 600 measured frames, vsync off.
cargo run --release --example stress -- --bench --sprites=200000

# A sweep from 10,000 to 1,000,000 sprites; prints a Markdown table for the results below.
cargo run --release --example stress -- --sweep
```

Each run prints a `FLUX2D SUMMARY` line with the same fields as Cosmic Commando's
`PERF SUMMARY` (average, p50, p95, p99 and max frame time, the share of frames over 16.7 ms)
plus where the main thread's time went:

| Field | Meaning |
| --- | --- |
| `update_ms` | one fixed update (the stress test moves every sprite, on all cores); a slow frame runs several |
| `draw_ms` | filling the sprite batch |
| `upload_ms` | copying the batch to the GPU, recording and submitting the draws |
| `present_ms` | waiting for a swapchain image and presenting; GPU-bound time shows up here |

Other options: `--frames=N`, `--warmup=N`, `--vsync=on|off`, `--size=WxH`, `--zoom=Z`,
`--static` (no movement, drawing only), `--shot=out.png` (save a frame and quit).
Run with `--help` for the full list.

The shooter's bench is the game-shaped one: `cargo run --release --example shooter -- --bench --enemies=2000`
prints the same fields plus entity, bolt and particle counts.

Every sprite in the stress test is on screen at once (the scene is the size of the window at
zoom 1), so large counts are mostly a test of the GPU's fill rate: a real game spreads them over
a bigger world.

## Results

Milestone 1's gate was 200,000 sprites at 60 fps on an RTX 3080 Ti.

| Machine | 200k | 1M |
| --- | ---: | ---: |
| RTX 3080 Ti, Windows | 163 fps (6.13 ms) | 59 fps (16.9 ms) |

At 200k and below the frame was a flat 6.13 ms spent almost all in present (likely a driver or
G-Sync cap); at 1M the batch upload took 6.3 ms and drawing 5.3 ms.

See [docs/benchmarks.md](docs/benchmarks.md) for the full tables, including the cloud
container's software renderer.

## Using it

```rust
use flux2d::prelude::*;

struct MyGame {
    ship: SpriteRegion,
    font: Font,
    pew: Sound,
    pos: Vec2,
}

impl Game for MyGame {
    fn update(&mut self, ctx: &mut Context, dt: f32) {
        let mut dir = ctx.input.left_stick();
        if ctx.input.key_down(KeyCode::ArrowRight) {
            dir.x += 1.0;
        }
        self.pos += dir * 200.0 * dt;
        // Edges read in `update` mean "since the last update": a tap is never missed.
        if ctx.input.key_pressed(KeyCode::Space) || ctx.input.pad_pressed(GamepadButton::South) {
            ctx.audio.play(&self.pew, Bus::Sfx);
        }
    }

    fn draw(&mut self, _ctx: &mut Context, batch: &mut SpriteBatch, _alpha: f32) {
        batch.push(1, Blend::Alpha, SpriteInstance::new(&self.ship, self.pos, 0.25, 0.0));
        // Text is sprites too; screen space is window pixels, drawn over the world.
        self.font.draw(batch, "Hello, Commander", vec2(20.0, 20.0), TextStyle::screen(32.0));
    }
}

fn main() {
    flux2d::run(Config::default(), |ctx| {
        let assets = Assets::new("assets");
        let mut art = AtlasBuilder::new();
        let ship = assets.image(&mut art, "ship.png", None).unwrap();
        let font = assets.font(&mut art, "fonts/hud.ttf", 32.0, flux2d::render::text::LATIN).unwrap();
        let atlas = art.build(&ctx.gpu).unwrap();
        ctx.set_atlas(&atlas);
        MyGame {
            ship: atlas.region(ship),
            font: atlas.font(font).clone(),
            pew: assets.sound("sfx/pew.ogg").unwrap(),
            pos: Vec2::ZERO,
        }
    })
    .unwrap();
}
```

- `update` runs at a fixed 60 Hz; `draw` runs once per displayed frame with `alpha`, how far
  the clock is between two updates, for smooth motion on any refresh rate.
- Sprites are drawn by layer (0 first), alpha-blended before additive within a layer, and in the
  order they were added within a bucket. Nothing is sorted per frame.
- `SpriteBatch::alloc(layer, blend, n)` hands back `n` slots to fill, for example in parallel
  with `rayon`. `push_screen` / `alloc_in(Space::Screen, ..)` draw in window pixels (HUD).

What else is in the box (all reachable from `flux2d::`):

| | |
| --- | --- |
| `physics::CollisionGrid` | circles and boxes with Godot-style layers and masks; `query`, `pairs`, `raycast` (thick rays for bolts). Rebuild it each step; query it from many threads. |
| `fx::Particles`, `fx::Trail` | particle pools with burst `Emitter`s, and ribbon trails |
| `audio::Audio` (`ctx.audio`) | sounds and streamed music on master / music / SFX / voice buses (kira) |
| `ecs` | bevy_ecs on its own, for game objects. Add `bevy_ecs = "0.20"` to your game's `Cargo.toml` too, so `#[derive(Component)]` finds it. |
| `assets::Assets` | finds the game's folder (env var, next to the exe, or up from the working directory) and loads images, whole art folders (in parallel), fonts, sounds and JSON |
| `ctx.overlay` | the F3 dev overlay; write your own lines into `ctx.overlay.text` |
| `core::Rng` | a small seedable random number generator |

### Profiling

Build with `--features profile` and run the [Tracy](https://github.com/wolfpld/tracy) profiler, version 0.14 (the one
tracy-client 0.19 speaks), to see
each frame's `update`, `draw`, `upload` and `present` zones. Mark your own code with
`flux2d::profile_scope!("name");`; without the feature it compiles to nothing.

```sh
cargo run --release --features profile --example shooter -- --enemies=1000
```

## Development

```sh
cargo fmt --all
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

The GPU tests (`crates/flux2d-render/tests/offscreen.rs`) draw into an offscreen texture and
check pixels (blending, rotation, screen space and text). They run on any GPU, on Mesa's software Vulkan driver in CI, and skip themselves
when there is no adapter at all.
