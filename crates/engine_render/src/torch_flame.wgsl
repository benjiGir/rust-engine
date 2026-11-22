const SURFACE_TORCH_FLAME: u32 = 2u;
const SURFACE_TORCH_WOOD: u32 = 3u;

// La flamme s'évase au niveau de la braise, puis ses pointes se déforment en montant.
fn torch_flame(uv: vec2<f32>, time: f32, phase: f32) -> vec4<f32> {
    if any(uv < vec2(0.0)) || any(uv > vec2(1.0)) { return vec4(0.0); }

    let grid = vec2(18.0, 24.0);
    let p = (floor(uv * grid) + 0.5) / grid;
    let h = 1.0 - p.y;
    let t = time + phase;
    let height = 0.79 + 0.09 * sin(t * 7.1) + 0.04 * sin(t * 12.7);
    let y = h / height;
    let lean = y * y * (0.20 * sin(t * 5.3) + 0.08 * sin(y * 9.0 - t * 8.0));
    let x = p.x - 0.5 - lean;
    let width = (0.17 + 0.37 * y) * max(1.0 - y, 0.0);
    let rim = abs(x) / max(width, 0.001);
    let side_height = 0.49 + 0.09 * sin(t * 9.2 + 1.7);
    let side_x = x + 0.19 + y * 0.09 * sin(t * 6.2);
    let side = length(vec2(side_x / 0.10, (y - 0.31) / side_height));
    if y >= 0.0 && y < 1.0 && (rim < 1.0 || side < 1.0) {
        let edge = mix(vec3(1.0, 0.21, 0.012), vec3(0.91, 0.055, 0.006), y);
        let middle = mix(vec3(1.0, 0.66, 0.055), vec3(1.0, 0.26, 0.012), y);
        let hot = vec3(1.0, 0.93, 0.44);
        let glow = 1.0 - smoothstep(0.44, 0.95, rim);
        let core = (1.0 - smoothstep(0.15, 0.52, rim)) * (1.0 - smoothstep(0.26, 0.72, y));
        return vec4(mix(mix(edge, middle, glow), hot, core), 1.0);
    }

    // Petites braises indépendantes qui ralentissent en s'éloignant du foyer.
    for (var i = 0; i < 2; i++) {
        let cycle = t * 1.25 + f32(i) * 0.5;
        let age = fract(cycle);
        let drift = sin(floor(cycle) * 4.1 + f32(i) * 2.7 + phase);
        let spark = vec2(0.5 + drift * (0.12 + age * 0.25), 0.43 + 0.51 * age * (2.0 - age));
        if all(floor(vec2(p.x, h) * grid) == floor(spark * grid)) {
            return vec4(mix(vec3(1.0, 0.41, 0.045), vec3(0.64, 0.078, 0.009), age), 1.0);
        }
    }
    return vec4(0.0);
}

// Les texels orange vif du bois représentent les fissures incandescentes.
fn torch_embers(albedo: vec3<f32>, time: f32, phase: f32) -> vec3<f32> {
    let mask = smoothstep(0.33, 0.66, albedo.r) * smoothstep(0.20, 0.55, albedo.r - albedo.g);
    let heat = 0.64 + 0.12 * sin(time * 7.1 + phase);
    return vec3(0.76, 0.13, 0.012) * mask * heat;
}
