//! Reading a frame back from the GPU and saving it as a PNG.

use flux2d_render::{Gpu, wgpu};
use std::path::Path;

pub(crate) struct Readback {
    buffer: wgpu::Buffer,
    width: u32,
    height: u32,
    bytes_per_row: u32,
    bgra: bool,
}

/// Records a copy of `texture` into a mappable buffer. `None` if the texture can't be copied
/// (no `COPY_SRC` usage, or a format other than 8-bit RGBA / BGRA).
pub(crate) fn copy(
    gpu: &Gpu,
    encoder: &mut wgpu::CommandEncoder,
    texture: &wgpu::Texture,
) -> Option<Readback> {
    use wgpu::TextureFormat as F;
    if !texture.usage().contains(wgpu::TextureUsages::COPY_SRC) {
        log::warn!("screenshot: this surface can't be copied");
        return None;
    }
    let bgra = match texture.format() {
        F::Bgra8Unorm | F::Bgra8UnormSrgb => true,
        F::Rgba8Unorm | F::Rgba8UnormSrgb => false,
        other => {
            log::warn!("screenshot: unsupported surface format {other:?}");
            return None;
        }
    };
    let (width, height) = (texture.width(), texture.height());
    let align = wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
    let bytes_per_row = (width * 4).div_ceil(align) * align;
    let buffer = gpu.device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("flux2d screenshot"),
        size: u64::from(bytes_per_row) * u64::from(height),
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    encoder.copy_texture_to_buffer(
        texture.as_image_copy(),
        wgpu::TexelCopyBufferInfo {
            buffer: &buffer,
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(bytes_per_row),
                rows_per_image: Some(height),
            },
        },
        texture.size(),
    );
    Some(Readback { buffer, width, height, bytes_per_row, bgra })
}

impl Readback {
    /// Waits for the GPU and writes the PNG.
    pub(crate) fn save(self, gpu: &Gpu, path: &Path) -> Result<(), Box<dyn std::error::Error>> {
        let slice = self.buffer.slice(..);
        slice.map_async(wgpu::MapMode::Read, |_| {});
        gpu.device.poll(wgpu::PollType::wait_indefinitely())?;
        let data = slice.get_mapped_range()?;
        let mut img = image::RgbaImage::new(self.width, self.height);
        for (y, row) in data.chunks(self.bytes_per_row as usize).take(self.height as usize).enumerate() {
            for x in 0..self.width as usize {
                let p = &row[x * 4..x * 4 + 4];
                let px = if self.bgra { [p[2], p[1], p[0], 255] } else { [p[0], p[1], p[2], 255] };
                img.put_pixel(x as u32, y as u32, image::Rgba(px));
            }
        }
        if let Some(dir) = path.parent().filter(|d| !d.as_os_str().is_empty()) {
            std::fs::create_dir_all(dir)?;
        }
        img.save(path)?;
        Ok(())
    }
}
