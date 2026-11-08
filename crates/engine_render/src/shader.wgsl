// Shader des boîtes instanciées (les entités) : un cube unité, déplacé, étiré et tourné
// par instance, éclairé comme les chunks (ciel × jour, torches, brouillard).

struct Camera {
    view_proj: mat4x4<f32>,
    position_fog_start: vec4<f32>,
    fog_color_end: vec4<f32>,
    sun: vec4<f32>,
    sun_color: vec4<f32>,
};

@group(0) @binding(0)
var<uniform> camera: Camera;

struct VertexIn {
    // Un cube de (0, 0, 0) à (1, 1, 1).
    @location(0) position: vec3<f32>,
    @location(1) normal: vec3<f32>,
};

// Données propres à chaque boîte (une entrée du tampon d'instances).
struct InstanceIn {
    @location(2) center: vec3<f32>,
    @location(3) size: vec3<f32>,
    @location(4) color: vec3<f32>,
    @location(5) yaw: f32,
    @location(6) light: vec2<f32>,
};

struct VertexOut {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) color: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) world_position: vec3<f32>,
    @location(3) light: vec2<f32>,
};

// Rotation autour de y : l'avant du modèle (+x) regarde dans la direction `yaw`.
fn rotate_y(v: vec3<f32>, yaw: f32) -> vec3<f32> {
    let c = cos(yaw);
    let s = sin(yaw);
    return vec3<f32>(v.x * c - v.z * s, v.y, v.x * s + v.z * c);
}

@vertex
fn vs_main(vertex: VertexIn, instance: InstanceIn) -> VertexOut {
    var out: VertexOut;
    let local = (vertex.position - vec3<f32>(0.5)) * instance.size;
    let world = instance.center + rotate_y(local, instance.yaw);
    out.clip_position = camera.view_proj * vec4<f32>(world, 1.0);
    out.color = instance.color;
    out.normal = rotate_y(vertex.normal, instance.yaw);
    out.world_position = world;
    out.light = instance.light;
    return out;
}

fn brightness(level: f32) -> f32 {
    return pow(0.8, (1.0 - level) * 15.0);
}

@fragment
fn fs_main(in: VertexOut) -> @location(0) vec4<f32> {
    let sun_direction = normalize(camera.sun.xyz);
    let facing = max(dot(normalize(in.normal), sun_direction), 0.0) * step(0.0, sun_direction.y);
    let sky = brightness(in.light.x) * camera.sun.w * (0.6 + 0.4 * facing) * camera.sun_color.rgb;
    let torch = brightness(in.light.y) * vec3<f32>(1.0, 0.82, 0.6);
    // Les faces du bas un peu plus sombres : les boîtes ont du relief même dans l'ombre.
    let shade = 0.8 + 0.2 * max(in.normal.y, 0.0);
    let lit = in.color * (max(sky, torch) + 0.03) * shade;
    let offset = in.world_position.xz - camera.position_fog_start.xz;
    let fog = smoothstep(camera.position_fog_start.w, camera.fog_color_end.w, length(offset));
    return vec4<f32>(mix(lit, camera.fog_color_end.rgb, fog), 1.0);
}
