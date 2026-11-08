// Shader des lignes : le contour du bloc visé (dans le monde) et le réticule
// (directement à l'écran). Pas d'éclairage : une couleur unie par sommet.

struct Camera {
    view_proj: mat4x4<f32>,
    position_fog_start: vec4<f32>,
    fog_color_end: vec4<f32>,
};

@group(0) @binding(0)
var<uniform> camera: Camera;

struct VertexIn {
    @location(0) position: vec3<f32>,
    @location(1) color: vec3<f32>,
};

struct VertexOut {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) color: vec3<f32>,
};

// Positions en coordonnées monde, transformées par la caméra.
@vertex
fn vs_world(vertex: VertexIn) -> VertexOut {
    var out: VertexOut;
    out.clip_position = camera.view_proj * vec4<f32>(vertex.position, 1.0);
    out.color = vertex.color;
    return out;
}

// Positions déjà à l'écran : x et y entre -1 et 1, (0, 0) au centre.
@vertex
fn vs_screen(vertex: VertexIn) -> VertexOut {
    var out: VertexOut;
    out.clip_position = vec4<f32>(vertex.position.xy, 0.0, 1.0);
    out.color = vertex.color;
    return out;
}

@fragment
fn fs_main(in: VertexOut) -> @location(0) vec4<f32> {
    return vec4<f32>(in.color, 1.0);
}
