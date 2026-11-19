use std::fmt;

/// The wgpu device and queue, plus what adapter they came from.
pub struct Gpu {
    pub instance: wgpu::Instance,
    pub adapter: wgpu::Adapter,
    pub device: wgpu::Device,
    pub queue: wgpu::Queue,
}

#[derive(Debug)]
pub enum GpuError {
    NoAdapter(String),
    Device(String),
}

impl fmt::Display for GpuError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            GpuError::NoAdapter(e) => write!(f, "no suitable GPU adapter: {e}"),
            GpuError::Device(e) => write!(f, "could not open the GPU device: {e}"),
        }
    }
}

impl std::error::Error for GpuError {}

impl Gpu {
    /// Opens the best adapter that can draw to `surface` (or any adapter if `None`).
    ///
    /// Prefers the high-performance (discrete) GPU. `WGPU_BACKEND`, `WGPU_POWER_PREF` and
    /// `WGPU_ADAPTER_NAME` environment variables override the choice, as in wgpu.
    pub async fn new(
        instance: wgpu::Instance,
        surface: Option<&wgpu::Surface<'_>>,
    ) -> Result<Self, GpuError> {
        let adapter = match wgpu::util::initialize_adapter_from_env(&instance, surface).await {
            Ok(a) => a,
            Err(_) => instance
                .request_adapter(&wgpu::RequestAdapterOptions {
                    power_preference: wgpu::PowerPreference::from_env()
                        .unwrap_or(wgpu::PowerPreference::HighPerformance),
                    force_fallback_adapter: false,
                    compatible_surface: surface,
                    ..Default::default()
                })
                .await
                .map_err(|e| GpuError::NoAdapter(e.to_string()))?,
        };
        let limits = wgpu::Limits {
            // Big instance buffers: 1M sprites x 48 bytes = 48 MB.
            max_buffer_size: adapter.limits().max_buffer_size,
            ..wgpu::Limits::downlevel_defaults().using_resolution(adapter.limits())
        };
        let (device, queue) = adapter
            .request_device(&wgpu::DeviceDescriptor {
                label: Some("flux2d device"),
                required_features: wgpu::Features::empty(),
                required_limits: limits,
                ..Default::default()
            })
            .await
            .map_err(|e| GpuError::Device(e.to_string()))?;
        Ok(Self { instance, adapter, device, queue })
    }

    /// A device with no window, for tests and offscreen rendering.
    pub async fn headless() -> Result<Self, GpuError> {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle_from_env());
        Self::new(instance, None).await
    }

    /// "NVIDIA GeForce RTX 3080 Ti (Vulkan)" style name for logs and benchmark output.
    pub fn adapter_name(&self) -> String {
        let info = self.adapter.get_info();
        format!("{} ({:?})", info.name, info.backend)
    }
}
