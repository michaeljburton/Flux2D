use crate::Gpu;
use crate::text::{self, Font, FontId, PendingFont};
use glam::Vec2;
use image::RgbaImage;
use rayon::prelude::*;
use std::collections::HashMap;
use std::fmt;
use std::path::{Path, PathBuf};

/// Transparent pixels kept round every sprite in the atlas, so smaller mip levels don't bleed
/// one sprite's edge into its neighbour.
const PADDING: u32 = 8;
/// Mip levels generated for each layer (1 = no mips).
const MAX_MIPS: u32 = 5;
/// Smallest layer size tried.
const MIN_LAYER: u32 = 256;
/// Largest layer size used, if the GPU allows it.
const MAX_LAYER: u32 = 4096;

/// Handle to one sprite's art in an [`Atlas`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct SpriteId(pub u32);

/// Where a sprite's art sits in the atlas texture array.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SpriteRegion {
    /// Texture array layer.
    pub layer: u32,
    /// `[u_min, v_min, u_max, v_max]` in 0..1.
    pub uv: [f32; 4],
    /// The art's size in pixels.
    pub size: Vec2,
}

#[derive(Debug)]
pub enum AtlasError {
    Image(String),
    Font(String),
    TooBig { name: String, width: u32, height: u32, max: u32 },
    TooManyLayers { needed: u32, max: u32 },
}

impl fmt::Display for AtlasError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            AtlasError::Image(e) => write!(f, "could not load image: {e}"),
            AtlasError::Font(e) => write!(f, "could not load font: {e}"),
            AtlasError::TooBig { name, width, height, max } => {
                write!(f, "{name} is {width}x{height}, larger than the {max} px atlas layer")
            }
            AtlasError::TooManyLayers { needed, max } => {
                write!(f, "atlas needs {needed} layers, the GPU allows {max}")
            }
        }
    }
}

impl std::error::Error for AtlasError {}

/// Collects sprite images and fonts, then packs them into one texture array with
/// [`build`](Self::build).
///
/// Every atlas also gets a plain white sprite ([`Atlas::white`]) for rectangles, lines and
/// bars, and the built-in 8x8 pixel font ([`Atlas::debug_font`]) for the dev overlay.
#[derive(Default)]
pub struct AtlasBuilder {
    images: Vec<(String, RgbaImage)>,
    fonts: Vec<PendingFont>,
}

/// Image file extensions [`AtlasBuilder::add_dir`] picks up.
pub const IMAGE_EXTENSIONS: [&str; 4] = ["png", "jpg", "jpeg", "webp"];

fn load_image(path: &Path, max_size: Option<u32>) -> Result<RgbaImage, AtlasError> {
    let mut img =
        image::open(path).map_err(|e| AtlasError::Image(format!("{}: {e}", path.display())))?.into_rgba8();
    if let Some(max) = max_size {
        let (w, h) = img.dimensions();
        if w > max || h > max {
            let scale = max as f32 / w.max(h) as f32;
            let nw = ((w as f32 * scale).round() as u32).max(1);
            let nh = ((h as f32 * scale).round() as u32).max(1);
            img = image::imageops::resize(&img, nw, nh, image::imageops::FilterType::Triangle);
        }
    }
    Ok(img)
}

fn collect_images(dir: &Path, out: &mut Vec<PathBuf>) -> std::io::Result<()> {
    for entry in std::fs::read_dir(dir)? {
        let path = entry?.path();
        if path.is_dir() {
            collect_images(&path, out)?;
        } else if path
            .extension()
            .and_then(|e| e.to_str())
            .is_some_and(|e| IMAGE_EXTENSIONS.iter().any(|x| x.eq_ignore_ascii_case(e)))
        {
            out.push(path);
        }
    }
    Ok(())
}

impl AtlasBuilder {
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds an image (straight, not premultiplied, alpha) and returns its id.
    pub fn add(&mut self, name: impl Into<String>, image: RgbaImage) -> SpriteId {
        self.images.push((name.into(), image));
        SpriteId(self.images.len() as u32 - 1)
    }

    /// Loads an image file and adds it, scaled down so neither side is over `max_size` pixels
    /// (`None` keeps the full size).
    pub fn add_file(
        &mut self,
        path: impl AsRef<Path>,
        max_size: Option<u32>,
    ) -> Result<SpriteId, AtlasError> {
        let path = path.as_ref();
        let img = load_image(path, max_size)?;
        Ok(self.add(path.display().to_string(), img))
    }

    /// Loads many image files at once, decoding them on every core. Each is named by its path
    /// as given. Results are in the same order as `paths`.
    pub fn add_files<P: AsRef<Path> + Sync>(
        &mut self,
        paths: &[P],
        max_size: Option<u32>,
    ) -> Vec<Result<SpriteId, AtlasError>> {
        let decoded: Vec<_> = paths.par_iter().map(|p| load_image(p.as_ref(), max_size)).collect();
        decoded
            .into_iter()
            .zip(paths)
            .map(|(img, p)| img.map(|img| self.add(p.as_ref().display().to_string(), img)))
            .collect()
    }

    /// Loads every image under `dir` (and its subfolders) in parallel. Each sprite is named by
    /// its path relative to `dir` with `/` separators (`"player/ship.png"`), so
    /// [`Atlas::find`] can look it up. Returns the names and ids, sorted by name.
    pub fn add_dir(
        &mut self,
        dir: impl AsRef<Path>,
        max_size: Option<u32>,
    ) -> Result<Vec<(String, SpriteId)>, AtlasError> {
        let dir = dir.as_ref();
        let mut paths = Vec::new();
        collect_images(dir, &mut paths).map_err(|e| AtlasError::Image(format!("{}: {e}", dir.display())))?;
        paths.sort();
        let decoded: Vec<_> = paths.par_iter().map(|p| load_image(p, max_size)).collect();
        let mut out = Vec::with_capacity(paths.len());
        for (path, img) in paths.iter().zip(decoded) {
            let name = path
                .strip_prefix(dir)
                .unwrap_or(path)
                .components()
                .map(|c| c.as_os_str().to_string_lossy())
                .collect::<Vec<_>>()
                .join("/");
            let id = self.add(name.clone(), img?);
            out.push((name, id));
        }
        Ok(out)
    }

    /// Bakes a TrueType / OpenType font's `chars` (e.g. [`text::LATIN`]) into the atlas at
    /// `px` pixels. Text drawn at `px` is pixel-exact; smaller sizes use the mip levels, larger
    /// ones get soft, so pick the largest size the game draws it at.
    pub fn add_font(&mut self, name: &str, data: &[u8], px: f32, chars: &str) -> Result<FontId, AtlasError> {
        let pending = text::rasterize_ttf(name, data, px, chars, |n, img| self.add(n, img))
            .map_err(AtlasError::Font)?;
        self.fonts.push(pending);
        Ok(FontId(self.fonts.len() as u32 - 1))
    }

    /// [`add_font`](Self::add_font) from a `.ttf` / `.otf` file.
    pub fn add_font_file(
        &mut self,
        path: impl AsRef<Path>,
        px: f32,
        chars: &str,
    ) -> Result<FontId, AtlasError> {
        let path = path.as_ref();
        let data = std::fs::read(path).map_err(|e| AtlasError::Font(format!("{}: {e}", path.display())))?;
        let name =
            path.file_stem().map_or_else(|| path.display().to_string(), |s| s.to_string_lossy().into_owned());
        self.add_font(&name, &data, px, chars)
    }

    pub fn len(&self) -> usize {
        self.images.len()
    }

    pub fn is_empty(&self) -> bool {
        self.images.is_empty()
    }

    /// Packs every image into texture array layers, builds mip levels and uploads them.
    pub fn build(mut self, gpu: &Gpu) -> Result<Atlas, AtlasError> {
        let white = self.add("white", RgbaImage::from_pixel(8, 8, image::Rgba([255; 4])));
        let debug = text::debug_font(|n, img| {
            self.images.push((n, img));
            SpriteId(self.images.len() as u32 - 1)
        });
        self.fonts.push(debug);
        let limits = gpu.device.limits();
        let max_layer = MAX_LAYER.min(limits.max_texture_dimension_2d);
        let max_layers = limits.max_texture_array_layers;
        let sizes: Vec<(u32, u32)> = self.images.iter().map(|(_, i)| i.dimensions()).collect();
        for ((name, _), &(w, h)) in self.images.iter().zip(&sizes) {
            if w + 2 * PADDING > max_layer || h + 2 * PADDING > max_layer {
                return Err(AtlasError::TooBig { name: name.clone(), width: w, height: h, max: max_layer });
            }
        }

        // Smallest square layer that holds everything in one layer, else many max-size layers.
        let largest = sizes.iter().map(|&(w, h)| w.max(h) + 2 * PADDING).max().unwrap_or(1);
        let mut layer_size = largest.next_power_of_two().clamp(MIN_LAYER, max_layer);
        let placements = loop {
            let p = pack(&sizes, layer_size, PADDING).expect("sizes were checked");
            let layers = p.iter().map(|pl| pl.layer + 1).max().unwrap_or(1);
            if layers == 1 || layer_size >= max_layer {
                break p;
            }
            layer_size *= 2;
        };
        let layer_count = placements.iter().map(|p| p.layer + 1).max().unwrap_or(1);
        if layer_count > max_layers {
            return Err(AtlasError::TooManyLayers { needed: layer_count, max: max_layers });
        }

        let mip_count = MAX_MIPS.min(layer_size.ilog2() + 1);
        let texture = gpu.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("flux2d atlas"),
            size: wgpu::Extent3d {
                width: layer_size,
                height: layer_size,
                depth_or_array_layers: layer_count,
            },
            mip_level_count: mip_count,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: Atlas::FORMAT,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });

        let mut regions = vec![SpriteRegion { layer: 0, uv: [0.0; 4], size: Vec2::ZERO }; self.images.len()];
        let inv = 1.0 / layer_size as f32;
        for layer in 0..layer_count {
            let mut canvas = RgbaImage::new(layer_size, layer_size);
            for (i, p) in placements.iter().enumerate().filter(|(_, p)| p.layer == layer) {
                let img = &self.images[i].1;
                image::imageops::replace(&mut canvas, img, i64::from(p.x), i64::from(p.y));
                let (w, h) = img.dimensions();
                regions[i] = SpriteRegion {
                    layer,
                    uv: [p.x as f32 * inv, p.y as f32 * inv, (p.x + w) as f32 * inv, (p.y + h) as f32 * inv],
                    size: Vec2::new(w as f32, h as f32),
                };
            }
            premultiply(&mut canvas);
            let mut level = canvas;
            for mip in 0..mip_count {
                let (w, h) = level.dimensions();
                gpu.queue.write_texture(
                    wgpu::TexelCopyTextureInfo {
                        texture: &texture,
                        mip_level: mip,
                        origin: wgpu::Origin3d { x: 0, y: 0, z: layer },
                        aspect: wgpu::TextureAspect::All,
                    },
                    level.as_raw(),
                    wgpu::TexelCopyBufferLayout {
                        offset: 0,
                        bytes_per_row: Some(4 * w),
                        rows_per_image: Some(h),
                    },
                    wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
                );
                if mip + 1 < mip_count {
                    level = downsample(&level);
                }
            }
        }

        let view = texture.create_view(&wgpu::TextureViewDescriptor {
            label: Some("flux2d atlas view"),
            dimension: Some(wgpu::TextureViewDimension::D2Array),
            ..Default::default()
        });
        // The white sprite samples only its middle texels, so filtering never reaches the
        // transparent padding: a solid 1x1 unit, scaled to any rectangle.
        let w = regions[white.0 as usize];
        let inset = 2.0 * inv;
        regions[white.0 as usize] = SpriteRegion {
            layer: w.layer,
            uv: [w.uv[0] + inset, w.uv[1] + inset, w.uv[2] - inset, w.uv[3] - inset],
            size: Vec2::ONE,
        };
        let fonts: Vec<Font> = self.fonts.into_iter().map(|f| Font::resolve(f, &regions)).collect();
        let names: Vec<String> = self.images.into_iter().map(|(n, _)| n).collect();
        let by_name = names.iter().enumerate().map(|(i, n)| (n.clone(), SpriteId(i as u32))).collect();
        Ok(Atlas { texture, view, regions, names, by_name, fonts, white, layer_size, layer_count, mip_count })
    }
}

/// Sprite art packed into one GPU texture array.
pub struct Atlas {
    pub texture: wgpu::Texture,
    pub view: wgpu::TextureView,
    regions: Vec<SpriteRegion>,
    names: Vec<String>,
    by_name: HashMap<String, SpriteId>,
    fonts: Vec<Font>,
    white: SpriteId,
    layer_size: u32,
    layer_count: u32,
    mip_count: u32,
}

impl Atlas {
    /// Atlas texels are premultiplied alpha in the art's own (sRGB) encoding; blending happens
    /// in that space too, like Godot's default 2D renderer.
    pub const FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;

    pub fn region(&self, id: SpriteId) -> SpriteRegion {
        self.regions[id.0 as usize]
    }

    pub fn name(&self, id: SpriteId) -> &str {
        &self.names[id.0 as usize]
    }

    /// The sprite added under `name` ([`AtlasBuilder::add_dir`] names them by relative path).
    pub fn find(&self, name: &str) -> Option<SpriteId> {
        self.by_name.get(name).copied()
    }

    /// A font added with [`AtlasBuilder::add_font`].
    pub fn font(&self, id: FontId) -> &Font {
        &self.fonts[id.0 as usize]
    }

    /// The built-in 8x8 pixel font, rasterised at 16 px; in every atlas.
    pub fn debug_font(&self) -> &Font {
        self.fonts.last().expect("every atlas has the debug font")
    }

    /// A solid white 1x1 sprite: tint and size it for rectangles, bars and lines.
    pub fn white(&self) -> SpriteRegion {
        self.regions[self.white.0 as usize]
    }

    pub fn len(&self) -> usize {
        self.regions.len()
    }

    pub fn is_empty(&self) -> bool {
        self.regions.is_empty()
    }

    /// `(layer size in pixels, layers, mip levels)`.
    pub fn dimensions(&self) -> (u32, u32, u32) {
        (self.layer_size, self.layer_count, self.mip_count)
    }
}

/// Where one image landed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Placement {
    pub layer: u32,
    pub x: u32,
    pub y: u32,
}

/// Shelf packing: tallest images first, left to right along shelves, a new shelf when a row is
/// full and a new layer when a layer is full. `None` if an image can't fit in an empty layer.
pub(crate) fn pack(sizes: &[(u32, u32)], layer_size: u32, pad: u32) -> Option<Vec<Placement>> {
    let mut order: Vec<usize> = (0..sizes.len()).collect();
    order.sort_by_key(|&i| (std::cmp::Reverse(sizes[i].1), std::cmp::Reverse(sizes[i].0)));
    let mut out = vec![Placement { layer: 0, x: 0, y: 0 }; sizes.len()];
    let (mut layer, mut x, mut y, mut shelf_h) = (0u32, 0u32, 0u32, 0u32);
    for i in order {
        let (w, h) = (sizes[i].0 + 2 * pad, sizes[i].1 + 2 * pad);
        if w > layer_size || h > layer_size {
            return None;
        }
        if x + w > layer_size {
            x = 0;
            y += shelf_h;
            shelf_h = 0;
        }
        if y + h > layer_size {
            layer += 1;
            x = 0;
            y = 0;
            shelf_h = 0;
        }
        out[i] = Placement { layer, x: x + pad, y: y + pad };
        x += w;
        shelf_h = shelf_h.max(h);
    }
    Some(out)
}

fn premultiply(img: &mut RgbaImage) {
    for px in img.pixels_mut() {
        let a = u16::from(px[3]);
        for c in 0..3 {
            px[c] = ((u16::from(px[c]) * a + 127) / 255) as u8;
        }
    }
}

/// Halves an image with a 2x2 box filter (on premultiplied texels, so edges stay clean).
fn downsample(src: &RgbaImage) -> RgbaImage {
    let (w, h) = src.dimensions();
    let (nw, nh) = ((w / 2).max(1), (h / 2).max(1));
    let mut dst = RgbaImage::new(nw, nh);
    for y in 0..nh {
        for x in 0..nw {
            let mut sum = [0u32; 4];
            for (dx, dy) in [(0, 0), (1, 0), (0, 1), (1, 1)] {
                let p = src.get_pixel((2 * x + dx).min(w - 1), (2 * y + dy).min(h - 1));
                for c in 0..4 {
                    sum[c] += u32::from(p[c]);
                }
            }
            dst.put_pixel(x, y, image::Rgba(sum.map(|s| ((s + 2) / 4) as u8)));
        }
    }
    dst
}

#[cfg(test)]
mod tests {
    use super::*;

    fn overlaps(a: (Placement, (u32, u32)), b: (Placement, (u32, u32))) -> bool {
        a.0.layer == b.0.layer
            && a.0.x < b.0.x + b.1.0
            && b.0.x < a.0.x + a.1.0
            && a.0.y < b.0.y + b.1.1
            && b.0.y < a.0.y + a.1.1
    }

    #[test]
    fn packs_without_overlap_inside_layers() {
        let sizes: Vec<(u32, u32)> = (0..200).map(|i| (16 + (i * 37) % 200, 16 + (i * 53) % 180)).collect();
        let p = pack(&sizes, 1024, PADDING).unwrap();
        for i in 0..sizes.len() {
            assert!(p[i].x + sizes[i].0 + PADDING <= 1024 && p[i].y + sizes[i].1 + PADDING <= 1024);
            for j in 0..i {
                assert!(!overlaps((p[i], sizes[i]), (p[j], sizes[j])), "{i} overlaps {j}");
            }
        }
        assert!(p.iter().map(|q| q.layer).max().unwrap() >= 1, "should need several layers");
    }

    #[test]
    fn too_big_is_refused() {
        assert!(pack(&[(300, 10)], 256, PADDING).is_none());
    }

    #[test]
    fn premultiply_and_downsample() {
        let mut img = RgbaImage::from_pixel(4, 4, image::Rgba([255, 0, 0, 128]));
        premultiply(&mut img);
        assert_eq!(*img.get_pixel(0, 0), image::Rgba([128, 0, 0, 128]));
        let half = downsample(&img);
        assert_eq!(half.dimensions(), (2, 2));
        assert_eq!(*half.get_pixel(1, 1), image::Rgba([128, 0, 0, 128]));
    }
}
