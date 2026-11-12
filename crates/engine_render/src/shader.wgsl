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
@group(1) @binding(0) var block_textures: texture_2d_array<f32>;
@group(1) @binding(1) var block_sampler: sampler;

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
    @location(7) opacity: f32,
    @location(8) textures: vec3<u32>,
    @location(9) cutout: u32,
    @location(10) uv_min: vec3<f32>,
    @location(11) uv_max: vec3<f32>,
    // x : angle de balancement (autour de l'axe latéral z), y : hauteur du pivot au-dessus du centre.
    @location(12) swing: vec2<f32>,
};

struct VertexOut {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) color: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) world_position: vec3<f32>,
    @location(3) light: vec2<f32>,
    @location(4) uv: vec2<f32>,
    @location(5) @interpolate(flat) texture: u32,
    @location(6) @interpolate(flat) cutout: u32,
    @location(7) opacity: f32,
};

// Rotation autour de y : l'avant du modèle (+x) regarde dans la direction `yaw`.
fn rotate_y(v: vec3<f32>, yaw: f32) -> vec3<f32> {
    let c = cos(yaw);
    let s = sin(yaw);
    return vec3<f32>(v.x * c - v.z * s, v.y, v.x * s + v.z * c);
}

// Rotation autour de l'axe latéral (z) : un angle positif envoie le bas vers l'avant (+x).
fn rotate_z(v: vec3<f32>, angle: f32) -> vec3<f32> {
    let c = cos(angle);
    let s = sin(angle);
    return vec3<f32>(v.x * c - v.y * s, v.x * s + v.y * c, v.z);
}

@vertex
fn vs_main(vertex: VertexIn, instance: InstanceIn) -> VertexOut {
    var out: VertexOut;
    var local = (vertex.position - vec3<f32>(0.5)) * instance.size;
    var normal = vertex.normal;
    if instance.swing.x != 0.0 {
        // Une patte pivote par la hanche : on tourne autour du point d'attache.
        let pivot = vec3<f32>(0.0, instance.swing.y, 0.0);
        local = pivot + rotate_z(local - pivot, instance.swing.x);
        normal = rotate_z(normal, instance.swing.x);
    }
    let world = instance.center + rotate_y(local, instance.yaw);
    out.clip_position = camera.view_proj * vec4<f32>(world, 1.0);
    out.color = instance.color;
    out.normal = rotate_y(normal, instance.yaw);
    out.world_position = world;
    out.light = instance.light;
    let p = mix(instance.uv_min, instance.uv_max, vertex.position);
    out.texture = instance.textures.y;
    if abs(vertex.normal.y) > 0.5 {
        out.texture = select(instance.textures.z, instance.textures.x, vertex.normal.y > 0.0);
        out.uv = p.xz;
    } else if abs(vertex.normal.x) > 0.5 {
        out.uv = vec2<f32>(p.z, 1.0 - p.y);
    } else {
        out.uv = vec2<f32>(p.x, 1.0 - p.y);
    }
    out.cutout = instance.cutout;
    out.opacity = instance.opacity;
    return out;
}

fn brightness(level: f32) -> f32 {
    return pow(0.8, (1.0 - level) * 15.0);
}

@fragment
fn fs_main(in: VertexOut) -> @location(0) vec4<f32> {
    // Échantillonnage inconditionnel : dérivées bien définies sur les limites de face.
    let texel = textureSample(block_textures, block_sampler, in.uv, in.texture);
    if in.cutout != 0u && texel.a < 0.5 { discard; }
    let sun_direction = normalize(camera.sun.xyz);
    let facing = max(dot(normalize(in.normal), sun_direction), 0.0) * step(0.0, sun_direction.y);
    let sky = brightness(in.light.x) * camera.sun.w * (0.6 + 0.4 * facing) * camera.sun_color.rgb;
    let torch = brightness(in.light.y) * vec3<f32>(1.0, 0.82, 0.6);
    // Les faces du bas un peu plus sombres : les boîtes ont du relief même dans l'ombre.
    let shade = 0.8 + 0.2 * max(in.normal.y, 0.0);
    let lit = in.color * texel.rgb * (max(sky, torch) + 0.03) * shade;
    let offset = in.world_position.xz - camera.position_fog_start.xz;
    let fog = smoothstep(camera.position_fog_start.w, camera.fog_color_end.w, length(offset));
    return vec4<f32>(mix(lit, camera.fog_color_end.rgb, fog), texel.a * in.opacity);
}
