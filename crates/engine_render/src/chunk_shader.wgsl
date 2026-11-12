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
    // Position dans le chunk : (p + 8) × 1024 ; w : index du bloc animé (x + 32z + 1024y).
    @location(0) position: vec4<u32>,
    // Déjà ramenée de -127..127 à -1..1 par le GPU (`Snorm8x4`) ; w : vitesse × 8 / 127.
    @location(1) normal: vec4<f32>,
    @location(2) color: vec4<f32>,
    // x : ciel, y : blocs, z : AO ; w : axe de rotation / 255 (X = 0, Y = 1, Z = 2).
    @location(3) light: vec4<f32>,
    // Les 16 bits du bas : la couche de texture ; au-dessus, des drapeaux.
    @location(4) texture: u32,
    // L'origine du chunk dans le monde (xyz) et, pour une structure mobile, son lacet (w) :
    // les sommets tournent autour de l'axe vertical avant d'être placés.
    @location(5) origin: vec4<f32>,
    // La base des coordonnées de texture (xyz) : l'origine du chunk dans son propre repère.
    // Pour le monde, c'est l'origine ; pour une structure qui bouge, ses coordonnées locales,
    // afin que les textures restent collées aux blocs. `w` : l'échelle des positions locales
    // (0 ou 1 : un bloc par unité ; un terrain lointain en met 4, 8, 16... par unité).
    @location(6) uv_base: vec4<f32>,
};

// Le codage des positions (voir `engine_voxel::POSITION_SCALE` et `POSITION_BIAS`).
const POSITION_SCALE: f32 = 1024.0;
const POSITION_BIAS: f32 = 8.0;

// Les drapeaux de `texture` (voir `engine_voxel::mesh`).
const FLAG_CUTOUT: u32 = 65536u;   // 1 << 16 : pixels transparents découpés
const FLAG_SWAY: u32 = 131072u;    // 1 << 17 : se balance au vent
const FLAG_FILLED: u32 = 262144u;  // 1 << 18 : pixels transparents remplis (feuille opaque)
const FLAG_ROTATE: u32 = 524288u;
const FLAG_FURNACE_FIRE: u32 = 1048576u;
const FLAG_GROUND: u32 = 2097152u;
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
    // Position et normale dans le repère des textures (celui de la structure, s'il y en a une).
    @location(6) tex_position: vec3<f32>,
    @location(7) tex_normal: vec3<f32>,
};

// Fait tourner un vecteur autour de l'axe vertical (même convention que `rotate_y` côté jeu).
fn rotate_y(v: vec3<f32>, yaw: f32) -> vec3<f32> {
    let s = sin(yaw);
    let c = cos(yaw);
    return vec3<f32>(v.x * c - v.z * s, v.y, v.x * s + v.z * c);
}

fn rotate_axis(v: vec3<f32>, axis: vec3<f32>, angle: f32) -> vec3<f32> {
    let c = cos(angle);
    return v * c + cross(axis, v) * sin(angle) + axis * dot(axis, v) * (1.0 - c);
}

@vertex
fn vs_main(vertex: VertexIn) -> VertexOut {
    var out: VertexOut;
    var local = vec3<f32>(vertex.position.xyz) / POSITION_SCALE - vec3<f32>(POSITION_BIAS);
    if vertex.uv_base.w > 1.0 {
        local = local * vertex.uv_base.w;
    }
    // Les UV restent dans le repère de repos : la texture tourne avec la pièce.
    let tex_position = vertex.uv_base.xyz + local;
    var normal = vertex.normal.xyz;
    if (vertex.texture & FLAG_ROTATE) != 0u {
        let index = vertex.position.w;
        let pivot = vec3<f32>(f32(index & 31u), f32(index >> 10u), f32((index >> 5u) & 31u)) + vec3<f32>(0.5);
        let axis_index = u32(round(vertex.light.w * 255.0));
        var axis = vec3<f32>(0.0);
        axis[axis_index] = 1.0;
        let angle = camera.sun_color.w * vertex.normal.w * (127.0 / 8.0);
        local = pivot + rotate_axis(local - pivot, axis, angle);
        normal = rotate_axis(normal, axis, angle);
    }
    var world = vertex.origin.xyz + local;
    if vertex.origin.w != 0.0 {
        world = vertex.origin.xyz + rotate_y(local, vertex.origin.w);
        normal = rotate_y(normal, vertex.origin.w);
    }
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
    out.normal = normal;
    out.world_position = world;
    out.tex_position = tex_position;
    out.tex_normal = vertex.normal.xyz;
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
    var texel = textureSample(block_textures, block_sampler, face_uv(in.tex_position, in.tex_normal), in.texture);
    // Une feuille au cœur du feuillage : ses trous deviennent un vert sombre (la couleur
    // de la feuille, assombrie), comme le fond d'un buisson dense.
    if (in.flags & FLAG_FILLED) != 0u && texel.a < 0.5 {
        texel = vec4<f32>(vec3<f32>(0.32), 1.0);
    }
    // Feuilles et plantes : les pixels transparents de la texture sont jetés. Le reste
    // est opaque, dessiné dans la passe opaque (pas de tri, pas de mélange).
    if (in.flags & FLAG_CUTOUT) != 0u && texel.a < 0.5 {
        discard;
    }
    var albedo = in.color * texel;
    if (in.flags & FLAG_GROUND) != 0u {
        // Variation douce sur plusieurs blocs, sans changement d'UV aux limites des
        // blocs ou des quads greedy. Le motif 16×16 n'impose plus une grille au terrain.
        let p = in.tex_position.xz * 0.13;
        let cell = floor(p);
        let f = fract(p);
        let u = f * f * (vec2<f32>(3.0) - 2.0 * f);
        let a = fract(sin(dot(cell, vec2<f32>(127.1, 311.7))) * 43758.5453);
        let b = fract(sin(dot(cell + vec2<f32>(1.0, 0.0), vec2<f32>(127.1, 311.7))) * 43758.5453);
        let c = fract(sin(dot(cell + vec2<f32>(0.0, 1.0), vec2<f32>(127.1, 311.7))) * 43758.5453);
        let d = fract(sin(dot(cell + vec2<f32>(1.0), vec2<f32>(127.1, 311.7))) * 43758.5453);
        albedo = vec4<f32>(albedo.rgb * (0.96 + 0.08 * mix(mix(a, b, u.x), mix(c, d, u.x), u.y)), albedo.a);
    }
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
    var lit = albedo.rgb * light;
    if (in.flags & FLAG_FURNACE_FIRE) != 0u {
        // Même bouche 8 × 6 que furnace_front.png. Animation pixel art à 10 images/s,
        // calculée sur place : aucune texture transférée ni chunk remaillé par image.
        let pixel = floor(fract(face_uv(in.tex_position, in.tex_normal)) * 16.0);
        let frame = floor(camera.sun_color.w * 10.0);
        let height = 2.0 + floor(3.0 * fract(sin(pixel.x * 12.9898 + frame * 4.1414) * 43758.5453));
        if pixel.x >= 4.0 && pixel.x < 12.0 && pixel.y >= 14.0 - height && pixel.y < 14.0 {
            // Les flammes éclairent leur propre surface, même dans une grotte.
            lit = select(vec3<f32>(1.0, 0.22, 0.015), vec3<f32>(1.0, 0.72, 0.12), pixel.y >= 12.0);
        }
    }

    // Brouillard : distance horizontale, le brouillard forme un cylindre comme le
    // rayon de chargement. `smoothstep` : 0 avant le début, 1 après la fin.
    let offset = in.world_position.xz - camera.position_fog_start.xz;
    let fog = smoothstep(camera.position_fog_start.w, camera.fog_color_end.w, length(offset));
    return vec4<f32>(mix(lit, camera.fog_color_end.rgb, fog), albedo.a);
}
