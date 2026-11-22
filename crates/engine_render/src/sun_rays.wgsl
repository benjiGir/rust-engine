// La demi-résolution conserve la distance de la géométrie dans le canal alpha.
struct Rays {
    inverse_view_projection: mat4x4<f32>,
    shadow_projections: array<mat4x4<f32>, 2>,
    eye: vec4<f32>,
    sun: vec4<f32>,
    shadow_blend: vec4<f32>,
};

@group(0) @binding(0) var<uniform> rays: Rays;
@group(0) @binding(1) var scene_depth: texture_depth_2d;
@group(1) @binding(0) var sun_depth: texture_depth_2d_array;
@group(1) @binding(1) var sun_compare: sampler_comparison;
@group(2) @binding(0) var half_light: texture_2d<f32>;

struct Screen {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
};

@vertex
fn vs_screen(@builtin(vertex_index) index: u32) -> Screen {
    let xy = array(vec2(-1.0, -1.0), vec2(3.0, -1.0), vec2(-1.0, 3.0))[index];
    return Screen(vec4(xy, 0.0, 1.0), vec2(0.5 + 0.5 * xy.x, 0.5 - 0.5 * xy.y));
}

fn view_ray(uv: vec2<f32>) -> vec3<f32> {
    let size = vec2<i32>(textureDimensions(scene_depth));
    let pixel = clamp(vec2<i32>(uv * vec2<f32>(size)), vec2(0), size - 1);
    let depth = textureLoad(scene_depth, pixel, 0);
    let clip = vec4(uv.x * 2.0 - 1.0, 1.0 - uv.y * 2.0, depth, 1.0);
    let world = rays.inverse_view_projection * clip;
    return world.xyz / max(world.w, 0.00001) - rays.eye.xyz;
}

fn shadow_visibility(point: vec3<f32>, layer: i32) -> f32 {
    let p = (rays.shadow_projections[layer] * vec4(point, 1.0)).xyz;
    let edge = max(abs(p.x), abs(p.y));
    if edge >= 0.995 || p.z <= 0.0 || p.z >= 1.0 {
        return 0.0;
    }
    let uv = vec2(0.5 + 0.5 * p.x, 0.5 - 0.5 * p.y);
    return textureSampleCompareLevel(sun_depth, sun_compare, uv, layer, p.z - 0.00013)
        * (1.0 - smoothstep(0.83, 0.995, edge));
}

fn solar_visibility(point: vec3<f32>) -> f32 {
    return mix(shadow_visibility(point, 0), shadow_visibility(point, 1), rays.shadow_blend.x);
}

const MAX_RANGE: f32 = 86.0;
const STEPS: u32 = 24u;

@fragment
fn fs_volume(in: Screen) -> @location(0) vec4<f32> {
    let ray = view_ray(in.uv);
    let distance = min(length(ray), MAX_RANGE);
    let direction = ray / max(length(ray), 0.001);
    let night = rays.sun.w;
    let toward = max(dot(direction, rays.sun.xyz), 0.0);
    let phase = 0.10 + 0.70 * pow(toward, 3.0) + 1.80 * pow(toward, 12.0);
    var light = 0.0;
    for (var step = 0u; step < STEPS; step++) {
        let a = f32(step) / f32(STEPS);
        let b = f32(step + 1u) / f32(STEPS);
        // Les strates gardent leur position lorsque le fond du rayon change.
        let start = a * a * MAX_RANGE;
        if start >= distance { break; }
        let span = min(b * b * MAX_RANGE, distance) - start;
        let along = start + span * 0.5;
        let point = rays.eye.xyz + direction * along;
        let mist = 0.0022 + 0.0040 * exp(-max(point.y - 66.0, 0.0) * 0.050);
        let transmit = exp(-along * 0.010);
        light += span * mist * transmit * solar_visibility(point);
    }
    let warm = exp(-7.0 * rays.sun.y * rays.sun.y);
    let solar = mix(vec3(1.0, 0.96, 0.79), vec3(1.0, 0.70, 0.37), warm);
    let lunar = vec3(0.37, 0.56, 0.93);
    let color = mix(solar, lunar, night) * phase * light * rays.eye.w;
    return vec4(color, distance);
}

@fragment
fn fs_compose(in: Screen) -> @location(0) vec4<f32> {
    let distance = min(length(view_ray(in.uv)), MAX_RANGE);
    let size = vec2<i32>(textureDimensions(half_light));
    let grid = in.uv * vec2<f32>(size) - 0.5;
    let top_left = vec2<i32>(floor(grid));
    let frac = fract(grid);
    let tolerance = 0.35 + distance * 0.014;
    var color = vec3(0.0);
    var norm = 0.0;
    for (var y = 0; y < 2; y++) {
        for (var x = 0; x < 2; x++) {
            let q = clamp(top_left + vec2(x, y), vec2(0), size - 1);
            let sample = textureLoad(half_light, q, 0);
            let bilinear = select(1.0 - frac.x, frac.x, x != 0) * select(1.0 - frac.y, frac.y, y != 0);
            let delta = sample.a - distance;
            // Un rayon calculé derrière la silhouette contient de la lumière
            // que le rayon du premier plan n'a jamais traversée.
            let behind = 1.0 - smoothstep(tolerance * 0.65, tolerance * 2.0, delta);
            let weight = bilinear * exp(-abs(delta) / tolerance) * behind;
            color += sample.rgb * weight;
            norm += weight;
        }
    }
    return vec4(color / max(norm, 0.18), 0.0);
}
