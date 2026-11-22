//! Text drawn as sprites: every glyph is baked into the atlas once, so a line of text is just
//! more instances in the same batch (and usually the same draw call) as everything else.

use crate::{Blend, SpriteBatch, SpriteId, SpriteInstance, SpriteRegion, batch::Space};
use glam::Vec2;
use image::{Rgba, RgbaImage};

/// Handle to a font added to an [`AtlasBuilder`](crate::AtlasBuilder).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct FontId(pub u32);

/// Printable ASCII plus the Latin-1 letters and signs French (and most western European
/// languages) need: `é è ê ë à â ç ô ù û ü ï î œ « »` and friends.
pub const LATIN: &str = concat!(
    " !\"#$%&'()*+,-./0123456789:;<=>?@ABCDEFGHIJKLMNOPQRSTUVWXYZ[\\]^_`abcdefghijklmnopqrstuvwxyz{|}~",
    "¡¢£¤¥¦§¨©ª«¬®¯°±²³´µ¶·¸¹º»¼½¾¿ÀÁÂÃÄÅÆÇÈÉÊËÌÍÎÏÐÑÒÓÔÕÖ×ØÙÚÛÜÝÞßàáâãäåæçèéêëìíîïðñòóôõö÷øùúûüýþÿ",
    "ŒœŸ€…‘’“”–—•"
);

/// One glyph's art and metrics, in the font's raster pixels.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Glyph {
    /// `None` for blank glyphs (space).
    pub region: Option<SpriteRegion>,
    /// Top-left of the art relative to the pen on the baseline (y down).
    pub offset: Vec2,
    /// Size of the art.
    pub size: Vec2,
    /// How far the pen moves after this glyph.
    pub advance: f32,
}

/// A glyph before the atlas is built.
#[derive(Debug, Clone)]
pub(crate) struct PendingGlyph {
    pub ch: char,
    pub sprite: Option<SpriteId>,
    pub offset: Vec2,
    pub size: Vec2,
    pub advance: f32,
}

#[derive(Debug, Clone)]
pub(crate) struct PendingFont {
    pub name: String,
    pub px: f32,
    pub ascent: f32,
    pub descent: f32,
    pub line_gap: f32,
    pub glyphs: Vec<PendingGlyph>,
}

/// A font baked into an atlas at one raster size; draws at any size (mip-mapped below it).
///
/// Get it from [`Atlas::font`](crate::Atlas::font) or [`Atlas::debug_font`](crate::Atlas::debug_font).
#[derive(Debug, Clone)]
pub struct Font {
    name: String,
    px: f32,
    ascent: f32,
    descent: f32,
    line_gap: f32,
    /// Glyph index for chars 32..128.
    ascii: [u16; 96],
    /// Other chars, sorted, for a binary search.
    others: Vec<(char, u16)>,
    glyphs: Vec<Glyph>,
    fallback: u16,
}

/// Where a block of text sits relative to the position given to [`Font::draw_aligned`].
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Align(pub Vec2);

impl Align {
    pub const TOP_LEFT: Align = Align(Vec2::new(0.0, 0.0));
    pub const TOP: Align = Align(Vec2::new(0.5, 0.0));
    pub const TOP_RIGHT: Align = Align(Vec2::new(1.0, 0.0));
    pub const CENTER: Align = Align(Vec2::new(0.5, 0.5));
    pub const BOTTOM_LEFT: Align = Align(Vec2::new(0.0, 1.0));
    pub const BOTTOM: Align = Align(Vec2::new(0.5, 1.0));
    pub const BOTTOM_RIGHT: Align = Align(Vec2::new(1.0, 1.0));
}

/// How to draw a piece of text.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TextStyle {
    /// Text size in world units (pixels for [`Space::Screen`]): the font's raster size draws
    /// it pixel for pixel.
    pub size: f32,
    /// RGBA, straight alpha.
    pub color: [u8; 4],
    pub space: Space,
    pub layer: u8,
    pub blend: Blend,
}

impl TextStyle {
    /// White text on the screen (in pixels, top-left origin), on draw layer 250.
    pub fn screen(size: f32) -> Self {
        Self { size, color: [255; 4], space: Space::Screen, layer: 250, blend: Blend::Alpha }
    }

    /// White text in the world, on draw layer 200.
    pub fn world(size: f32) -> Self {
        Self { size, color: [255; 4], space: Space::World, layer: 200, blend: Blend::Alpha }
    }

    pub fn color(mut self, rgba: [u8; 4]) -> Self {
        self.color = rgba;
        self
    }

    pub fn layer(mut self, layer: u8) -> Self {
        self.layer = layer;
        self
    }
}

impl Font {
    pub(crate) fn resolve(p: PendingFont, regions: &[SpriteRegion]) -> Font {
        let glyphs: Vec<Glyph> = p
            .glyphs
            .iter()
            .map(|g| Glyph {
                region: g.sprite.map(|id| regions[id.0 as usize]),
                offset: g.offset,
                size: g.size,
                advance: g.advance,
            })
            .collect();
        let find = |c: char| p.glyphs.iter().position(|g| g.ch == c).map(|i| i as u16);
        let fallback = find('?').or_else(|| find(' ')).unwrap_or(0);
        let mut ascii = [fallback; 96];
        for (i, slot) in ascii.iter_mut().enumerate() {
            if let Some(g) = find(char::from(32 + i as u8)) {
                *slot = g;
            }
        }
        let mut others: Vec<(char, u16)> = p
            .glyphs
            .iter()
            .enumerate()
            .filter(|(_, g)| !(' '..='\x7f').contains(&g.ch))
            .map(|(i, g)| (g.ch, i as u16))
            .collect();
        others.sort_unstable_by_key(|&(c, _)| c);
        Font {
            name: p.name,
            px: p.px,
            ascent: p.ascent,
            descent: p.descent,
            line_gap: p.line_gap,
            ascii,
            others,
            glyphs,
            fallback,
        }
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    /// The size the glyphs were rasterised at; text drawn at this size is pixel-exact.
    pub fn raster_size(&self) -> f32 {
        self.px
    }

    /// Distance between baselines at text size `size`.
    pub fn line_height(&self, size: f32) -> f32 {
        (self.ascent - self.descent + self.line_gap) * size / self.px
    }

    /// The glyph for `c` (a `?` if the font doesn't have it).
    #[inline]
    pub fn glyph(&self, c: char) -> &Glyph {
        let code = c as u32;
        let idx = if (32..128).contains(&code) {
            self.ascii[(code - 32) as usize]
        } else {
            match self.others.binary_search_by_key(&c, |&(ch, _)| ch) {
                Ok(i) => self.others[i].1,
                Err(_) => self.fallback,
            }
        };
        &self.glyphs[idx as usize]
    }

    /// Width and height of `text` at `size` (lines split on `\n`).
    pub fn measure(&self, text: &str, size: f32) -> Vec2 {
        let scale = size / self.px;
        let mut widest = 0.0f32;
        let mut lines = 0;
        for line in text.split('\n') {
            lines += 1;
            widest = widest.max(line.chars().map(|c| self.glyph(c).advance).sum::<f32>() * scale);
        }
        Vec2::new(widest, self.line_height(size) * lines as f32 - self.line_gap * scale)
    }

    /// Draws `text` with its top-left corner at `pos`; returns the size drawn.
    /// Allocates nothing: each glyph is one sprite pushed to `batch`.
    pub fn draw(&self, batch: &mut SpriteBatch, text: &str, pos: Vec2, style: TextStyle) -> Vec2 {
        let scale = style.size / self.px;
        let snap = style.space == Space::Screen && (scale - scale.round()).abs() < 1e-3;
        let origin = if snap { pos.round() } else { pos };
        let line_h = self.line_height(style.size);
        let mut pen = Vec2::new(origin.x, origin.y + self.ascent * scale);
        let mut widest = 0.0f32;
        let mut lines = 0;
        for line in text.split('\n') {
            lines += 1;
            for c in line.chars() {
                let g = self.glyph(c);
                if let Some(region) = &g.region {
                    let size = g.size * scale;
                    let centre = pen + g.offset * scale + size * 0.5;
                    let sprite = SpriteInstance::sized(region, centre, size, 0.0).with_color(style.color);
                    batch.push_in(style.space, style.layer, style.blend, sprite);
                }
                pen.x += g.advance * scale;
            }
            widest = widest.max(pen.x - origin.x);
            pen.x = origin.x;
            pen.y += line_h;
        }
        Vec2::new(widest, line_h * lines as f32 - self.line_gap * scale)
    }

    /// Draws `text` placed by `align` round `pos` (e.g. [`Align::CENTER`] centres it on `pos`).
    pub fn draw_aligned(
        &self,
        batch: &mut SpriteBatch,
        text: &str,
        pos: Vec2,
        align: Align,
        style: TextStyle,
    ) -> Vec2 {
        let size = self.measure(text, style.size);
        self.draw(batch, text, pos - size * align.0, style)
    }
}

/// Rasterises a TrueType / OpenType font's `chars` at `px` into images for the atlas.
pub(crate) fn rasterize_ttf(
    name: &str,
    data: &[u8],
    px: f32,
    chars: &str,
    mut add: impl FnMut(String, RgbaImage) -> SpriteId,
) -> Result<PendingFont, String> {
    use ab_glyph::{Font as _, FontRef, PxScale, ScaleFont, point};
    let font = FontRef::try_from_slice(data).map_err(|e| format!("{name}: {e}"))?;
    let scaled = font.as_scaled(PxScale::from(px));
    let mut glyphs = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for ch in chars.chars().chain(['?']) {
        if !seen.insert(ch) {
            continue;
        }
        let id = font.glyph_id(ch);
        if id.0 == 0 && ch != ' ' {
            continue; // not in this font: falls back to '?'
        }
        let advance = scaled.h_advance(id);
        let glyph = id.with_scale_and_position(PxScale::from(px), point(0.0, 0.0));
        let (sprite, offset, size) = match font.outline_glyph(glyph) {
            Some(outline) => {
                let b = outline.px_bounds();
                let (w, h) = (b.width() as u32, b.height() as u32);
                if w == 0 || h == 0 {
                    (None, Vec2::ZERO, Vec2::ZERO)
                } else {
                    let mut img = RgbaImage::new(w, h);
                    outline.draw(|x, y, c| {
                        if x < w && y < h {
                            img.put_pixel(
                                x,
                                y,
                                Rgba([255, 255, 255, (c.clamp(0.0, 1.0) * 255.0).round() as u8]),
                            );
                        }
                    });
                    let id = add(format!("{name} '{ch}'"), img);
                    (Some(id), Vec2::new(b.min.x, b.min.y), Vec2::new(w as f32, h as f32))
                }
            }
            None => (None, Vec2::ZERO, Vec2::ZERO),
        };
        glyphs.push(PendingGlyph { ch, sprite, offset, size, advance });
    }
    Ok(PendingFont {
        name: name.to_string(),
        px,
        ascent: scaled.ascent(),
        descent: scaled.descent(),
        line_gap: scaled.line_gap(),
        glyphs,
    })
}

/// Pixel scale of the built-in 8x8 debug font in the atlas.
const DEBUG_SCALE: u32 = 2;

/// The built-in 8x8 pixel font (public domain `font8x8`), ASCII and Latin-1, at 16 px.
pub(crate) fn debug_font(mut add: impl FnMut(String, RgbaImage) -> SpriteId) -> PendingFont {
    use font8x8::UnicodeFonts;
    let s = DEBUG_SCALE;
    let cell = 8 * s;
    let mut glyphs = Vec::new();
    let chars = (' '..='~').chain('\u{a0}'..='\u{ff}');
    for ch in chars {
        let rows = font8x8::BASIC_FONTS.get(ch).or_else(|| font8x8::LATIN_FONTS.get(ch));
        let Some(rows) = rows else { continue };
        let blank = rows.iter().all(|&r| r == 0);
        let sprite = (!blank).then(|| {
            let img = RgbaImage::from_fn(cell, cell, |x, y| {
                let on = rows[(y / s) as usize] & (1 << (x / s)) != 0;
                Rgba([255, 255, 255, if on { 255 } else { 0 }])
            });
            add(format!("debug font '{ch}'"), img)
        });
        glyphs.push(PendingGlyph {
            ch,
            sprite,
            // Baseline under row 7 of 8: rows 0..=6 sit above it.
            offset: Vec2::new(0.0, -(7.0 * s as f32)),
            size: Vec2::splat(cell as f32),
            advance: cell as f32,
        });
    }
    PendingFont {
        name: "debug 8x8".into(),
        px: cell as f32,
        ascent: 7.0 * s as f32,
        descent: -(1.0 * s as f32),
        line_gap: 2.0 * s as f32,
        glyphs,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fake_font() -> Font {
        let mut n = 0;
        let pending = debug_font(|_, _| {
            n += 1;
            SpriteId(n - 1)
        });
        let regions = vec![SpriteRegion { layer: 0, uv: [0.0; 4], size: Vec2::splat(16.0) }; n as usize];
        Font::resolve(pending, &regions)
    }

    #[test]
    fn debug_font_measures_and_draws() {
        let f = fake_font();
        assert_eq!(f.raster_size(), 16.0);
        assert_eq!(f.glyph('A').advance, 16.0);
        assert!(f.glyph(' ').region.is_none());
        assert!(f.glyph('é').region.is_some(), "Latin-1 letters are in");
        assert_eq!(f.glyph('\u{2603}'), f.glyph('?'), "unknown chars fall back to ?");
        let size = f.measure("ab\nabcd", 32.0);
        assert_eq!(size.x, 4.0 * 32.0);
        let mut batch = SpriteBatch::new();
        let drawn = f.draw(&mut batch, "Hi there", Vec2::new(10.0, 20.0), TextStyle::screen(16.0));
        assert_eq!(batch.len(), 7, "one sprite per visible glyph");
        assert_eq!(drawn.x, 8.0 * 16.0);
        let (space, _, sprites) = batch.buckets().next().unwrap();
        assert_eq!(space, Space::Screen);
        // 'H' sits with its top-left at the given position (cell is 16 px, centred at +8).
        assert_eq!(sprites[0].position, [18.0, 28.0]);
    }

    #[test]
    fn ttf_rasterises_when_a_font_is_around() {
        let candidates = [
            "/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf",
            "/usr/share/fonts/TTF/DejaVuSans.ttf",
            "C:\\Windows\\Fonts\\arial.ttf",
            "/System/Library/Fonts/Supplemental/Arial.ttf",
        ];
        let Some(data) = candidates.iter().find_map(|p| std::fs::read(p).ok()) else {
            eprintln!("no system font found; skipping");
            return;
        };
        let mut images = Vec::new();
        let pending = rasterize_ttf("test", &data, 32.0, LATIN, |_, img| {
            images.push(img);
            SpriteId(images.len() as u32 - 1)
        })
        .unwrap();
        assert!(pending.ascent > 20.0 && pending.descent < 0.0);
        let regions = vec![SpriteRegion { layer: 0, uv: [0.0; 4], size: Vec2::ONE }; images.len()];
        let font = Font::resolve(pending, &regions);
        let w = font.glyph('W');
        assert!(w.region.is_some() && w.size.x > 20.0, "{w:?}");
        assert!(font.glyph('é').region.is_some());
        assert!(font.measure("iiii", 32.0).x < font.measure("WWWW", 32.0).x, "proportional");
        // Some pixels of 'W' are solid.
        assert!(images.iter().any(|i| i.pixels().any(|p| p[3] == 255)));
    }
}
