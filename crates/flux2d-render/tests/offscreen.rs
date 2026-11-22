//! Draws a few sprites into an offscreen texture and checks the pixels.
//!
//! Needs a GPU adapter. CI uses Mesa's software Vulkan driver (lavapipe); on a machine with no
//! adapter at all the test prints why and passes, so `cargo test` still works anywhere.

use flux2d_render::{
    AtlasBuilder, Blend, Camera2D, Gpu, SpriteBatch, SpriteInstance, SpriteRenderer, TextStyle, wgpu,
};
use glam::Vec2;
use image::{Rgba, RgbaImage};

const W: u32 = 64;
const H: u32 = 32;
const FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;

/// Renders with a camera that makes world units window pixels.
fn render(gpu: &Gpu, renderer: &mut SpriteRenderer, batch: &SpriteBatch) -> RgbaImage {
    let mut camera = Camera2D::new(Vec2::new(W as f32, H as f32));
    camera.position = Vec2::new(W as f32 / 2.0, H as f32 / 2.0); // world = pixels
    render_with(gpu, renderer, batch, &camera)
}

fn render_with(
    gpu: &Gpu,
    renderer: &mut SpriteRenderer,
    batch: &SpriteBatch,
    camera: &Camera2D,
) -> RgbaImage {
    let target = gpu.device.create_texture(&wgpu::TextureDescriptor {
        label: Some("test target"),
        size: wgpu::Extent3d { width: W, height: H, depth_or_array_layers: 1 },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: FORMAT,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    });
    let view = target.create_view(&Default::default());
    let upload = renderer.prepare(gpu, batch, camera);
    assert_eq!(upload.sprites as usize, batch.len());

    let bytes_per_row = 256; // >= W * 4, aligned to COPY_BYTES_PER_ROW_ALIGNMENT
    let readback = gpu.device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("readback"),
        size: u64::from(bytes_per_row * H),
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let mut enc = gpu.device.create_command_encoder(&Default::default());
    {
        let mut pass = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("test pass"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: &view,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color { r: 0.0, g: 0.0, b: 1.0, a: 1.0 }),
                    store: wgpu::StoreOp::Store,
                },
            })],
            ..Default::default()
        });
        renderer.draw(&mut pass);
    }
    enc.copy_texture_to_buffer(
        target.as_image_copy(),
        wgpu::TexelCopyBufferInfo {
            buffer: &readback,
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(bytes_per_row),
                rows_per_image: Some(H),
            },
        },
        wgpu::Extent3d { width: W, height: H, depth_or_array_layers: 1 },
    );
    gpu.queue.submit([enc.finish()]);
    readback.slice(..).map_async(wgpu::MapMode::Read, |r| r.unwrap());
    gpu.device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
    let data = readback.slice(..).get_mapped_range().expect("mapped");
    let mut img = RgbaImage::new(W, H);
    for y in 0..H {
        for x in 0..W {
            let i = (y * bytes_per_row + x * 4) as usize;
            img.put_pixel(x, y, Rgba([data[i], data[i + 1], data[i + 2], data[i + 3]]));
        }
    }
    img
}

fn close(a: Rgba<u8>, b: [u8; 4]) -> bool {
    a.0.iter().zip(b).all(|(&x, y)| x.abs_diff(y) <= 3)
}

#[test]
fn sprites_draw_with_alpha_and_additive_blending() {
    let gpu = match pollster::block_on(Gpu::headless()) {
        Ok(g) => g,
        Err(e) => {
            eprintln!("skipping: {e}");
            return;
        }
    };
    eprintln!("adapter: {}", gpu.adapter_name());

    let mut atlas = AtlasBuilder::new();
    let white = atlas.add("white", RgbaImage::from_pixel(8, 8, Rgba([255, 255, 255, 255])));
    let half = atlas.add("half red", RgbaImage::from_pixel(8, 8, Rgba([255, 0, 0, 128])));
    let atlas = atlas.build(&gpu).unwrap();

    let mut renderer = SpriteRenderer::new(&gpu, FORMAT);
    renderer.set_atlas(&gpu, &atlas);

    let mut batch = SpriteBatch::new();
    let w = atlas.region(white);
    // Left: an opaque red square (white art tinted red) 16x16 at (12, 16).
    batch.push(
        0,
        Blend::Alpha,
        SpriteInstance::sized(&w, Vec2::new(12.0, 16.0), Vec2::splat(16.0), 0.0).with_color([255, 0, 0, 255]),
    );
    // Middle: half-transparent red art over the blue background.
    batch.push(
        0,
        Blend::Alpha,
        SpriteInstance::sized(&atlas.region(half), Vec2::new(32.0, 16.0), Vec2::splat(16.0), 0.0),
    );
    // Right: additive green on blue gives cyan; pushed on a lower layer but drawn after alpha.
    batch.push(
        0,
        Blend::Additive,
        SpriteInstance::sized(&w, Vec2::new(52.0, 16.0), Vec2::splat(16.0), 0.0).with_color([0, 255, 0, 255]),
    );

    let img = render(&gpu, &mut renderer, &batch);
    assert!(close(*img.get_pixel(12, 16), [255, 0, 0, 255]), "opaque: {:?}", img.get_pixel(12, 16));
    assert!(close(*img.get_pixel(32, 16), [128, 0, 127, 255]), "alpha: {:?}", img.get_pixel(32, 16));
    assert!(close(*img.get_pixel(52, 16), [0, 255, 255, 255]), "additive: {:?}", img.get_pixel(52, 16));
    assert!(close(*img.get_pixel(1, 1), [0, 0, 255, 255]), "background: {:?}", img.get_pixel(1, 1));
    // Edges land where the camera says: (12,16) +- 8 px.
    assert!(close(*img.get_pixel(5, 16), [255, 0, 0, 255]) && close(*img.get_pixel(3, 16), [0, 0, 255, 255]));
}

#[test]
fn rotation_turns_the_quad() {
    let Ok(gpu) = pollster::block_on(Gpu::headless()) else { return };
    let mut atlas = AtlasBuilder::new();
    let white = atlas.add("white", RgbaImage::from_pixel(4, 4, Rgba([255, 255, 255, 255])));
    let atlas = atlas.build(&gpu).unwrap();
    let mut renderer = SpriteRenderer::new(&gpu, FORMAT);
    renderer.set_atlas(&gpu, &atlas);
    let mut batch = SpriteBatch::new();
    // A 40x4 bar at the centre, turned a quarter turn: tall instead of wide.
    batch.push(
        0,
        Blend::Alpha,
        SpriteInstance::sized(
            &atlas.region(white),
            Vec2::new(32.0, 16.0),
            Vec2::new(40.0, 4.0),
            std::f32::consts::FRAC_PI_2,
        ),
    );
    let img = render(&gpu, &mut renderer, &batch);
    assert!(close(*img.get_pixel(32, 2), [255, 255, 255, 255]), "{:?}", img.get_pixel(32, 2));
    assert!(close(*img.get_pixel(44, 16), [0, 0, 255, 255]), "{:?}", img.get_pixel(44, 16));
}

#[test]
fn screen_space_ignores_the_camera_and_text_draws() {
    let Ok(gpu) = pollster::block_on(Gpu::headless()) else { return };
    let atlas = AtlasBuilder::new().build(&gpu).unwrap();
    let mut renderer = SpriteRenderer::new(&gpu, FORMAT);
    renderer.set_atlas(&gpu, &atlas);
    let white = atlas.white();

    // A world camera far away, zoomed and turned: world sprites at the origin are off screen.
    let mut camera = Camera2D::new(Vec2::new(W as f32, H as f32));
    camera.position = Vec2::new(5000.0, -300.0);
    camera.zoom = 3.0;
    camera.rotation = 0.5;

    let mut batch = SpriteBatch::new();
    batch.push(0, Blend::Alpha, SpriteInstance::sized(&white, Vec2::new(10.0, 10.0), Vec2::splat(8.0), 0.0));
    // Screen space: a red 8x8 square centred on pixel (8, 8), whatever the camera does.
    batch.push_screen(
        0,
        Blend::Alpha,
        SpriteInstance::sized(&white, Vec2::new(8.0, 8.0), Vec2::splat(8.0), 0.0)
            .with_color([255, 0, 0, 255]),
    );
    // Text at (32, 8) in the built-in font at its raster size: a white "#" (solid rows).
    atlas.debug_font().draw(&mut batch, "#", Vec2::new(32.0, 8.0), TextStyle::screen(16.0));

    let img = render_with(&gpu, &mut renderer, &batch, &camera);
    assert!(close(*img.get_pixel(8, 8), [255, 0, 0, 255]), "screen square: {:?}", img.get_pixel(8, 8));
    assert!(close(*img.get_pixel(13, 8), [0, 0, 255, 255]), "square is 8 px: {:?}", img.get_pixel(13, 8));
    let lit = (32..48)
        .flat_map(|x| (8..24).map(move |y| (x, y)))
        .filter(|&(x, y)| img.get_pixel(x, y)[1] > 200)
        .count();
    assert!(lit > 40, "the glyph drew {lit} white pixels");
    assert!(close(*img.get_pixel(60, 28), [0, 0, 255, 255]), "background: {:?}", img.get_pixel(60, 28));
}
