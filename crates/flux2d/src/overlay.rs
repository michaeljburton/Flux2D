//! The dev overlay: frame timings, a frame-time graph and the game's own debug lines, drawn
//! in screen space with the atlas's built-in font. F3 shows and hides it.

use crate::FrameTimings;
use flux2d_render::{Blend, Font, SpriteBatch, SpriteInstance, SpriteRegion, TextStyle};
use glam::Vec2;
use std::fmt::Write;

/// Frames kept for the graph and the averages.
const HISTORY: usize = 240;
/// Draw layer of the overlay (screen space, so it is over the world anyway).
const LAYER: u8 = 254;

/// The dev overlay's state. Reach it as `ctx.overlay`.
pub struct DevOverlay {
    /// Shown or not (F3 toggles).
    pub visible: bool,
    /// Extra lines from the game, shown under the timings. Cleared after every frame; write
    /// into it with `write!` / `writeln!` so nothing allocates once it has grown.
    pub text: String,
    frames: [f32; HISTORY],
    next: usize,
    filled: usize,
    line: String,
    gpu_name: String,
}

impl DevOverlay {
    pub(crate) fn new(visible: bool, gpu_name: String) -> Self {
        Self {
            visible,
            text: String::new(),
            frames: [0.0; HISTORY],
            next: 0,
            filled: 0,
            line: String::with_capacity(256),
            gpu_name,
        }
    }

    pub(crate) fn record(&mut self, frame_ms: f32) {
        self.frames[self.next] = frame_ms;
        self.next = (self.next + 1) % HISTORY;
        self.filled = (self.filled + 1).min(HISTORY);
    }

    /// Average and worst frame time over the graph's history.
    pub fn recent(&self) -> (f32, f32) {
        if self.filled == 0 {
            return (0.0, 0.0);
        }
        let s = &self.frames[..self.filled];
        (s.iter().sum::<f32>() / s.len() as f32, s.iter().copied().fold(0.0, f32::max))
    }

    pub(crate) fn draw(
        &mut self,
        batch: &mut SpriteBatch,
        font: &Font,
        white: &SpriteRegion,
        t: &FrameTimings,
    ) {
        let (avg, worst) = self.recent();
        let style = TextStyle::screen(font.raster_size()).layer(LAYER);
        let line_h = font.line_height(style.size);
        let pad = 8.0;
        let graph_h = 60.0;
        let graph_w = HISTORY as f32 * 2.0;
        let lines = 5 + self.text.lines().count();
        let panel = Vec2::new(graph_w.max(560.0) + 2.0 * pad, lines as f32 * line_h + graph_h + 3.0 * pad);
        let rect = |batch: &mut SpriteBatch, pos: Vec2, size: Vec2, color: [u8; 4]| {
            batch.push_screen(
                LAYER,
                Blend::Alpha,
                SpriteInstance::sized(white, pos + size * 0.5, size, 0.0).with_color(color),
            );
        };
        rect(batch, Vec2::ZERO, panel, [0, 0, 0, 170]);

        let mut y = pad;
        let mut text = |batch: &mut SpriteBatch, s: &str, color: [u8; 4]| {
            font.draw(batch, s, Vec2::new(pad, y), style.color(color));
            y += line_h;
        };
        let fps = if avg > 0.0 { 1000.0 / avg } else { 0.0 };
        let l = &mut self.line;
        l.clear();
        let _ = write!(l, "FPS {fps:.0}   frame {avg:.2} ms   worst {worst:.2} ms");
        let color = if worst > 1000.0 / 30.0 {
            [255, 110, 90, 255]
        } else if worst > 1000.0 / 60.0 {
            [255, 220, 90, 255]
        } else {
            [140, 255, 140, 255]
        };
        text(batch, l, color);
        l.clear();
        let _ = write!(
            l,
            "update {:.2} (x{})  draw {:.2}  upload {:.2}  present {:.2}",
            t.update_ms, t.updates, t.draw_ms, t.upload_ms, t.present_ms
        );
        text(batch, l, [255; 4]);
        l.clear();
        let _ = write!(
            l,
            "sprites {}  draw calls {}  upload {:.1} MB",
            thousands(t.upload.sprites),
            t.upload.draw_calls,
            t.upload.bytes as f32 / 1e6
        );
        text(batch, l, [255; 4]);
        text(batch, &self.gpu_name, [170, 190, 255, 255]);
        text(batch, "F3 hides this", [150, 150, 150, 255]);
        for game_line in self.text.lines() {
            text(batch, game_line, [255, 230, 160, 255]);
        }

        // Frame-time graph: one bar per frame, oldest on the left; lines at 16.7 and 33.3 ms.
        let top = y + pad;
        let base = top + graph_h;
        let scale = graph_h / 40.0; // px per ms: the graph tops out at 40 ms
        rect(batch, Vec2::new(pad, top), Vec2::new(graph_w, graph_h), [255, 255, 255, 20]);
        for (ms, color) in [(1000.0 / 60.0, [140, 255, 140, 90]), (1000.0 / 30.0, [255, 220, 90, 90])] {
            rect(batch, Vec2::new(pad, base - ms * scale), Vec2::new(graph_w, 1.0), color);
        }
        for i in 0..self.filled {
            let idx = (self.next + HISTORY - self.filled + i) % HISTORY;
            let ms = self.frames[idx];
            let h = (ms * scale).clamp(1.0, graph_h);
            let color = if ms > 1000.0 / 30.0 {
                [255, 90, 70, 230]
            } else if ms > 1000.0 / 60.0 {
                [255, 210, 70, 230]
            } else {
                [90, 220, 120, 230]
            };
            rect(batch, Vec2::new(pad + i as f32 * 2.0, base - h), Vec2::new(2.0, h), color);
        }
    }
}

/// 1234567 -> "1,234,567" without allocating per digit group.
pub(crate) fn thousands(n: u32) -> Thousands {
    Thousands(n)
}

pub(crate) struct Thousands(u32);

impl std::fmt::Display for Thousands {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let n = self.0;
        if n >= 1_000_000 {
            write!(f, "{},{:03},{:03}", n / 1_000_000, n / 1000 % 1000, n % 1000)
        } else if n >= 1000 {
            write!(f, "{},{:03}", n / 1000, n % 1000)
        } else {
            write!(f, "{n}")
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn thousands_groups() {
        assert_eq!(thousands(7).to_string(), "7");
        assert_eq!(thousands(12_345).to_string(), "12,345");
        assert_eq!(thousands(1_000_001).to_string(), "1,000,001");
    }

    #[test]
    fn history_averages() {
        let mut o = DevOverlay::new(true, String::new());
        assert_eq!(o.recent(), (0.0, 0.0));
        for i in 0..(HISTORY + 10) {
            o.record(if i % 2 == 0 { 10.0 } else { 20.0 });
        }
        let (avg, worst) = o.recent();
        assert!((avg - 15.0).abs() < 1e-3);
        assert_eq!(worst, 20.0);
    }
}
