@group(0) @binding(0) var texture: texture_2d<f32>;
@group(0) @binding(1) var texture_sampler: sampler;

@group(1) @binding(0) var<uniform> filter_args: BlurArgs;

struct BlurVertexOutput {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) @interpolate(flat) direction: vec2<f32>,
}

fn blur_vertex(in: filter__VertexInput, direction: vec2<f32>) -> BlurVertexOutput {
    let result = filter__main_vertex(in);
    // Pre-shifting the UV coords to put the center of the first trivially
    // sampled (with 1.0 weight, "in the center") pixel at 0.0.
    return BlurVertexOutput(result.position, result.uv - direction * filter_args.m, direction);
}

@vertex
fn main_vertex(in: filter__VertexInput) -> BlurVertexOutput {
    return blur_vertex(in, vec2<f32>(filter_args.dir_x, filter_args.dir_y));
}

@vertex
fn axis_vertex(in: filter__VertexInput) -> BlurVertexOutput {
    let direction = vec2<f32>(filter_args.dir_x, filter_args.dir_y) / vec2<f32>(textureDimensions(texture, 0));
    return blur_vertex(in, direction);
}

@fragment
fn main_fragment(in: BlurVertexOutput) -> @location(0) vec4<f32> {
    return box_blur(texture, texture_sampler, in.uv, filter_args, in.direction);
}
