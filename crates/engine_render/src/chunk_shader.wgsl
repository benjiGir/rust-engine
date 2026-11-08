// Shader des chunks : chaque sommet porte déjà sa position monde, sa normale
// et sa couleur. Pas d'instance : un chunk = un tampon de sommets.

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
    @location(2) color: vec3<f32>,
};

struct VertexOut {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) color: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) world_position: vec3<f32>,
};

@vertex
fn vs_main(vertex: VertexIn) -> VertexOut {
    var out: VertexOut;
    out.clip_position = camera.view_proj * vec4<f32>(vertex.position, 1.0);
    out.color = vertex.color;
    out.normal = vertex.normal;
    out.world_position = vertex.position;
    return out;
}

// Même éclairage que les cubes instanciés, plus un brouillard qui fond
// les chunks lointains dans la couleur du ciel : la limite de vue disparaît.
@fragment
fn fs_main(in: VertexOut) -> @location(0) vec4<f32> {
    let sun = normalize(vec3<f32>(0.4, 1.0, 0.3));
    let diffuse = max(dot(normalize(in.normal), sun), 0.0);
    let light = 0.35 + 0.65 * diffuse;
    let lit = in.color * light;

    // Distance horizontale : le brouillard forme un cylindre, comme le rayon de chargement.
    let offset = in.world_position.xz - camera.position_fog_start.xz;
    // `smoothstep` : 0 avant le début, 1 après la fin, une courbe douce entre les deux.
    let fog = smoothstep(camera.position_fog_start.w, camera.fog_color_end.w, length(offset));
    return vec4<f32>(mix(lit, camera.fog_color_end.rgb, fog), 1.0);
}
