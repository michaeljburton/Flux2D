# Flux2D: notes for coding agents

The one set of instructions for every coding agent in this repo (`CLAUDE.md` only imports it).

## What this is

Flux2D is a 2D game engine in Rust built to draw very large numbers of sprites. Its test case is
a port of Mike's Godot game Cosmic Commando (repo `mrhatefulgames/Cosmic-Commando`, game in
`cosmic-commando-2d/`). The plan and milestones are in `docs/architecture.md`.

## Layout

| Crate | Job |
| --- | --- |
| `crates/flux2d-core` | fixed timestep, input state, frame stats; no GPU |
| `crates/flux2d-render` | wgpu: `Gpu`, `Atlas` (texture array), `SpriteBatch`, `SpriteRenderer`, `Camera2D`, `sprite.wgsl` |
| `crates/flux2d` | the app loop (`run`, `Game`, `Context`), screenshots; re-exports the others; examples |

## Rules

- **Performance first.** Per-sprite data stays plain `Copy` structs in `Vec`s (no per-sprite
  heap objects, no trait objects in hot loops). Anything that runs per sprite per frame must not
  allocate. Prefer filling `SpriteBatch::alloc` slices in parallel with `rayon`.
- **Measure.** A change to the renderer or the loop comes with before / after numbers from
  `cargo run --release --example stress -- --bench --sprites=N` (same N, same machine).
- **Check your work:** `cargo fmt --all`, `cargo clippy --workspace --all-targets -- -D warnings`,
  `cargo test --workspace` (the GPU tests in `crates/flux2d-render/tests/` run on lavapipe in a
  container: `apt-get install mesa-vulkan-drivers`). For anything visual, take a screenshot and
  look at it: `xvfb-run -a ./target/release/examples/stress --shot=out.png` (needs
  `libxkbcommon-x11-0` in a container).
- **Docs:** keep `README.md` and `docs/` in step with the code in the same PR.
- Blending happens in the art's sRGB encoding with premultiplied alpha (like Godot 2D), so the
  port matches the game's look. Don't switch the swapchain to an sRGB format without a reason.
- Commit messages: a short summary line, then a paragraph on what changed and why.
