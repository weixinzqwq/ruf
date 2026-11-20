/// Shader used for drawing bitmap fills.
/// NOTE: The `common.wgsl` source is prepended to this before compilation.

struct VertexOutput {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) mult_color: vec4<f32>,
    @location(2) add_color: vec4<f32>,
    @location(3) @interpolate(flat) upscale: f32,
};

@group(1) @binding(0) var<uniform> transforms: array<common__Transforms, max_transforms>;
@group(2) @binding(0) var texture: texture_2d<f32>;
@group(2) @binding(1) var texture_sampler: sampler;
override late_saturate: bool = false;

fn safe_reciprocal(value: f32) -> f32 {
    if (value > 0.00000001) {
        return 1.0 / value;
    }
    return 0.0;
}

struct EasuSet {
    dir: vec2<f32>,
    strength: f32,
}

struct EasuTap {
    color: vec4<f32>,
    weight: f32,
}

// FSR 1 EASU, adapted from AMD FidelityFX-FSR (MIT).
// https://github.com/GPUOpen-Effects/FidelityFX-FSR
fn easu_set(accumulator: EasuSet, weight: f32, a: f32, b: f32, c: f32, d: f32, e: f32) -> EasuSet {
    if (weight <= 0.0) {
        return accumulator;
    }
    let dc = d - c;
    let cb = c - b;
    let len_x = safe_reciprocal(max(abs(dc), abs(cb)));
    let dir_x = d - b;
    let ec = e - c;
    let ca = c - a;
    let len_y = safe_reciprocal(max(abs(ec), abs(ca)));
    let dir_y = e - a;
    let shaped_x = saturate(abs(dir_x) * len_x);
    let shaped_y = saturate(abs(dir_y) * len_y);
    return EasuSet(
        accumulator.dir + vec2<f32>(dir_x, dir_y) * weight,
        accumulator.strength + (shaped_x * shaped_x + shaped_y * shaped_y) * weight,
    );
}

fn easu_tap(
    tap: EasuTap,
    offset: vec2<f32>,
    dir: vec2<f32>,
    len: vec2<f32>,
    lobe: f32,
    clip: f32,
    color: vec4<f32>,
) -> EasuTap {
    let rotated = vec2<f32>(
        offset.x * dir.x + offset.y * dir.y,
        offset.x * -dir.y + offset.y * dir.x,
    ) * len;
    let distance = min(dot(rotated, rotated), clip);
    let base = 0.4 * distance - 1.0;
    let window = lobe * distance - 1.0;
    let weight = (1.5625 * base * base - 0.5625) * window * window;
    return EasuTap(tap.color + color * weight, tap.weight + weight);
}

fn easu_sample(uv: vec2<f32>) -> vec4<f32> {
    let size = vec2<f32>(textureDimensions(texture));
    let projected = uv * size - vec2<f32>(0.5);
    // Canonicalize texel-center roundoff so an integer coordinate cannot select different tap neighborhoods.
    let coord = select(projected, round(projected), abs(projected - round(projected)) < vec2<f32>(0.00001));
    let base = floor(coord);
    let fraction = coord - base;
    let b = textureSampleLevel(texture, texture_sampler, (base + vec2<f32>(0.0, -1.0) + 0.5) / size, 0.0);
    let c = textureSampleLevel(texture, texture_sampler, (base + vec2<f32>(1.0, -1.0) + 0.5) / size, 0.0);
    let i = textureSampleLevel(texture, texture_sampler, (base + vec2<f32>(-1.0, 1.0) + 0.5) / size, 0.0);
    let j = textureSampleLevel(texture, texture_sampler, (base + vec2<f32>(0.0, 1.0) + 0.5) / size, 0.0);
    let f = textureSampleLevel(texture, texture_sampler, (base + vec2<f32>(0.0, 0.0) + 0.5) / size, 0.0);
    let e = textureSampleLevel(texture, texture_sampler, (base + vec2<f32>(-1.0, 0.0) + 0.5) / size, 0.0);
    let k = textureSampleLevel(texture, texture_sampler, (base + vec2<f32>(1.0, 1.0) + 0.5) / size, 0.0);
    let l = textureSampleLevel(texture, texture_sampler, (base + vec2<f32>(2.0, 1.0) + 0.5) / size, 0.0);
    let h = textureSampleLevel(texture, texture_sampler, (base + vec2<f32>(2.0, 0.0) + 0.5) / size, 0.0);
    let g = textureSampleLevel(texture, texture_sampler, (base + vec2<f32>(1.0, 0.0) + 0.5) / size, 0.0);
    let o = textureSampleLevel(texture, texture_sampler, (base + vec2<f32>(1.0, 2.0) + 0.5) / size, 0.0);
    let n = textureSampleLevel(texture, texture_sampler, (base + vec2<f32>(0.0, 2.0) + 0.5) / size, 0.0);
    let luma = vec4<f32>(
        b.r * 0.5 + b.g + b.b * 0.5,
        c.r * 0.5 + c.g + c.b * 0.5,
        i.r * 0.5 + i.g + i.b * 0.5,
        j.r * 0.5 + j.g + j.b * 0.5,
    );
    let luma_f = vec4<f32>(
        f.r * 0.5 + f.g + f.b * 0.5,
        e.r * 0.5 + e.g + e.b * 0.5,
        k.r * 0.5 + k.g + k.b * 0.5,
        l.r * 0.5 + l.g + l.b * 0.5,
    );
    let luma_g = vec4<f32>(
        h.r * 0.5 + h.g + h.b * 0.5,
        g.r * 0.5 + g.g + g.b * 0.5,
        o.r * 0.5 + o.g + o.b * 0.5,
        n.r * 0.5 + n.g + n.b * 0.5,
    );
    var state = EasuSet(vec2<f32>(0.0), 0.0);
    state = easu_set(state, (1.0 - fraction.x) * (1.0 - fraction.y), luma.x, luma_f.y, luma_f.x, luma_g.y, luma.w);
    state = easu_set(state, fraction.x * (1.0 - fraction.y), luma.y, luma_f.x, luma_g.y, luma_g.x, luma_f.z);
    state = easu_set(state, (1.0 - fraction.x) * fraction.y, luma_f.x, luma.z, luma.w, luma_f.z, luma_g.w);
    state = easu_set(state, fraction.x * fraction.y, luma_g.y, luma.w, luma_f.z, luma_f.w, luma_g.z);
    let dir_len_sq = dot(state.dir, state.dir);
    var dir = state.dir;
    if (dir_len_sq < 0.000030517578125) {
        dir = vec2<f32>(1.0, 0.0);
    } else {
        dir = dir * inverseSqrt(dir_len_sq);
    }
    let edge_length = state.strength * 0.5;
    let strength = edge_length * edge_length;
    let stretch = safe_reciprocal(max(abs(dir.x), abs(dir.y)));
    let len = vec2<f32>(1.0 + (stretch - 1.0) * strength, 1.0 - 0.5 * strength);
    let lobe = 0.5 - 0.29 * strength;
    let clip = safe_reciprocal(lobe);
    let min4 = min(min(f, g), min(j, k));
    let max4 = max(max(f, g), max(j, k));
    var tap = EasuTap(vec4<f32>(0.0), 0.0);
    tap = easu_tap(tap, vec2<f32>(0.0, -1.0) - fraction, dir, len, lobe, clip, b);
    tap = easu_tap(tap, vec2<f32>(1.0, -1.0) - fraction, dir, len, lobe, clip, c);
    tap = easu_tap(tap, vec2<f32>(-1.0, 1.0) - fraction, dir, len, lobe, clip, i);
    tap = easu_tap(tap, vec2<f32>(0.0, 1.0) - fraction, dir, len, lobe, clip, j);
    tap = easu_tap(tap, vec2<f32>(0.0, 0.0) - fraction, dir, len, lobe, clip, f);
    tap = easu_tap(tap, vec2<f32>(-1.0, 0.0) - fraction, dir, len, lobe, clip, e);
    tap = easu_tap(tap, vec2<f32>(1.0, 1.0) - fraction, dir, len, lobe, clip, k);
    tap = easu_tap(tap, vec2<f32>(2.0, 1.0) - fraction, dir, len, lobe, clip, l);
    tap = easu_tap(tap, vec2<f32>(2.0, 0.0) - fraction, dir, len, lobe, clip, h);
    tap = easu_tap(tap, vec2<f32>(1.0, 0.0) - fraction, dir, len, lobe, clip, g);
    tap = easu_tap(tap, vec2<f32>(1.0, 2.0) - fraction, dir, len, lobe, clip, o);
    tap = easu_tap(tap, vec2<f32>(0.0, 2.0) - fraction, dir, len, lobe, clip, n);
    var color = clamp(tap.color * safe_reciprocal(tap.weight), min4, max4);
    color = saturate(vec4<f32>(min(color.rgb, vec3<f32>(color.a)), color.a));
    return color;
}

fn integer_small_bitmap_scale(uv: vec2<f32>, dx: vec2<f32>, dy: vec2<f32>, size: vec2<f32>) -> bool {
    // dx/dy are screen-space UV derivatives. Zero cross terms reject rotation and skew.
    let x_step = dx * size;
    let y_step = dy * size;
    if (abs(x_step.y) > 0.0001 || abs(y_step.x) > 0.0001) {
        return false;
    }
    let sx = abs(x_step.x);
    let sy = abs(y_step.y);
    let integer_x = abs(sx - 0.5) < 0.0001
        || abs(sx - 0.33333334) < 0.0001
        || abs(sx - 0.25) < 0.0001;
    let integer_y = abs(sy - 0.5) < 0.0001
        || abs(sy - 0.33333334) < 0.0001
        || abs(sy - 0.25) < 0.0001;
    return size.x <= 96.0 && size.y <= 96.0 && integer_x && integer_y;
}

@vertex
fn main_vertex(in: common__VertexInputUv, @builtin(instance_index) instanceIndex: u32) -> VertexOutput {
    let pos = common__globals.global_matrix * (transforms[instanceIndex].world_matrix * vec4<f32>(in.position.x, in.position.y, 0.0, 1.0));
    return VertexOutput(pos, in.uv.xy / in.uv.z, transforms[instanceIndex].mult_color, transforms[instanceIndex].add_color, common__globals.bitmap_upscale.x);
}

@fragment
fn main_fragment(in: VertexOutput) -> @location(0) vec4<f32> {
    let size = vec2<f32>(textureDimensions(texture));
    let dx = dpdx(in.uv);
    let dy = dpdy(in.uv);
    let step_x = length(dx * size);
    let step_y = length(dy * size);
    var color = textureSample(texture, texture_sampler, in.uv);
    if (in.upscale > 0.5 && step_x > 0.0 && step_y > 0.0 && step_x < 0.999 && step_y < 0.999) {
        if (integer_small_bitmap_scale(in.uv, dx, dy, size)) {
            // Exact source texels even when the movie requests a linear sampler.
            let center = (floor(in.uv * size) + 0.5) / size;
            color = textureSampleLevel(texture, texture_sampler, center, 0.0);
        } else if (in.upscale > 1.5) {
            color = easu_sample(in.uv);
        }
    }
    // Texture is premultiplied by alpha.
    // Unmultiply alpha, apply color transform, remultiply alpha.
    if (color.a > 0.0) {
        color = vec4<f32>(color.rgb / color.a, color.a);
        color = color * in.mult_color + in.add_color;
        if (!late_saturate) {
            color = saturate(color);
        }
        color = vec4<f32>(color.rgb * color.a, color.a);
        if (late_saturate) {
            color = saturate(color);
        }
    }
    return color;
}
