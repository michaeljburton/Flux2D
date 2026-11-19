use crate::{Atlas, Blend, Camera2D, DrawRun, Gpu, SpriteBatch, SpriteInstance};
use std::mem::size_of;

/// What one [`SpriteRenderer::prepare`] uploaded.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct FrameUpload {
    pub sprites: u32,
    pub draw_calls: u32,
    pub bytes: u64,
}

/// Draws [`SpriteBatch`]es: one buffer write and one instanced draw per run per frame.
pub struct SpriteRenderer {
    pipelines: [wgpu::RenderPipeline; 2],
    camera_buffer: wgpu::Buffer,
    camera_group: wgpu::BindGroup,
    texture_layout: wgpu::BindGroupLayout,
    texture_group: Option<wgpu::BindGroup>,
    sampler: wgpu::Sampler,
    instances: wgpu::Buffer,
    capacity: u64,
    runs: Vec<DrawRun>,
}

const INSTANCE_ATTRIBUTES: [wgpu::VertexAttribute; 7] = wgpu::vertex_attr_array![
    0 => Float32x2, // position
    1 => Float32x2, // size
    2 => Float32x4, // uv
    3 => Float32,   // rotation
    4 => Uint32,    // layer
    5 => Unorm8x4,  // color
    6 => Float32,   // param
];

impl SpriteRenderer {
    /// A renderer that draws into textures of `target_format`.
    pub fn new(gpu: &Gpu, target_format: wgpu::TextureFormat) -> Self {
        let device = &gpu.device;
        let shader = device.create_shader_module(wgpu::include_wgsl!("sprite.wgsl"));

        let camera_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("flux2d camera layout"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            }],
        });
        let texture_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("flux2d atlas layout"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2Array,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });
        let camera_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("flux2d camera"),
            size: 64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let camera_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("flux2d camera group"),
            layout: &camera_layout,
            entries: &[wgpu::BindGroupEntry { binding: 0, resource: camera_buffer.as_entire_binding() }],
        });
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("flux2d atlas sampler"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::MipmapFilterMode::Linear,
            ..Default::default()
        });

        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("flux2d sprite pipeline layout"),
            bind_group_layouts: &[Some(&camera_layout), Some(&texture_layout)],
            immediate_size: 0,
        });
        let pipeline = |blend: Blend| {
            let state = match blend {
                Blend::Alpha => wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING,
                Blend::Additive => wgpu::BlendState {
                    color: wgpu::BlendComponent {
                        src_factor: wgpu::BlendFactor::One,
                        dst_factor: wgpu::BlendFactor::One,
                        operation: wgpu::BlendOperation::Add,
                    },
                    alpha: wgpu::BlendComponent {
                        src_factor: wgpu::BlendFactor::Zero,
                        dst_factor: wgpu::BlendFactor::One,
                        operation: wgpu::BlendOperation::Add,
                    },
                },
            };
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(match blend {
                    Blend::Alpha => "flux2d sprites (alpha)",
                    Blend::Additive => "flux2d sprites (additive)",
                }),
                layout: Some(&layout),
                vertex: wgpu::VertexState {
                    module: &shader,
                    entry_point: Some("vs_main"),
                    compilation_options: Default::default(),
                    buffers: &[Some(wgpu::VertexBufferLayout {
                        array_stride: size_of::<SpriteInstance>() as u64,
                        step_mode: wgpu::VertexStepMode::Instance,
                        attributes: &INSTANCE_ATTRIBUTES,
                    })],
                },
                primitive: wgpu::PrimitiveState {
                    topology: wgpu::PrimitiveTopology::TriangleStrip,
                    cull_mode: None,
                    ..Default::default()
                },
                depth_stencil: None,
                multisample: wgpu::MultisampleState::default(),
                fragment: Some(wgpu::FragmentState {
                    module: &shader,
                    entry_point: Some("fs_main"),
                    compilation_options: Default::default(),
                    targets: &[Some(wgpu::ColorTargetState {
                        format: target_format,
                        blend: Some(state),
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                }),
                multiview_mask: None,
                cache: None,
            })
        };
        let pipelines = [pipeline(Blend::Alpha), pipeline(Blend::Additive)];
        let capacity = 1024;
        let instances = Self::instance_buffer(device, capacity);
        Self {
            pipelines,
            camera_buffer,
            camera_group,
            texture_layout,
            texture_group: None,
            sampler,
            instances,
            capacity,
            runs: Vec::new(),
        }
    }

    fn instance_buffer(device: &wgpu::Device, capacity: u64) -> wgpu::Buffer {
        device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("flux2d sprite instances"),
            size: capacity * size_of::<SpriteInstance>() as u64,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        })
    }

    /// Uses `atlas` for every sprite drawn from now on.
    pub fn set_atlas(&mut self, gpu: &Gpu, atlas: &Atlas) {
        self.texture_group = Some(gpu.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("flux2d atlas group"),
            layout: &self.texture_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&atlas.view),
                },
                wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::Sampler(&self.sampler) },
            ],
        }));
    }

    /// Uploads the camera and every sprite in `batch`; call once per frame before [`draw`](Self::draw).
    pub fn prepare(&mut self, gpu: &Gpu, batch: &SpriteBatch, camera: &Camera2D) -> FrameUpload {
        gpu.queue.write_buffer(
            &self.camera_buffer,
            0,
            bytemuck::cast_slice(&camera.view_proj().to_cols_array()),
        );
        batch.runs(&mut self.runs);
        let count = self.runs.iter().map(|r| u64::from(r.count)).sum::<u64>();
        if count == 0 {
            return FrameUpload::default();
        }
        if count > self.capacity {
            self.capacity = count.next_power_of_two();
            self.instances = Self::instance_buffer(&gpu.device, self.capacity);
        }
        let stride = size_of::<SpriteInstance>();
        let bytes = count * stride as u64;
        // Write straight into wgpu's staging memory: no intermediate copy of the frame.
        let mut view = gpu
            .queue
            .write_buffer_with(&self.instances, 0, wgpu::BufferSize::new(bytes).expect("count > 0"))
            .expect("instance buffer is large enough");
        let mut offset = 0usize;
        for (_, sprites) in batch.buckets() {
            let src: &[u8] = bytemuck::cast_slice(sprites);
            view.slice(offset..offset + src.len()).copy_from_slice(src);
            offset += src.len();
        }
        drop(view);
        FrameUpload { sprites: count as u32, draw_calls: self.runs.len() as u32, bytes }
    }

    /// Records the draws for the last [`prepare`](Self::prepare)d batch into `pass`.
    ///
    /// Does nothing until an atlas is set.
    pub fn draw(&self, pass: &mut wgpu::RenderPass<'_>) {
        let Some(textures) = &self.texture_group else { return };
        if self.runs.is_empty() {
            return;
        }
        pass.set_bind_group(0, &self.camera_group, &[]);
        pass.set_bind_group(1, textures, &[]);
        pass.set_vertex_buffer(0, self.instances.slice(..));
        let mut current = None;
        for run in &self.runs {
            if current != Some(run.blend) {
                pass.set_pipeline(&self.pipelines[run.blend as usize]);
                current = Some(run.blend);
            }
            pass.draw(0..4, run.start..run.start + run.count);
        }
    }
}
