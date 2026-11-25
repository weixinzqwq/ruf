@group(0) @binding(0) var texture: texture_2d<f32>;
@group(0) @binding(1) var texture_sampler: sampler;

@group(1) @binding(0) var<uniform> filter_args: BlurArgs;

@vertex
fn main_vertex(in: filter__VertexInput) -> filter__VertexOutput {
    var result = filter__main_vertex(in);

    let direction = vec2<f32>(filter_args.dir_x, filter_args.dir_y);
    // Pre-shifting the UV coords to put the center of the first trivially
    // sampled (with 1.0 weight, "in the center") pixel at 0.0.
    result.uv -= direction * filter_args.m;

    return result;
}

@fragment
fn main_fragment(in: filter__VertexOutput) -> @location(0) vec4<f32> {
    return box_blur(texture, texture_sampler, in.uv, filter_args);
}
