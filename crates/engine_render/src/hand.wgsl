// Shader du bras et de l'objet tenu (la vue à la première personne).
//
// Tout est en coordonnées de VUE : la caméra est à l'origine et regarde vers -z.
// Le bras ne bouge donc pas quand on tourne la tête : il est « collé » à l'écran.
// Chaque boîte a sa matrice complète (position, rotation, taille), contrairement aux
// entités qui ne tournent qu'autour de la verticale.

struct Hand {
    // La projection propre au bras (son propre champ de vision).
    proj: mat4x4<f32>,
    // x : lumière du ciel, y : lumière des blocs (de 0 à 1) à la position du joueur,
    // z : intensité du jour.
    light: vec4<f32>,
    // rgb : teinte de la lumière du ciel.
    sun_color: vec4<f32>,
};

@group(0) @binding(0)
var<uniform> hand: Hand;

@group(1) @binding(0)
var block_textures: texture_2d_array<f32>;
@group(1) @binding(1)
var block_sampler: sampler;

struct VertexIn {
    // Un cube de (0, 0, 0) à (1, 1, 1).
    @location(0) position: vec3<f32>,
    @location(1) normal: vec3<f32>,
};

struct InstanceIn {
    // La matrice de la boîte, colonne par colonne.
    @location(2) model_0: vec4<f32>,
    @location(3) model_1: vec4<f32>,
    @location(4) model_2: vec4<f32>,
    @location(5) model_3: vec4<f32>,
    @location(6) color: vec4<f32>,
    // Dessus, côtés, dessous ; 0 : couleur unie.
    @location(7) textures: vec3<u32>,
    @location(8) cutout: u32,
    @location(9) uv_min: vec3<f32>,
    @location(10) uv_max: vec3<f32>,
};

struct VertexOut {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) color: vec4<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) uv: vec2<f32>,
    @location(3) @interpolate(flat) texture: u32,
    @location(4) @interpolate(flat) cutout: u32,
};

@vertex
fn vs_main(vertex: VertexIn, instance: InstanceIn) -> VertexOut {
    let model = mat4x4<f32>(instance.model_0, instance.model_1, instance.model_2, instance.model_3);
    // Le cube est centré sur l'origine avant la transformation.
    let local = vertex.position - vec3<f32>(0.5);
    var out: VertexOut;
    out.clip_position = hand.proj * model * vec4<f32>(local, 1.0);
    out.normal = normalize((model * vec4<f32>(vertex.normal, 0.0)).xyz);
    out.color = instance.color;
    out.texture = instance.textures.y;
    out.cutout = instance.cutout;
    // Les boîtes partielles gardent la même découpe de texture que dans le monde.
    let p = mix(instance.uv_min, instance.uv_max, vertex.position);
    let n = abs(vertex.normal);
    if n.y > 0.5 {
        out.texture = select(instance.textures.z, instance.textures.x, vertex.normal.y > 0.0);
        out.uv = p.xz;
    } else if n.x > 0.5 {
        out.uv = vec2<f32>(p.z, 1.0 - p.y);
    } else {
        out.uv = vec2<f32>(p.x, 1.0 - p.y);
    }
    return out;
}

fn brightness(level: f32) -> f32 {
    return pow(0.8, (1.0 - level) * 15.0);
}

@fragment
fn fs_main(in: VertexOut) -> @location(0) vec4<f32> {
    let albedo = in.color * textureSample(block_textures, block_sampler, in.uv, in.texture);
    if in.cutout != 0u && albedo.a < 0.5 { discard; }
    // La lumière du lieu où se tient le joueur : le bras est sombre dans une grotte.
    let sky = brightness(hand.light.x) * hand.light.z * hand.sun_color.rgb;
    let torch = brightness(hand.light.y) * vec3<f32>(1.0, 0.82, 0.6);
    // Un relief simple : une lumière qui vient d'en haut à gauche, dans la vue.
    let facing = max(dot(in.normal, normalize(vec3<f32>(-0.4, 0.8, 0.45))), 0.0);
    let lit = albedo.rgb * (max(sky, torch) + 0.04) * (0.55 + 0.45 * facing);
    return vec4<f32>(lit, 1.0);
}
