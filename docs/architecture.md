# Flux2D architecture and plan

The full proposal (with the reasoning) is the project's plan document,
[Rust 2D engine: architecture plan and first milestone](https://claude.ai/artifact/HG4kQCiFFMtmaw6TigDZTa).
This page is the short version kept next to the code.

## Goal

Draw hundreds of thousands of sprites at 60 fps, then prove it by porting Cosmic Commando
(Godot 4.7, ~43,600 lines of GDScript), starting with Horde mode. The Godot game's worst case
today is the Mayday fleet battle: ~95 enemies, ~60 allies, 30 carrier fighters and 300 to 500
laser bolts at once.

## How a frame works

1. `flux2d::run` owns the window (winit) and the GPU (wgpu).
2. `Game::update` runs at a fixed rate (60 Hz) as many times as real time asks for
   (`FixedTimestep`, at most 8 steps a frame so a hitch can't snowball).
3. `Game::draw` fills a `SpriteBatch` once per displayed frame. Sprites go into one bucket per
   (draw layer 0-255, blend mode); there is no sorting.
4. `SpriteRenderer::prepare` writes the camera matrix and copies every bucket, end to end, into
   one instance buffer through wgpu's staging memory (no intermediate copy).
5. `SpriteRenderer::draw` issues one instanced draw per run of buckets that share a blend mode.
   The stress test's 200,000 sprites are 2 draw calls.
6. The vertex shader builds each quad from the vertex index (4-vertex triangle strip, no vertex
   buffer) and the instance's position, size, rotation and atlas rectangle.

## Data

- `SpriteInstance` (48 bytes): position, size, UV rectangle, rotation, texture array layer,
  RGBA tint, one free `param` for effect shaders.
- `Atlas`: every image packed (shelf packing, 8 px padding) into one `Rgba8Unorm` texture
  array, premultiplied alpha, 5 mip levels built on the CPU. Layers are square, the smallest
  power of two that fits (up to 4096); more art means more layers, not more textures.
- Blending is premultiplied alpha (normal) or additive, done in the art's sRGB encoding like
  Godot's 2D renderer, so the port keeps the game's look.

## Game foundations (M2)

| Crate | What it gives a game |
| --- | --- |
| `flux2d-core` | `Input` with keyboard, mouse and up to 4 gamepads (gilrs; sticks deadzoned, y down). Edges are kept twice: read in `update` they mean "since the last fixed step" (so a tap is never lost on a 240 Hz screen, or seen twice on a slow frame); read elsewhere they last one displayed frame. `Rng` (xorshift, seedable). `profile_scope!` / `profile_frame!` for Tracy. |
| `flux2d-render` | `Space::Screen` sprites in window pixels, drawn after the world through a second camera matrix (one uniform buffer, dynamic offset). Text as sprites: `AtlasBuilder::add_font` bakes a TTF's glyphs (ASCII plus Latin-1 and French punctuation by default) into the atlas; every atlas also carries a white sprite and a built-in 8x8 pixel font. `add_dir` / `add_files` decode art on every core. |
| `flux2d-physics` | `CollisionGrid`: rebuilt every step by a counting sort into flat arrays (no per-collider allocation). Circles and axis-aligned boxes, Godot-style 32-bit layers and masks, shape queries, all overlapping pairs, and thick raycasts by grid walking. Read-only after `build`, so bolts can query it from many threads. |
| `flux2d-fx` | `Particles`: one pool per look, 52-byte `Copy` particles, burst `Emitter`s, drag and gravity, updated and drawn in parallel above 4,096. `Trail`: a fixed ring of 24 points drawn as a tapering ribbon. |
| `flux2d-audio` | kira with three buses (music, SFX, voice) under a master volume, up to 128 voices a bus (extras are dropped and counted), streamed looping music with cross-fades, and code-made sounds (`synth`). No audio device means silent, not an error. |
| `flux2d` | `ctx.audio`, `ctx.overlay` (F3: FPS, timings, sprite and draw-call counts, a 240-frame graph, plus the game's own lines), `assets::Assets` (finds the game folder, loads JSON with serde, images, sounds, fonts), `flux2d::ecs` (bevy_ecs). |

How the shooter example splits the work, which is the pattern for the port:

- Ships are **bevy_ecs entities**, moved by systems in a `Schedule`. bevy_ecs runs
  single-threaded here (no `multi_threaded` feature) so it doesn't start a second thread pool
  next to rayon's.
- Bolts and particles are **packed arrays**. Each step every bolt's path is raycast against the
  collision grid on all cores, then hits are applied in one pass.
- Explosions are particle bursts plus a sound panned by screen position; the HUD is TTF text in
  screen space, the same batch and draw calls as everything else (6 draw calls a frame).

## Milestones

| | Milestone | Gate |
| --- | --- | --- |
| M1 | Sprite renderer and stress test (done) | 200,000 sprites at 60 fps on an RTX 3080 Ti |
| M2 | Game foundations: gamepads, audio buses (kira), text and fonts, collision grid and layers, ECS (bevy_ecs on its own), asset loading, particles and trails, dev overlay and profiling (this) | a ship that flies, shoots and blows things up (`--example shooter`) |
| M3 | Horde mode slice: the Commander's flight and guns, fighters, drones, heavies, a destroyer with parts, orbs, explosions and shields in WGSL, a basic HUD | beats the Godot build side by side with 5x the enemies |
| M4 | Full Horde mode: overclocks, supply ship shop, results, menus, English and French, settings, music | a Horde run you'd play in place of the Godot one |
| M5 | Campaign: solar system with floating origin, stations and docking, missions, map, saves, story comics and comms, release builds | |

## Known limits (planned for later milestones)

- One atlas at a time (`Context::set_atlas`). More atlases will add the atlas to the bucket key.
  Fonts live in the atlas, so bake every size the game draws large.
- Text has no kerning, wrapping or right-to-left; enough for a HUD and menus in English and
  French.
- Collision shapes are circles and axis-aligned boxes; rotated boxes and polygons come with the
  destroyer's parts in M3.
- No asset hot reload yet (planned with the `notify` crate when the port needs it).
- No GPU timestamps: GPU-bound time shows up as `present_ms`. Tracy (`--features profile`) shows
  the CPU side.
