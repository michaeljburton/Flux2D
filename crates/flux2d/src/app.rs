use crate::overlay::DevOverlay;
use crate::screenshot;
use flux2d_audio::Audio;
#[cfg(feature = "gamepad")]
use flux2d_core::GamepadPoller;
use flux2d_core::{FixedTimestep, Input, KeyCode, profile_frame, profile_scope};
use flux2d_render::{
    Atlas, Camera2D, Font, FrameUpload, Gpu, SpriteBatch, SpriteRegion, SpriteRenderer, wgpu,
};
use glam::Vec2;
use std::sync::Arc;
use std::time::Instant;
use winit::application::ApplicationHandler;
use winit::dpi::PhysicalSize;
use winit::event::{ElementState, MouseScrollDelta, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::keyboard::PhysicalKey;
use winit::window::{Window, WindowId};

/// Window and loop settings.
#[derive(Debug, Clone)]
pub struct Config {
    pub title: String,
    pub width: u32,
    pub height: u32,
    /// Wait for the display's refresh (no tearing, capped frame rate).
    pub vsync: bool,
    /// Fixed updates per second.
    pub update_hz: f64,
    /// Background colour, linear 0..1 RGBA.
    pub clear_color: [f64; 4],
    /// Show the dev overlay from the start (F3 toggles it either way).
    pub dev_overlay: bool,
    /// Open the audio device. Off, `ctx.audio` is silent.
    pub audio: bool,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            title: "Flux2D".into(),
            width: 1920,
            height: 1080,
            vsync: true,
            update_hz: 60.0,
            clear_color: [0.01, 0.01, 0.03, 1.0],
            dev_overlay: false,
            audio: true,
        }
    }
}

/// Where the last frame's time went, in milliseconds of CPU time on the main thread.
#[derive(Debug, Clone, Copy, Default)]
pub struct FrameTimings {
    /// Wall-clock time since the previous frame started.
    pub frame_ms: f32,
    /// All fixed updates run this frame.
    pub update_ms: f32,
    /// The game's `draw` (filling the sprite batch).
    pub draw_ms: f32,
    /// Copying the batch to the GPU and recording the draws.
    pub upload_ms: f32,
    /// Waiting for the next swapchain image and presenting (GPU-bound time shows up here).
    pub present_ms: f32,
    pub updates: u32,
    pub upload: FrameUpload,
}

/// Everything a game can reach: the GPU, input, audio, the camera and timings.
pub struct Context {
    pub gpu: Gpu,
    pub window: Arc<Window>,
    /// Keyboard, mouse and gamepads. Edges (`key_pressed`...) read in `update` mean "since the
    /// last update"; read anywhere else they mean "this frame".
    pub input: Input,
    pub audio: Audio,
    pub camera: Camera2D,
    pub clear_color: [f64; 4],
    /// The last frame's timings.
    pub timings: FrameTimings,
    /// The dev overlay (F3). Write extra lines into `overlay.text` during `update` or `draw`.
    pub overlay: DevOverlay,
    renderer: SpriteRenderer,
    #[cfg(feature = "gamepad")]
    gamepads: GamepadPoller,
    debug_font: Option<Font>,
    white: Option<SpriteRegion>,
    surface: wgpu::Surface<'static>,
    surface_config: wgpu::SurfaceConfiguration,
    exit: bool,
    vsync_changed: bool,
    screenshot: Option<std::path::PathBuf>,
}

impl Context {
    /// Saves the next frame to a PNG file at `path` (if the window surface allows copying).
    pub fn request_screenshot(&mut self, path: impl Into<std::path::PathBuf>) {
        self.screenshot = Some(path.into());
    }

    /// Draw every sprite from `atlas` from now on.
    pub fn set_atlas(&mut self, atlas: &Atlas) {
        self.renderer.set_atlas(&self.gpu, atlas);
        self.debug_font = Some(atlas.debug_font().clone());
        self.white = Some(atlas.white());
    }

    /// The current atlas's built-in 8x8 font (none before [`set_atlas`](Self::set_atlas)).
    pub fn debug_font(&self) -> Option<&Font> {
        self.debug_font.as_ref()
    }

    /// The current atlas's solid white sprite.
    pub fn white(&self) -> Option<SpriteRegion> {
        self.white
    }

    /// Ends the loop after this frame.
    pub fn exit(&mut self) {
        self.exit = true;
    }

    pub fn vsync(&self) -> bool {
        self.surface_config.present_mode == wgpu::PresentMode::AutoVsync
    }

    pub fn set_vsync(&mut self, on: bool) {
        let mode = if on { wgpu::PresentMode::AutoVsync } else { wgpu::PresentMode::AutoNoVsync };
        if self.surface_config.present_mode != mode {
            self.surface_config.present_mode = mode;
            self.vsync_changed = true;
        }
    }

    /// Window size in physical pixels.
    pub fn window_size(&self) -> Vec2 {
        Vec2::new(self.surface_config.width as f32, self.surface_config.height as f32)
    }

    fn resize(&mut self, size: PhysicalSize<u32>) {
        if size.width == 0 || size.height == 0 {
            return;
        }
        self.surface_config.width = size.width;
        self.surface_config.height = size.height;
        self.surface.configure(&self.gpu.device, &self.surface_config);
        self.camera.viewport = Vec2::new(size.width as f32, size.height as f32);
    }
}

/// A game: a fixed-rate `update` and a once-per-frame `draw`.
pub trait Game: 'static {
    /// One fixed step of `dt` seconds.
    fn update(&mut self, ctx: &mut Context, dt: f32);

    /// Fill `batch` with this frame's sprites. `alpha` (0..1) is how far the clock is between
    /// the last update and the next, for interpolating positions. The batch starts empty.
    fn draw(&mut self, ctx: &mut Context, batch: &mut SpriteBatch, alpha: f32);

    /// Called after the frame is presented, with its timings in `ctx.timings`.
    fn frame_end(&mut self, _ctx: &mut Context) {}
}

/// Opens the window and runs `make_game`'s game until it exits or the window closes.
///
/// `make_game` runs once the GPU is ready, so it can load textures.
pub fn run<G, F>(config: Config, make_game: F) -> Result<(), Box<dyn std::error::Error>>
where
    G: Game,
    F: FnOnce(&mut Context) -> G + 'static,
{
    let event_loop = EventLoop::new()?;
    event_loop.set_control_flow(ControlFlow::Poll);
    let mut runner = Runner::<G, F> { config, make_game: Some(make_game), state: None, error: None };
    event_loop.run_app(&mut runner)?;
    match runner.error {
        Some(e) => Err(e),
        None => Ok(()),
    }
}

struct State<G> {
    ctx: Context,
    game: G,
    batch: SpriteBatch,
    clock: FixedTimestep,
    last_frame: Instant,
}

struct Runner<G, F> {
    config: Config,
    make_game: Option<F>,
    state: Option<State<G>>,
    error: Option<Box<dyn std::error::Error>>,
}

impl<G: Game, F: FnOnce(&mut Context) -> G> Runner<G, F> {
    fn start(&mut self, event_loop: &ActiveEventLoop) -> Result<(), Box<dyn std::error::Error>> {
        let attrs = Window::default_attributes()
            .with_title(self.config.title.clone())
            .with_inner_size(PhysicalSize::new(self.config.width, self.config.height));
        let window = Arc::new(event_loop.create_window(attrs)?);
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_with_display_handle_from_env(
            Box::new(event_loop.owned_display_handle()),
        ));
        let surface = instance.create_surface(window.clone())?;
        let gpu = pollster::block_on(Gpu::new(instance, Some(&surface)))?;
        log::info!("GPU: {}", gpu.adapter_name());

        let caps = surface.get_capabilities(&gpu.adapter);
        // Blend in the art's own encoding (like Godot 2D): prefer a non-sRGB swapchain format.
        let format = caps.formats.iter().copied().find(|f| !f.is_srgb()).unwrap_or(caps.formats[0]);
        let size = window.inner_size();
        // COPY_SRC lets `request_screenshot` read the frame back, where the platform allows it.
        let usage = wgpu::TextureUsages::RENDER_ATTACHMENT | (caps.usages & wgpu::TextureUsages::COPY_SRC);
        let surface_config = wgpu::SurfaceConfiguration {
            usage,
            format,
            width: size.width.max(1),
            height: size.height.max(1),
            present_mode: if self.config.vsync {
                wgpu::PresentMode::AutoVsync
            } else {
                wgpu::PresentMode::AutoNoVsync
            },
            desired_maximum_frame_latency: 2,
            alpha_mode: caps.alpha_modes[0],
            view_formats: vec![],
            color_space: Default::default(),
        };
        surface.configure(&gpu.device, &surface_config);
        let renderer = SpriteRenderer::new(&gpu, format);
        #[cfg_attr(not(feature = "gamepad"), allow(unused_mut))]
        let mut input = Input::default();
        #[cfg(feature = "gamepad")]
        let gamepads = GamepadPoller::new(&mut input);
        let audio = if self.config.audio { Audio::new() } else { Audio::silent() };
        flux2d_core::start_profiler();
        let overlay = DevOverlay::new(self.config.dev_overlay, gpu.adapter_name());
        let mut ctx = Context {
            camera: Camera2D::new(Vec2::new(surface_config.width as f32, surface_config.height as f32)),
            gpu,
            window,
            input,
            audio,
            clear_color: self.config.clear_color,
            timings: FrameTimings::default(),
            overlay,
            renderer,
            #[cfg(feature = "gamepad")]
            gamepads,
            debug_font: None,
            white: None,
            surface,
            surface_config,
            exit: false,
            vsync_changed: false,
            screenshot: None,
        };
        let make_game = self.make_game.take().expect("started once");
        let game = make_game(&mut ctx);
        self.state = Some(State {
            ctx,
            game,
            batch: SpriteBatch::new(),
            clock: FixedTimestep::new(self.config.update_hz),
            last_frame: Instant::now(),
        });
        Ok(())
    }
}

impl<G: Game, F: FnOnce(&mut Context) -> G> ApplicationHandler for Runner<G, F> {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.state.is_none()
            && self.make_game.is_some()
            && let Err(e) = self.start(event_loop)
        {
            self.error = Some(e);
            event_loop.exit();
        }
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, _id: WindowId, event: WindowEvent) {
        let Some(state) = self.state.as_mut() else { return };
        let ctx = &mut state.ctx;
        match event {
            WindowEvent::CloseRequested => event_loop.exit(),
            WindowEvent::Resized(size) => ctx.resize(size),
            WindowEvent::Focused(false) => ctx.input.release_all(),
            WindowEvent::KeyboardInput { event, .. } => {
                if let PhysicalKey::Code(code) = event.physical_key {
                    ctx.input.set_key(code, event.state == ElementState::Pressed);
                }
            }
            WindowEvent::MouseInput { state: s, button, .. } => {
                ctx.input.set_mouse_button(button, s == ElementState::Pressed);
            }
            WindowEvent::CursorMoved { position, .. } => {
                ctx.input.set_mouse_position(Vec2::new(position.x as f32, position.y as f32));
            }
            WindowEvent::MouseWheel { delta, .. } => ctx.input.add_wheel(match delta {
                MouseScrollDelta::LineDelta(_, y) => y,
                MouseScrollDelta::PixelDelta(p) => p.y as f32 / 40.0,
            }),
            WindowEvent::RedrawRequested => {
                state.frame();
                if state.ctx.exit {
                    event_loop.exit();
                }
            }
            _ => {}
        }
    }

    fn about_to_wait(&mut self, _event_loop: &ActiveEventLoop) {
        if let Some(state) = &self.state {
            state.ctx.window.request_redraw();
        }
    }
}

impl<G: Game> State<G> {
    fn frame(&mut self) {
        let start = Instant::now();
        let frame_ms = (start - self.last_frame).as_secs_f32() * 1000.0;
        self.last_frame = start;
        let ctx = &mut self.ctx;

        if ctx.vsync_changed {
            ctx.surface.configure(&ctx.gpu.device, &ctx.surface_config);
            ctx.vsync_changed = false;
        }
        #[cfg(feature = "gamepad")]
        ctx.gamepads.poll(&mut ctx.input);
        if ctx.input.key_pressed(KeyCode::F3) {
            ctx.overlay.visible = !ctx.overlay.visible;
        }
        ctx.overlay.text.clear();

        let steps = self.clock.advance(f64::from(frame_ms) / 1000.0);
        let dt = self.clock.step_seconds() as f32;
        for _ in 0..steps {
            profile_scope!("update");
            ctx.input.begin_step();
            self.game.update(ctx, dt);
            ctx.input.end_step();
        }
        let t_update = Instant::now();

        self.batch.clear();
        {
            profile_scope!("draw");
            self.game.draw(ctx, &mut self.batch, self.clock.alpha());
        }
        if ctx.overlay.visible
            && let (Some(font), Some(white)) = (&ctx.debug_font, &ctx.white)
        {
            ctx.overlay.draw(&mut self.batch, font, white, &ctx.timings);
        }
        let t_draw = Instant::now();

        let surface_texture = match ctx.surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(t) => t,
            wgpu::CurrentSurfaceTexture::Suboptimal(t) => {
                ctx.vsync_changed = true; // reconfigure next frame
                t
            }
            wgpu::CurrentSurfaceTexture::Outdated | wgpu::CurrentSurfaceTexture::Lost => {
                ctx.surface.configure(&ctx.gpu.device, &ctx.surface_config);
                ctx.input.end_frame();
                return;
            }
            other => {
                log::warn!("skipping frame: {other:?}");
                ctx.input.end_frame();
                return;
            }
        };
        let t_acquired = Instant::now();

        let upload = {
            profile_scope!("upload");
            let upload = ctx.renderer.prepare(&ctx.gpu, &self.batch, &ctx.camera);
            let view = surface_texture.texture.create_view(&Default::default());
            let mut encoder = ctx
                .gpu
                .device
                .create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("flux2d frame") });
            {
                let [r, g, b, a] = ctx.clear_color;
                let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("flux2d sprites"),
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: &view,
                        depth_slice: None,
                        resolve_target: None,
                        ops: wgpu::Operations {
                            load: wgpu::LoadOp::Clear(wgpu::Color { r, g, b, a }),
                            store: wgpu::StoreOp::Store,
                        },
                    })],
                    ..Default::default()
                });
                ctx.renderer.draw(&mut pass);
            }
            let shot = ctx.screenshot.take().map(|path| {
                let buffer = screenshot::copy(&ctx.gpu, &mut encoder, &surface_texture.texture);
                (path, buffer)
            });
            ctx.gpu.queue.submit([encoder.finish()]);
            if let Some((path, Some(readback))) = shot {
                match readback.save(&ctx.gpu, &path) {
                    Ok(()) => log::info!("saved screenshot {}", path.display()),
                    Err(e) => log::error!("screenshot {}: {e}", path.display()),
                }
            }
            upload
        };
        let t_submitted = Instant::now();
        {
            profile_scope!("present");
            ctx.window.pre_present_notify();
            ctx.gpu.queue.present(surface_texture);
        }
        let t_presented = Instant::now();
        profile_frame!();

        let ms = |a: Instant, b: Instant| (b - a).as_secs_f32() * 1000.0;
        ctx.timings = FrameTimings {
            frame_ms,
            update_ms: ms(start, t_update),
            draw_ms: ms(t_update, t_draw),
            upload_ms: ms(t_acquired, t_submitted),
            present_ms: ms(t_draw, t_acquired) + ms(t_submitted, t_presented),
            updates: steps,
            upload,
        };
        ctx.overlay.record(frame_ms);
        self.game.frame_end(ctx);
        ctx.input.end_frame();
    }
}
