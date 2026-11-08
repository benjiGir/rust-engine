// Shader des chunks : chaque sommet porte déjà sa position monde, sa normale,
// sa couleur et sa lumière. Pas d'instance : un chunk = un tampon de sommets.

struct Camera {
    view_proj: mat4x4<f32>,
    // xyz : position de la caméra, w : début du brouillard.
    position_fog_start: vec4<f32>,
    // rgb : couleur du brouillard (le ciel), w : fin du brouillard.
    fog_color_end: vec4<f32>,
    // xyz : direction vers le soleil, w : intensité du jour (0 la nuit, 1 à midi).
    sun: vec4<f32>,
    // rgb : teinte de la lumière du ciel.
    sun_color: vec4<f32>,
};

@group(0) @binding(0)
var<uniform> camera: Camera;

// Les textures des blocs : un tableau de couches 16 × 16 (la couche 0 est blanche).
@group(1) @binding(0)
var block_textures: texture_2d_array<f32>;
@group(1) @binding(1)
var block_sampler: sampler;

struct VertexIn {
    @location(0) position: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) color: vec4<f32>,
    // x : lumière du ciel, y : lumière des blocs, z : occlusion ambiante (1 = dégagé).
    @location(3) light: vec3<f32>,
    @location(4) texture: u32,
};

struct VertexOut {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) color: vec4<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) world_position: vec3<f32>,
    @location(3) light: vec3<f32>,
    // `flat` : un entier ne s'interpole pas, on garde la valeur du premier sommet.
    @location(4) @interpolate(flat) texture: u32,
};

@vertex
fn vs_main(vertex: VertexIn) -> VertexOut {
    var out: VertexOut;
    out.clip_position = camera.view_proj * vec4<f32>(vertex.position, 1.0);
    out.color = vertex.color;
    out.normal = vertex.normal;
    out.world_position = vertex.position;
    // Interpolée entre les sommets : c'est ce qui rend la lumière « douce ».
    out.light = vertex.light;
    out.texture = vertex.texture;
    return out;
}

// Un niveau de lumière (0 à 1) vers une luminosité. Chaque niveau en moins divise
// par 1,25 environ, comme dans Minecraft : 15 donne 1, 7 donne 0,17, 0 donne 0,04.
fn brightness(level: f32) -> f32 {
    return pow(0.8, (1.0 - level) * 15.0);
}

// Coordonnées de texture tirées de la position dans le monde, selon l'orientation de la
// face : un bloc = une texture entière, et un grand quad fusionné la répète toute seule.
// Sur les côtés, v suit -y : le haut de l'image est en haut du bloc.
fn face_uv(position: vec3<f32>, normal: vec3<f32>) -> vec2<f32> {
    let n = abs(normal);
    if n.y > 0.5 {
        return position.xz;
    }
    if n.x > 0.5 {
        return vec2<f32>(position.z, -position.y);
    }
    return vec2<f32>(position.x, -position.y);
}

@fragment
fn fs_main(in: VertexOut) -> @location(0) vec4<f32> {
    let texel = textureSample(block_textures, block_sampler, face_uv(in.world_position, in.normal), in.texture);
    let albedo = in.color * texel;
    let daylight = camera.sun.w;
    let sun_direction = normalize(camera.sun.xyz);

    // Le ciel : sa lumière stockée, multipliée par l'intensité du jour. Le soleil
    // éclaire plus les faces tournées vers lui (éclairage diffus) ; sous l'horizon,
    // il n'éclaire plus directement.
    let facing = max(dot(normalize(in.normal), sun_direction), 0.0) * step(0.0, sun_direction.y);
    let sky = brightness(in.light.x) * daylight * (0.6 + 0.4 * facing) * camera.sun_color.rgb;
    // Les torches : une lumière chaude, la même jour et nuit.
    let torch = brightness(in.light.y) * vec3<f32>(1.0, 0.82, 0.6);
    // Un peu de lumière partout, pour ne jamais voir du noir complet.
    let ambient = 0.03;
    // On garde la plus forte des deux, comme Minecraft (on ne les additionne pas).
    var light = max(sky, torch) + ambient;
    // Occlusion ambiante : un coin enfermé descend à 45 % de la lumière.
    light *= 0.45 + 0.55 * in.light.z;
    let lit = albedo.rgb * light;

    // Brouillard : distance horizontale, le brouillard forme un cylindre comme le
    // rayon de chargement. `smoothstep` : 0 avant le début, 1 après la fin.
    let offset = in.world_position.xz - camera.position_fog_start.xz;
    let fog = smoothstep(camera.position_fog_start.w, camera.fog_color_end.w, length(offset));
    return vec4<f32>(mix(lit, camera.fog_color_end.rgb, fog), albedo.a);
}
