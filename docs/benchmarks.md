# Benchmarks

How to measure Flux2D and the numbers so far. Commands are in the [README](../README.md#benchmarks).

## RTX 3080 Ti (Mike's PC)

_To measure:_ `cargo run --release --example stress -- --sweep` at 1920x1080, then paste the
table it prints here and the 200k row into the README.

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
