# Flux2D

A 2D game engine in Rust, built to put a very large number of sprites on screen.

Flux2D draws with [wgpu](https://wgpu.rs) (DirectX 12 or Vulkan on Windows, Metal on macOS,
Vulkan on Linux). Every sprite is a 48-byte instance in one GPU buffer, all art lives in one
texture array, and a whole frame of sprites goes out in a single buffer write and a couple of
instanced draw calls. Its test case is a port of the Godot game **Cosmic Commando**.

Status: **milestone 1**, the sprite renderer and a stress test. See
[docs/architecture.md](docs/architecture.md) for the plan.

## Quick start

Install Rust (stable, 1.88 or newer) from [rustup.rs](https://rustup.rs), then:

```sh
git clone https://github.com/michaeljburton/flux2d
cd flux2d
cargo run --release --example stress
```

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

Every sprite in the stress test is on screen at once (the scene is the size of the window at
zoom 1), so large counts are mostly a test of the GPU's fill rate: a real game spreads them over
a bigger world.

## Results

Milestone 1 is done when 200,000 sprites hold 60 fps on an RTX 3080 Ti.

| Machine | 10k | 50k | 100k | 200k | 500k | 1M |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| RTX 3080 Ti, 1920x1080 | _to measure_ | | | | | |

See [docs/benchmarks.md](docs/benchmarks.md) for the full tables, including the cloud
container's software renderer.

## Using it

```rust
use flux2d::prelude::*;

struct MyGame {
    ship: SpriteRegion,
    pos: Vec2,
}

impl Game for MyGame {
    fn update(&mut self, ctx: &mut Context, dt: f32) {
        if ctx.input.key_down(KeyCode::ArrowRight) {
            self.pos.x += 200.0 * dt;
        }
    }

    fn draw(&mut self, _ctx: &mut Context, batch: &mut SpriteBatch, _alpha: f32) {
        batch.push(1, Blend::Alpha, SpriteInstance::new(&self.ship, self.pos, 0.25, 0.0));
    }
}

fn main() {
    flux2d::run(Config::default(), |ctx| {
        let mut art = AtlasBuilder::new();
        let ship = art.add_file("ship.png", None).unwrap();
        let atlas = art.build(&ctx.gpu).unwrap();
        ctx.set_atlas(&atlas);
        MyGame { ship: atlas.region(ship), pos: Vec2::ZERO }
    })
    .unwrap();
}
```

- `update` runs at a fixed 60 Hz; `draw` runs once per displayed frame with `alpha`, how far
  the clock is between two updates, for smooth motion on any refresh rate.
- Sprites are drawn by layer (0 first), alpha-blended before additive within a layer, and in the
  order they were added within a bucket. Nothing is sorted per frame.
- `SpriteBatch::alloc(layer, blend, n)` hands back `n` slots to fill, for example in parallel
  with `rayon`.

## Development

```sh
cargo fmt --all
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

The GPU tests (`crates/flux2d-render/tests/offscreen.rs`) draw into an offscreen texture and
check pixels. They run on any GPU, on Mesa's software Vulkan driver in CI, and skip themselves
when there is no adapter at all.
