# Benchmarks

How to measure Flux2D and the numbers so far. Commands are in the [README](../README.md#benchmarks).

## RTX 3080 Ti (Mike's PC, Windows)

Stress sweep after milestone 1 (summary; the full table is still to paste here from
`cargo run --release --example stress -- --sweep`):

- 10,000 to 200,000 sprites: 163 fps, a flat 6.13 ms frame spent almost all in present, so most
  likely a driver or G-Sync cap rather than the engine.
- 1,000,000 sprites: 59 fps; the batch upload took 6.3 ms and drawing 5.3 ms.
- One frame in the 500,000 run stalled for 10.4 s (not investigated yet).

_To measure for milestone 2:_ `cargo run --release --example shooter -- --bench --enemies=2000`
(and 500, 5000), then paste the `FLUX2D SUMMARY` lines here.

## Milestone 2, cloud container (2026-11-22)

Same container and software renderer as below (llvmpipe, 4 vCPU Xeon at 2.1 GHz, Xvfb), so the
frame times are dominated by software rasterising. The CPU fields are the useful part.

**The shooter** (`--bench`, 1280x720, autopilot, 300 measured frames). One update is the whole
game step: bevy_ecs systems for every ship, the collision grid rebuilt, every bolt raycast
against it on all cores, hits and explosions applied, particles moved.

| Enemies | Bolts | Particles | Sprites | Avg frame (ms) | One update (ms) | Draw (ms) | Draw calls |
| ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| 100 | 21 | 291 | 3,409 | 4.14 | 0.00 | 0.12 | 6 |
| 500 | 164 | 894 | 13,765 | 10.53 | 0.25 | 0.72 | 6 |
| 2,000 | 1,069 | 907 | 51,677 | 63.76 | 0.99 | 2.74 | 6 |

Two thousand attacking ships cost about 1 ms of game logic per step and under 3 ms to fill the
batch (stars, trails, bolts, particles and HUD text included) on this slow CPU; the rest of the
frame is the software rasteriser. Godot's worst case today is the Mayday battle's ~185 ships.

**Stress test before and after M2's renderer change** (screen-space sprites through a second
camera matrix bound with a dynamic offset; 640x360, `--zoom=0.05`, 300 frames, same binary
settings, run back to back):

| Sprites | Draw before (ms) | Draw after (ms) | One update before / after (ms) | Draw calls |
| ---: | ---: | ---: | ---: | ---: |
| 100,000 | 1.10 to 1.15 | 1.13 | 0.41 to 0.43 / 0.44 | 2 / 2 |
| 200,000 | 1.89 to 1.98 | 1.87 | 0.52 to 0.57 / 0.53 | 2 / 2 |

No change beyond run-to-run noise: the world-only stress scene still makes 2 draw calls and the
per-sprite path is untouched (a screen-space bucket is just another bucket).

## Cloud container, software renderer (2026-11-19)

The cloud container has no GPU: these runs use Mesa's llvmpipe, which rasterises on the CPU,
under Xvfb on a 4-vCPU Intel Xeon at 2.1 GHz. Frame times are therefore far slower than any real
GPU and the renderer competes with the game for the same 4 cores. They are here to show the
engine's own CPU cost per frame, which is what carries over to real hardware.

**Sprites spread over the whole window** (Cosmic Commando art, 1280x720, zoom 1, vsync off, 60
measured frames). Fill-rate bound on a software rasteriser:

| Sprites | Avg frame (ms) | p95 (ms) | p99 (ms) | FPS | Draw (ms) | Draw calls |
| ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| 10,000 | 68.05 | 79.65 | 82.72 | 15 | 0.12 | 2 |
| 50,000 | 331.76 | 355.16 | 365.36 | 3 | 0.61 | 2 |
| 100,000 | 647.71 | 674.32 | 704.80 | 2 | 1.09 | 2 |
| 200,000 | 1255.19 | 1306.76 | 1315.85 | 1 | 2.13 | 2 |

**Tiny sprites** (640x360, `--zoom=0.05`, so each sprite covers a few pixels and the software
rasteriser does little fill work): the engine's own per-frame cost.

| Sprites | Avg frame (ms) | One update (ms) | Draw (ms) | Draw calls | Uploaded per frame |
| ---: | ---: | ---: | ---: | ---: | ---: |
| 100,000 | 59.72 | 0.40 | 1.20 | 2 | 4.8 MB |
| 200,000 | 98.68 | 0.48 | 1.71 | 2 | 9.6 MB |
| 500,000 | 239.43 | 0.94 | 4.17 | 2 | 24 MB |
| 1,000,000 | 470.33 | 1.53 | 8.20 | 2 | 48 MB |

What this says:

- The draw call count stays at 2 however many sprites there are.
- At 200,000 sprites the engine's CPU work (one update to move every sprite plus filling the
  batch) is about 2.2 ms of the 16.7 ms a 60 fps frame allows, even on this slow, shared CPU.
  The rest of a frame on real hardware is the GPU's fill rate and the upload.
- At 1,000,000 sprites the CPU side is about 10 ms here, so a million is within reach on a
  desktop CPU; the GPU's fill rate decides the rest.
