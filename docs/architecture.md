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

## Milestones

| | Milestone | Gate |
| --- | --- | --- |
| M1 | Sprite renderer and stress test (this) | 200,000 sprites at 60 fps on an RTX 3080 Ti |
| M2 | Game foundations: gamepads, audio buses (kira), text and fonts, collision grid and layers, ECS (bevy_ecs on its own), asset loading, particles and trails, dev overlay and profiling | a ship that flies, shoots and blows things up |
| M3 | Horde mode slice: the Commander's flight and guns, fighters, drones, heavies, a destroyer with parts, orbs, explosions and shields in WGSL, a basic HUD | beats the Godot build side by side with 5x the enemies |
| M4 | Full Horde mode: overclocks, supply ship shop, results, menus, English and French, settings, music | a Horde run you'd play in place of the Godot one |
| M5 | Campaign: solar system with floating origin, stations and docking, missions, map, saves, story comics and comms, release builds | |

## Known limits of M1 (planned for later milestones)

- One atlas at a time (`Context::set_atlas`). More atlases will add the atlas to the bucket key.
- Input edges (`key_pressed`) last one displayed frame; read them in `draw`, not in `update`,
  until the input layer latches them for fixed steps (M2).
- No text yet: the stress test reports through the window title and the console.
- No GPU timestamps: GPU-bound time shows up as `present_ms`.
