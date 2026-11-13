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
    // x : 0 dehors, 1 sous l'eau ; y : altitude de la surface au-dessus de la caméra.
    water: vec4<f32>,
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
const FLAG_WATER: u32 = 4194304u;   // 1 << 22 : eau libre, animée
const TURN_SHIFT: u32 = 23u;        // 2 bits : quarts de tour de la texture (les rails)
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

// La hauteur d'une houle (somme de trois ondes qui se croisent) : sert à incliner la surface
// de l'eau, pas à la déplacer (une grande face fusionnée n'a que quatre sommets).
fn wave_height(p: vec2<f32>, t: f32) -> f32 {
    return sin(p.x * 1.3 + t * 1.1) * 0.5
        + sin(p.y * 1.7 - t * 0.9 + p.x * 0.4) * 0.35
        + sin((p.x + p.y) * 2.9 + t * 1.9) * 0.15;
}

fn wave_normal(p: vec2<f32>, t: f32) -> vec3<f32> {
    let e = 0.05;
    let h = wave_height(p, t);
    let dx = (wave_height(p + vec2<f32>(e, 0.0), t) - h) / e;
    let dz = (wave_height(p + vec2<f32>(0.0, e), t) - h) / e;
    return normalize(vec3<f32>(-dx * 0.08, 1.0, -dz * 0.08));
}

// Les reflets de lumière qui dansent au fond de l'eau : des lignes fines là où des ondulations
// se renforcent.
fn caustics(p: vec2<f32>, t: f32) -> f32 {
    let a = sin(p.x * 1.7 + t * 0.8 + sin(p.y * 1.3 + t * 0.6));
    let b = sin(p.y * 2.1 - t * 0.7 + sin(p.x * 1.9 - t * 0.5));
    let c = sin((p.x + p.y) * 1.1 + t * 0.4);
    return pow(1.0 - abs((a + b + c) / 3.0), 6.0);
}

// Les filets d'une chute d'eau : des rubans verticaux de largeurs et de vitesses différentes,
// dont des bandes plus claires descendent. `along` : la position en travers de la chute ;
// `down` : la hauteur (elle diminue le long du courant). Renvoie 0 (rien) à 1 (un filet clair).
fn fall_streaks(along: f32, down: f32, t: f32) -> f32 {
    var total = 0.0;
    for (var layer = 0; layer < 2; layer++) {
        let scale = select(5.0, 11.0, layer == 1);
        let column = along * scale + f32(layer) * 17.0;
        let id = floor(column);
        let f = fract(column);
        let h = fract(sin(id * 12.9898 + f32(layer) * 3.7) * 43758.5453);
        // Les bords du ruban s'adoucissent : pas de colonnes nettes comme une grille.
        let ribbon = smoothstep(0.0, 0.3, f) * smoothstep(1.0, 0.7, f);
        let band = fract(down * (0.22 + 0.1 * h) + t * (1.4 + 1.6 * h) + h * 9.0);
        let bright = smoothstep(0.0, 0.18, band) * smoothstep(0.75, 0.3, band);
        total += ribbon * (0.25 + 0.75 * bright) * select(1.0, 0.6, layer == 1);
    }
    return clamp(total * 0.7, 0.0, 1.0);
}

@fragment
fn fs_main(in: VertexOut) -> @location(0) vec4<f32> {
    let time = camera.sun_color.w;
    var uv = face_uv(in.tex_position, in.tex_normal);
    // Un rail : le même dessin tourné d'un, deux ou trois quarts de tour (sens horaire, vu d'en haut).
    let turns = (in.flags >> TURN_SHIFT) & 3u;
    if turns != 0u {
        let cell = floor(uv);
        var t = uv - cell;
        for (var i = 0u; i < turns; i++) {
            t = vec2<f32>(t.y, 1.0 - t.x);
        }
        uv = cell + t;
    }
    let is_water = (in.flags & FLAG_WATER) != 0u;
    var wp = in.world_position.xz;
    // Le courant : sur le dessus d'une eau libre, la normale est inclinée par la pente de la
    // surface (voir le maillage) ; l'eau coule vers le bas de la pente, d'autant plus vite
    // qu'elle est raide. Sur les côtés, elle tombe.
    var flow_dir = vec2<f32>(0.0);
    var current = 0.0;
    if is_water && abs(in.tex_normal.y) > 0.5 {
        let slope = length(in.tex_normal.xz);
        flow_dir = in.tex_normal.xz / max(slope, 1e-4);
        current = smoothstep(0.015, 0.16, slope);
    }
    if is_water {
        // La texture glisse doucement et ondule : l'eau ne reste pas figée.
        uv += vec2<f32>(time * 0.021, time * 0.013)
            + 0.02 * vec2<f32>(sin(wp.y * 3.1 + time * 1.3), cos(wp.x * 2.7 + time * 1.1));
        if abs(in.tex_normal.y) > 0.5 {
            // Les motifs partent avec le courant (1,2 bloc par seconde au plus).
            let drift = flow_dir * current * 1.2 * time;
            uv -= drift;
            wp -= drift * 1.5;
        } else {
            // Une chute : la texture descend, vite.
            uv.y -= time * 2.2;
        }
    }
    var texel = textureSample(block_textures, block_sampler, uv, in.texture);
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
    var alpha = albedo.a;
    if is_water {
        let to_camera = camera.position_fog_start.xyz - in.world_position;
        let view = normalize(to_camera);
        var n = vec3<f32>(0.0, 1.0, 0.0);
        let tilt = in.tex_normal.xyz - vec3<f32>(0.0, 1.0, 0.0);
        // Les vagues s'effacent avec la distance : trop petites, elles feraient du moiré.
        let calm = smoothstep(18.0, 70.0, length(to_camera));
        if abs(in.tex_normal.y) > 0.5 {
            // Une pente raide est une chute : on ne l'éclaire pas comme une vague.
            let gentle = 1.0 - smoothstep(0.4, 0.9, length(in.tex_normal.xz));
            n = normalize(mix(wave_normal(wp, time), n, calm) + tilt * 1.5 * gentle);
        }
        // Le dessus vu de dessous : la même surface, retournée.
        n = select(-n, n, view.y >= 0.0 || abs(in.tex_normal.y) < 0.5);
        // Fresnel : de biais, l'eau reflète le ciel ; de face, on voit à travers.
        let fresnel = pow(1.0 - clamp(abs(dot(n, view)), 0.0, 1.0), 4.0);
        let sky_color = camera.fog_color_end.rgb;
        lit = mix(lit, sky_color * (0.4 + 0.6 * daylight), fresnel * 0.55);
        // Le soleil, en reflet : une tache brillante qui scintille avec les vagues.
        let glint = pow(max(dot(reflect(-sun_direction, n), view), 0.0), 220.0)
            * daylight * step(0.0, sun_direction.y) * (1.0 - calm);
        lit += vec3<f32>(1.0, 0.93, 0.75) * glint * 0.5;
        alpha = clamp(alpha + fresnel * 0.35 + glint, 0.0, 1.0);
        // Une chute (côté d'une cascade, ou pente raide) : elle reste translucide, on y voit
        // couler des filets plus clairs vers le bas.
        let steep = smoothstep(0.55, 1.1, length(in.tex_normal.xz));
        let vertical = abs(in.tex_normal.y) < 0.5;
        if vertical || steep > 0.0 {
            var along = in.world_position.x + in.world_position.z;
            var down = in.world_position.y;
            if !vertical {
                // Sur une pente raide, « en bas » est le long du courant.
                along = dot(in.world_position.xz, vec2<f32>(-flow_dir.y, flow_dir.x));
                down = -dot(in.world_position.xz, flow_dir) * 1.3;
            }
            let strength = select(steep, 1.0, vertical);
            let streaks = fall_streaks(along, down, time) * strength;
            lit = mix(lit, vec3<f32>(0.88, 0.96, 1.0) * (0.45 + 0.55 * daylight), streaks * 0.55);
            alpha = mix(alpha, 0.26 + 0.32 * streaks, strength * 0.85);
        }
    }
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
    var fog = smoothstep(camera.position_fog_start.w, camera.fog_color_end.w, length(offset));
    let underwater = camera.water.x;
    if underwater > 0.0 {
        // Sous l'eau : le rouge s'éteint vite, puis le vert ; au loin tout devient un bleu
        // sombre. Des reflets de lumière dansent sur ce qui est près de la surface.
        let distance = length(in.world_position - camera.position_fog_start.xyz);
        let depth = max(camera.water.y - in.world_position.y, 0.0);
        let shafts = caustics(in.world_position.xz * 0.9 + in.world_position.y * 0.35, time);
        let day = 0.25 + 0.75 * daylight;
        lit *= 1.0 + underwater * shafts * 1.1 * exp(-depth * 0.14) * day;
        let transmit = exp(-distance * vec3<f32>(0.20, 0.075, 0.04));
        let deep = vec3<f32>(0.015, 0.11, 0.17) * day;
        lit = mix(lit, lit * transmit + deep * (vec3<f32>(1.0) - transmit), underwater);
        // Le brouillard du ciel n'a plus de sens sous l'eau : c'est la couleur de l'eau.
        fog *= 1.0 - underwater;
    }
    return vec4<f32>(mix(lit, camera.fog_color_end.rgb, fog), alpha);
}
