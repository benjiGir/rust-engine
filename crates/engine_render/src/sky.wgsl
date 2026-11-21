struct SkyUniform {
    right: vec4<f32>,
    up: vec4<f32>,
    forward: vec4<f32>,
    eye: vec4<f32>,
    sun: vec4<f32>,
    horizon: vec4<f32>,
    weather: vec3<f32>, // couverture, éclair, secondes
    seed: u32,
};

@group(0) @binding(0) var<uniform> sky: SkyUniform;
@group(1) @binding(0) var cloud_image: texture_2d<f32>;
@group(1) @binding(1) var cloud_sampler: sampler;

struct SkyVertex {
    @builtin(position) position: vec4<f32>,
    @location(0) ray: vec3<f32>,
    @location(1) uv: vec2<f32>,
};

@vertex
fn vs_sky(@builtin(vertex_index) index: u32) -> SkyVertex {
    let corner = array(vec2(-1.0, -1.0), vec2(3.0, -1.0), vec2(-1.0, 3.0))[index];
    let ray = sky.forward.xyz + sky.right.xyz * corner.x + sky.up.xyz * corner.y;
    return SkyVertex(vec4(corner, 1.0, 1.0), ray, corner * vec2(0.5, -0.5) + 0.5);
}

fn hash_bits(key: u32) -> f32 {
    var bits = key;
    bits = (bits ^ (bits >> 16u)) * 0x7feb352du;
    bits = (bits ^ (bits >> 15u)) * 0x846ca68bu;
    return f32((bits ^ (bits >> 16u)) >> 8u) * (1.0 / 16777216.0);
}

fn hash(cell: vec2<i32>, salt: u32) -> f32 {
    let p = bitcast<vec2<u32>>(cell);
    return hash_bits((p.x * 0x8da6b343u) ^ (p.y * 0xd8163841u) ^ (salt * 0xcb1ab31fu));
}

fn noise(p: vec2<f32>, salt: u32) -> f32 {
    let cell = vec2<i32>(floor(p));
    let f = fract(p);
    let u = f * f * (3.0 - 2.0 * f);
    return mix(
        mix(hash(cell, salt), hash(cell + vec2(1, 0), salt), u.x),
        mix(hash(cell + vec2(0, 1), salt), hash(cell + vec2(1, 1), salt), u.x),
        u.y,
    );
}

fn volume_noise(p: vec3<f32>, salt: u32) -> f32 {
    let cell = bitcast<vec3<u32>>(vec3<i32>(floor(p)));
    let f = fract(p);
    let t = f * f * (3.0 - 2.0 * f);
    let x0 = cell.x * 0x8da6b343u;
    let x1 = x0 + 0x8da6b343u;
    let y0 = cell.y * 0xd8163841u;
    let y1 = y0 + 0xd8163841u;
    let z0 = (cell.z * 0xcb1ab31fu) ^ salt;
    let z1 = ((cell.z + 1u) * 0xcb1ab31fu) ^ salt;
    let low = mix(mix(hash_bits(x0 ^ y0 ^ z0), hash_bits(x1 ^ y0 ^ z0), t.x),
        mix(hash_bits(x0 ^ y1 ^ z0), hash_bits(x1 ^ y1 ^ z0), t.x), t.y);
    let high = mix(mix(hash_bits(x0 ^ y0 ^ z1), hash_bits(x1 ^ y0 ^ z1), t.x),
        mix(hash_bits(x0 ^ y1 ^ z1), hash_bits(x1 ^ y1 ^ z1), t.x), t.y);
    return mix(low, high, t.z);
}

fn stars(ray: vec3<f32>) -> vec3<f32> {
    // Projection de l'hémisphère sur une grille fixe. Taille filtrée par pixel.
    let p = 135.0 * ray.xz / (1.0 + abs(ray.y));
    let cell = vec2<i32>(floor(p));
    let chance = hash(cell, 6143u);
    let magnitude = hash(cell, 7963u);
    let galactic_latitude = dot(ray, vec3(0.45, 0.55, 0.704));
    let band = exp(-26.0 * galactic_latitude * galactic_latitude);
    let offset = vec2(hash(cell, 827u), hash(cell, 1031u)) * 0.58 + 0.21;
    let radius = mix(0.060, 0.143, pow(magnitude, 3.5));
    let footprint = clamp(max(length(dpdx(p)), length(dpdy(p))) * 0.6, 0.012, 0.26);
    let spot = 1.0 - smoothstep(max(radius - footprint, 0.0), radius + footprint, length(fract(p) - offset));
    let strength = spot * step(mix(0.976, 0.955, band), chance)
        * radius / (radius + footprint) * (0.46 + 0.54 * magnitude);
    let star_color = mix(vec3(0.77, 0.86, 1.0), vec3(1.0, 0.84, 0.68), hash(cell, 3761u));
    let dust = 0.26 + 0.74 * noise(p * 0.082 + 23.0, 1769u);
    return star_color * strength + vec3(0.0045, 0.0050, 0.0086) * band * dust;
}

fn crater(uv: vec2<f32>, salt: u32) -> f32 {
    let cell = vec2<i32>(floor(uv));
    let offset = vec2(hash(cell, salt), hash(cell, salt + 37u)) * 0.42 + 0.29;
    let delta = fract(uv) - offset;
    let radius = mix(0.11, 0.28, hash(cell, salt + 19u));
    let r = length(delta) / radius;
    let bowl = 1.0 - smoothstep(0.42, 0.95, r);
    let rim = smoothstep(0.68, 0.90, r) * (1.0 - smoothstep(0.93, 1.06, r));
    let relief = clamp(dot(delta / radius, vec2(0.70, 0.35)), -1.0, 1.0);
    return (-0.17 * bowl + rim * (0.10 + 0.15 * relief)) * step(0.31, hash(cell, salt + 81u));
}

fn moon_surface(ray: vec3<f32>, direction: vec3<f32>, radius: f32) -> vec3<f32> {
    let axis = select(vec3(0.0, 1.0, 0.0), vec3(1.0, 0.0, 0.0), abs(direction.y) > 0.98);
    let right = normalize(cross(axis, direction));
    let up = cross(direction, right);
    let uv = vec2(dot(ray, right), dot(ray, up)) / radius;
    let depth = sqrt(max(1.0 - dot(uv, uv), 0.0));
    let sphere = vec3(uv, depth);
    let terrain = volume_noise(sphere * 3.2 + vec3(3.0, 7.0, 12.0), 17u) * 0.75
        + volume_noise(sphere * 8.4, 67u) * 0.25;
    let maria = smoothstep(0.37, 0.66, terrain);
    let craters = crater(uv * 3.4 + 21.0, 291u) + crater(uv * 8.2 + 7.6, 117u) * 0.55;
    let light = max(dot(sphere, vec3(-0.25, 0.19, 0.95)), 0.0);
    let albedo = mix(0.26, 0.83, maria) + craters;
    return vec3(0.95, 0.97, 1.0) * max(albedo, 0.08) * (0.13 + 0.98 * sqrt(light));
}

const CLOUD_BOTTOM: f32 = 202.0;
const CLOUD_TOP: f32 = 284.0;

fn cloud_density(position: vec3<f32>, drift: vec2<f32>, overcast: f32, detail: f32) -> f32 {
    let h = (position.y - CLOUD_BOTTOM) / (CLOUD_TOP - CLOUD_BOTTOM);
    if h <= 0.0 || h >= 1.0 {
        return 0.0;
    }
    let p = position.xz - drift;
    let weather = 0.72 * noise(p * 0.0048, sky.seed) + 0.28 * noise(p * 0.011 + 41.3, sky.seed + 5u);
    let ceiling = mix(0.36, 1.0, smoothstep(0.32, 0.82, weather + overcast * 0.12));
    let taper = smoothstep(0.55 * ceiling, ceiling, h);
    let edge = mix(0.535, 0.29, overcast) + taper * 0.45;
    if weather * 0.64 + 0.36 < edge {
        return 0.0;
    }
    let q = vec3(p.x, position.y - CLOUD_BOTTOM, p.y);
    let billows = volume_noise(q * vec3(0.019, 0.025, 0.019), sky.seed + 51u);
    var shape = 0.64 * weather + 0.36 * billows;
    if detail > 0.0 && shape > edge {
        let erosion = volume_noise(q * 0.072 + 7.3, sky.seed + 127u);
        shape -= detail * erosion * 0.095 * (1.0 - smoothstep(edge + 0.06, edge + 0.21, shape));
    }
    return smoothstep(edge, edge + 0.16, shape) * smoothstep(0.0, 0.18, h) * (1.0 - taper);
}

@fragment
fn fs_clouds(in: SkyVertex) -> @location(0) vec4<f32> {
    let ray = normalize(in.ray);
    let sun = normalize(sky.sun.xyz);
    let overcast = clamp(sky.weather.x, 0.0, 1.0);
    let eye = sky.eye.xyz;
    let slope = select(-max(abs(ray.y), 0.0001), max(abs(ray.y), 0.0001), ray.y >= 0.0);
    let lower = (CLOUD_BOTTOM - eye.y) / slope;
    let upper = (CLOUD_TOP - eye.y) / slope;
    let start = max(0.0, min(lower, upper));
    // Les couches s'effacent dans la brume avant l'horizon. Ce plafond borne
    // la longueur à échantillonner même si le rayon frôle le volume.
    let end = min(min(max(lower, upper), 1780.0), start + 260.0);
    if end <= start {
        return vec4(0.0);
    }

    let drift = sky.weather.z * vec2(1.45, 0.36);
    let distance = end - start;
    let step_size = mix(7.0, 16.0, smoothstep(55.0, 240.0, distance));
    let jitter = hash(vec2<i32>(in.position.xy), 199u);
    let step_offset = 0.5 + (jitter - 0.5) * mix(0.07, 0.29, smoothstep(85.0, 245.0, distance));
    let day = smoothstep(-0.16, 0.18, sun.y);
    let dusk = exp(-32.0 * sun.y * sun.y);
    let toward_sun = max(dot(ray, sun), 0.0);
    let flash = sky.weather.y;
    let ambient = mix(vec3(0.005, 0.011, 0.026), vec3(0.34, 0.41, 0.50), day);
    let light_dir = normalize(vec3(sun.x, max(sun.y, 0.10), sun.z));
    let light_through_weather = 1.0 - smoothstep(0.56, 0.96, overcast);
    let direct = mix(vec3(0.99, 0.98, 0.95), vec3(1.0, 0.52, 0.30), dusk * 0.93)
        * smoothstep(-0.19, 0.05, sun.y) * (0.67 + 0.30 * pow(toward_sun, 7.0));
    var transmission = 1.0;
    var scatter = vec3(0.0);
    for (var i = 0u; i < 18u; i += 1u) {
        let base = start + f32(i) * step_size;
        // Un dernier segment partiel évite une marche quand un rayon gagne un échantillon.
        let segment = min(step_size, end - base);
        if segment <= 0.0 {
            break;
        }
        // Décaler légèrement les échantillons évite des stries à l'horizon.
        let t = base + step_offset * segment;
        let p = eye + ray * t;
        let detail = 1.0 - smoothstep(710.0, 1030.0, t);
        let density = cloud_density(p, drift, overcast, detail);
        if density > 0.001 {
            var sunlight = 0.0;
            if sun.y > -0.19 && light_through_weather > 0.0 {
                let near = cloud_density(p + light_dir * 11.0, drift, overcast, 0.0);
                var shadow = density * 0.32 + near * 1.28;
                if t < 890.0 {
                    let far = cloud_density(p + light_dir * 29.0, drift, overcast, 0.0);
                    shadow += far * 1.25 * (1.0 - smoothstep(610.0, 890.0, t));
                }
                sunlight = exp(-shadow * 1.6) * light_through_weather;
            }
            let height = clamp((p.y - CLOUD_BOTTOM) / (CLOUD_TOP - CLOUD_BOTTOM), 0.0, 1.0);
            let core = 0.68 + 0.32 * (1.0 - density);
            var light = ambient * (0.72 + height * 0.62) * core + direct * sunlight;
            light *= 1.0 - 0.55 * overcast;
            light = mix(light, vec3(0.76, 0.83, 0.96), flash * 0.65);
            let haze = smoothstep(330.0, 1710.0, t);
            light = mix(light, sky.horizon.rgb, haze * 0.78);
            let alpha = 1.0 - exp(-density * segment * 0.105 * (1.0 - haze));
            scatter += transmission * alpha * light;
            transmission *= 1.0 - alpha;
            if transmission < 0.015 {
                break;
            }
        }
    }
    return vec4(scatter, 1.0 - transmission);
}

// Diffusion des molécules selon l'angle (Rayleigh) et lobe avant des aérosols (Mie).
// Ces termes analytiques donnent une direction à la lumière sans table précalculée.
fn atmosphere(ray: vec3<f32>, sun: vec3<f32>, overcast: f32) -> vec3<f32> {
    let altitude = max(ray.y, 0.0);
    let sun_dot = dot(ray, sun);
    let rayleigh = 0.75 * (1.0 + sun_dot * sun_dot);
    let mie = pow(0.0625 / (1.5625 - 1.5 * sun_dot), 1.5);
    let day = smoothstep(-0.11, 0.26, sun.y);
    let twilight = smoothstep(-0.30, -0.08, sun.y) * (1.0 - smoothstep(0.0, 0.26, sun.y));
    let sky_blue = vec3(0.038, 0.19, 0.47) * (0.78 + 0.22 * rayleigh);
    let night_blue = vec3(0.0015, 0.003, 0.012);
    var zenith = mix(night_blue, sky_blue, day) + vec3(0.026, 0.036, 0.098) * twilight;
    let gray = dot(zenith, vec3(0.2126, 0.7152, 0.0722));
    zenith = mix(zenith, gray * vec3(0.63, 0.76, 0.94), overcast * 0.87);
    let separation = smoothstep(0.0, 0.20, altitude);
    let haze = sky.horizon.rgb;
    var color = mix(haze, zenith, 1.0 - exp(-altitude * 2.85));
    let sunward = pow(max(sun_dot, 0.0), 3.5);
    let low_air = exp(-altitude * 4.2) * separation;
    let warmth = vec3(0.52, 0.154, 0.031) * sunward;
    let blue_hour = vec3(0.033, 0.055, 0.108) * (1.0 - sunward);
    color += (warmth + blue_hour) * twilight * low_air * (1.0 - overcast);
    let glow = vec3(1.0, 0.77, 0.50) * mie * smoothstep(-0.08, 0.10, sun.y);
    return color + glow * (1.0 - overcast) * (0.17 + 0.20 * twilight) * separation;
}

// Entrées de l'effet de ciel V1, après composition des nuages.
struct SkyEffect {
    color: vec3<f32>,
    ray: vec3<f32>,
    sun: vec3<f32>,
    overcast: f32,
    underwater: f32,
    time: f32,
};

// VOXEL_PACK_BEGIN
fn voxel_sky(input: SkyEffect) -> vec3<f32> {
    return input.color;
}
// VOXEL_PACK_END

@fragment
fn fs_sky(in: SkyVertex) -> @location(0) vec4<f32> {
    let ray = normalize(in.ray);
    let sun = normalize(sky.sun.xyz);
    let overcast = clamp(sky.weather.x, 0.0, 1.0);
    let night = 1.0 - smoothstep(-0.32, -0.11, sun.y);
    let haze = sky.horizon.rgb;
    var color = atmosphere(ray, sun, overcast);
    let sun_dot = dot(ray, sun);
    let moon_dot = -sun_dot;
    let sun_radius = 0.047 * (1.0 + 0.14 * exp(-18.0 * abs(sun.y)));
    let moon_radius = 0.054 * (1.0 + 0.12 * exp(-18.0 * abs(sun.y)));
    let sun_edge = sqrt(1.0 - sun_radius * sun_radius);
    let moon_edge = sqrt(1.0 - moon_radius * moon_radius);
    let aa = clamp(fwidth(sun_dot) * 0.65, 0.000003, 0.00025);
    let horizon_mask = smoothstep(0.0, 0.055, ray.y);
    let disc_horizon = smoothstep(-0.008, 0.012, ray.y);
    let sun_up = smoothstep(-0.048, 0.018, sun.y);
    let moon_up = smoothstep(-0.058, 0.023, -sun.y);
    let sun_disc = smoothstep(sun_edge - aa, sun_edge + aa, sun_dot) * sun_up * disc_horizon;
    let moon_disc = smoothstep(moon_edge - aa, moon_edge + aa, moon_dot) * moon_up * disc_horizon;
    let open_sky = (1.0 - overcast) * (1.0 - overcast);
    if night > 0.0 {
        let turn = sun.xy / max(length(sun.xy), 0.0001);
        let star_ray = vec3(dot(ray.xz, vec2(turn.x, -turn.y)), ray.y, dot(ray.xz, turn.yx));
        color += stars(star_ray) * night * open_sky * horizon_mask * (1.0 - moon_disc);
    }
    let warm = exp(-22.0 * max(sun.y, 0.0) * max(sun.y, 0.0));
    let solar_glow = mix(vec3(1.0, 0.90, 0.73), vec3(1.0, 0.43, 0.12), warm);
    color += solar_glow * 0.32 * pow(max(sun_dot, 0.0), 250.0) * sun_up * open_sky * horizon_mask;
    color += vec3(0.047, 0.072, 0.12) * pow(max(moon_dot, 0.0), 190.0) * moon_up * open_sky * horizon_mask;
    if sun_disc > 0.0 {
        let limb = sqrt(clamp((sun_dot - sun_edge) / (1.0 - sun_edge), 0.0, 1.0));
        let core = mix(vec3(1.32, 1.27, 1.09), vec3(1.38, 0.82, 0.35), warm);
        let edge = mix(vec3(1.08, 0.99, 0.76), vec3(1.08, 0.50, 0.16), warm);
        color = mix(color, mix(edge, core, limb), sun_disc * open_sky);
    }
    if moon_disc > 0.0 {
        let moon = moon_surface(ray, -sun, moon_radius);
        let extinction = mix(vec3(1.0, 0.74, 0.49), vec3(1.0), smoothstep(-0.02, 0.25, -sun.y));
        color = mix(color, moon * extinction, moon_disc * open_sky);
    }

    let jitter = hash(vec2<i32>(in.position.xy), 199u);
    color = mix(color, vec3(0.78, 0.84, 0.94), sky.weather.y * 0.38);
    // L'image intermédiaire contient déjà la lumière multipliée par l'opacité.
    let cloud = textureSampleLevel(cloud_image, cloud_sampler, in.uv, 0.0);
    color = color * (1.0 - cloud.a) + cloud.rgb;
    color = mix(color, haze, sky.horizon.w);
    // Léger tramage avant la conversion sRGB pour adoucir les dégradés sombres.
    let dither = (jitter - 0.5) * 0.0011;
    let tinted = voxel_sky(SkyEffect(color, ray, sun, overcast, sky.horizon.w, sky.weather.z));
    return vec4(max(tinted + dither, vec3(0.0)), 1.0);
}
