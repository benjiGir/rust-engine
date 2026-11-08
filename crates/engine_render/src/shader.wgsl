// Shader des cubes : position par instance, couleur par instance,
// éclairage diffus simple par un soleil fixe.

struct Camera {
    view_proj: mat4x4<f32>,
    // xyz : position de la caméra, w : début du brouillard.
    position_fog_start: vec4<f32>,
    // rgb : couleur du brouillard (le ciel), w : fin du brouillard.
    fog_color_end: vec4<f32>,
};

@group(0) @binding(0)
var<uniform> camera: Camera;

struct VertexIn {
    @location(0) position: vec3<f32>,
    @location(1) normal: vec3<f32>,
};

// Données propres à chaque cube (une entrée du tampon d'instances).
struct InstanceIn {
    @location(2) offset: vec3<f32>,
    @location(3) color: vec3<f32>,
};

struct VertexOut {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) color: vec3<f32>,
    @location(1) normal: vec3<f32>,
};

@vertex
fn vs_main(vertex: VertexIn, instance: InstanceIn) -> VertexOut {
    var out: VertexOut;
    let world = vertex.position + instance.offset;
    out.clip_position = camera.view_proj * vec4<f32>(world, 1.0);
    out.color = instance.color;
    out.normal = vertex.normal;
    return out;
}

@fragment
fn fs_main(in: VertexOut) -> @location(0) vec4<f32> {
    let sun = normalize(vec3<f32>(0.4, 1.0, 0.3));
    let diffuse = max(dot(normalize(in.normal), sun), 0.0);
    let light = 0.35 + 0.65 * diffuse;
    return vec4<f32>(in.color * light, 1.0);
}
