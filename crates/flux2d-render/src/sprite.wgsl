// Flux2D instanced sprite shader.
//
// No vertex buffer: each instance is one sprite, and the four corners of its quad come from
// the vertex index (drawn as a 4-vertex triangle strip).

struct Camera {
    view_proj: mat4x4<f32>,
};

@group(0) @binding(0) var<uniform> camera: Camera;
@group(1) @binding(0) var atlas: texture_2d_array<f32>;
@group(1) @binding(1) var atlas_sampler: sampler;

struct Instance {
    @location(0) position: vec2<f32>,
    @location(1) size: vec2<f32>,
    @location(2) uv: vec4<f32>,
    @location(3) rotation: f32,
    @location(4) layer: u32,
    @location(5) color: vec4<f32>,
    @location(6) param: f32,
};

struct VsOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) @interpolate(flat) layer: u32,
    @location(2) color: vec4<f32>,
};

@vertex
fn vs_main(@builtin(vertex_index) vi: u32, inst: Instance) -> VsOut {
    // 0:(0,0) 1:(1,0) 2:(0,1) 3:(1,1)
    let corner = vec2<f32>(f32(vi & 1u), f32(vi >> 1u));
    let local = (corner - vec2<f32>(0.5, 0.5)) * inst.size;
    let c = cos(inst.rotation);
    let s = sin(inst.rotation);
    let world = inst.position + vec2<f32>(local.x * c - local.y * s, local.x * s + local.y * c);

    var out: VsOut;
    out.clip = camera.view_proj * vec4<f32>(world, 0.0, 1.0);
    out.uv = mix(inst.uv.xy, inst.uv.zw, corner);
    out.layer = inst.layer;
    // Tint is straight alpha; premultiply so it matches the atlas texels.
    out.color = vec4<f32>(inst.color.rgb * inst.color.a, inst.color.a);
    return out;
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    let texel = textureSample(atlas, atlas_sampler, in.uv, in.layer);
    return texel * in.color;
}
