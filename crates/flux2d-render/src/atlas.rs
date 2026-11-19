use crate::Gpu;
use glam::Vec2;
use image::RgbaImage;
use std::fmt;
use std::path::Path;

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
    TooBig { name: String, width: u32, height: u32, max: u32 },
    TooManyLayers { needed: u32, max: u32 },
}

impl fmt::Display for AtlasError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            AtlasError::Image(e) => write!(f, "could not load image: {e}"),
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

/// Collects sprite images, then packs them into one texture array with [`build`](Self::build).
#[derive(Default)]
pub struct AtlasBuilder {
    images: Vec<(String, RgbaImage)>,
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
        let mut img = image::open(path)
            .map_err(|e| AtlasError::Image(format!("{}: {e}", path.display())))?
            .into_rgba8();
        if let Some(max) = max_size {
            let (w, h) = img.dimensions();
            if w > max || h > max {
                let scale = max as f32 / w.max(h) as f32;
                let nw = ((w as f32 * scale).round() as u32).max(1);
                let nh = ((h as f32 * scale).round() as u32).max(1);
                img = image::imageops::resize(&img, nw, nh, image::imageops::FilterType::Triangle);
            }
        }
        Ok(self.add(path.display().to_string(), img))
    }

    pub fn len(&self) -> usize {
        self.images.len()
    }

    pub fn is_empty(&self) -> bool {
        self.images.is_empty()
    }

    /// Packs every image into texture array layers, builds mip levels and uploads them.
    pub fn build(self, gpu: &Gpu) -> Result<Atlas, AtlasError> {
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
        let names = self.images.into_iter().map(|(n, _)| n).collect();
        Ok(Atlas { texture, view, regions, names, layer_size, layer_count, mip_count })
    }
}

/// Sprite art packed into one GPU texture array.
pub struct Atlas {
    pub texture: wgpu::Texture,
    pub view: wgpu::TextureView,
    regions: Vec<SpriteRegion>,
    names: Vec<String>,
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
