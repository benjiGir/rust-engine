// Shader des chunks : chaque sommet porte sa position dans le chunk, sa normale, sa
// couleur et sa lumière, en format compact (24 octets, voir `ColorVertex`). L'origine
// du chunk arrive à part, une fois pour tout le maillage (une « instance »).

struct Camera {
    view_proj: mat4x4<f32>,
    // xyz : position de la caméra, w : début du brouillard.
    position_fog_start: vec4<f32>,
    // rgb : couleur du brouillard (le ciel), w : fin du brouillard.
    fog_color_end: vec4<f32>,
    // xyz : direction vers le soleil, w : intensité du jour (0 la nuit, 1 à midi).
    sun: vec4<f32>,
    // rgb : teinte de la lumière du ciel, w : le temps en secondes (pour le vent).
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
    // Position dans le chunk : (p + 8) × 1024, en entiers (le 4e ne sert pas).
    @location(0) position: vec4<u32>,
    // Déjà ramenée de -127..127 à -1..1 par le GPU (`Snorm8x4`).
    @location(1) normal: vec4<f32>,
    @location(2) color: vec4<f32>,
    // x : lumière du ciel, y : lumière des blocs, z : occlusion ambiante (1 = dégagé).
    @location(3) light: vec4<f32>,
    // Les 16 bits du bas : la couche de texture ; au-dessus, des drapeaux.
    @location(4) texture: u32,
    // L'origine du chunk dans le monde (xyz).
    @location(5) origin: vec4<f32>,
};

// Le codage des positions (voir `engine_voxel::POSITION_SCALE` et `POSITION_BIAS`).
const POSITION_SCALE: f32 = 1024.0;
const POSITION_BIAS: f32 = 8.0;

// Les drapeaux de `texture` (voir `engine_voxel::mesh`).
const FLAG_CUTOUT: u32 = 65536u;   // 1 << 16 : pixels transparents découpés
const FLAG_SWAY: u32 = 131072u;    // 1 << 17 : se balance au vent
const TEXTURE_MASK: u32 = 65535u;

struct VertexOut {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) color: vec4<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) world_position: vec3<f32>,
    @location(3) light: vec3<f32>,
    // `flat` : un entier ne s'interpole pas, on garde la valeur du premier sommet.
    @location(4) @interpolate(flat) texture: u32,
    @location(5) @interpolate(flat) flags: u32,
};

@vertex
fn vs_main(vertex: VertexIn) -> VertexOut {
    var out: VertexOut;
    let world = vertex.origin.xyz + vec3<f32>(vertex.position.xyz) / POSITION_SCALE - vec3<f32>(POSITION_BIAS);
    var position = world;
    // Le vent : un petit déplacement horizontal qui ne dépend que de la position dans le
    // monde et du temps. Deux sommets au même endroit bougent pareil : les feuilles
    // voisines restent jointes. Un sommet trop éclairé par le ciel (dehors) bouge plus.
    if (vertex.texture & FLAG_SWAY) != 0u {
        let t = camera.sun_color.w;
        let p = world;
        let amount = 0.05 * (0.4 + 0.6 * vertex.light.x);
        position.x += sin(t * 1.7 + p.x * 0.6 + p.z * 0.35) * amount;
        position.z += cos(t * 1.3 + p.z * 0.55 + p.x * 0.25) * amount;
    }
    out.clip_position = camera.view_proj * vec4<f32>(position, 1.0);
    out.color = vertex.color;
    out.normal = vertex.normal.xyz;
    out.world_position = world;
    // Interpolée entre les sommets : c'est ce qui rend la lumière « douce ».
    out.light = vertex.light.xyz;
    out.texture = vertex.texture & TEXTURE_MASK;
    out.flags = vertex.texture & ~TEXTURE_MASK;
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
    // Feuilles et plantes : les pixels transparents de la texture sont jetés. Le reste
    // est opaque, dessiné dans la passe opaque (pas de tri, pas de mélange).
    if (in.flags & FLAG_CUTOUT) != 0u && texel.a < 0.5 {
        discard;
    }
    var albedo = in.color * texel;
    if (in.flags & FLAG_CUTOUT) != 0u {
        albedo.a = in.color.a;
    }
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
