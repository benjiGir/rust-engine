// Ombres froides, lumière basse chaude et brume liée au relief.
fn voxel_terrain(input: TerrainEffect) -> vec4<f32> {
    let ray = input.position - input.eye;
    let forward = ray / max(length(ray), 0.01);
    let dusk = exp(-7.0 * input.sun.y * input.sun.y);
    let sunlit = smoothstep(0.05, 0.56, input.daylight);
    let daylight = sunlit * (1.0 - input.underwater);
    let facing = max(dot(input.normal, input.sun), 0.0);
    let brightness = dot(input.lit, vec3(0.2126, 0.7152, 0.0722));
    let dark = 1.0 - smoothstep(0.08, 0.39, brightness);
    var color = input.lit * mix(vec3(1.0), vec3(0.83, 0.95, 1.15), dark * daylight);
    color *= mix(vec3(1.0), vec3(1.27, 1.08, 0.80), daylight * dusk * (0.27 + facing * 0.73) * (1.0 - dark));

    if input.water {
        let depth = smoothstep(0.15, 4.0, input.water_depth);
        let looking_down = clamp(abs(dot(forward, input.normal)), 0.0, 1.0);
        let tint = mix(vec3(0.93, 1.13, 1.04), vec3(0.66, 1.04, 1.18), depth);
        let reflection = 1.0 - smoothstep(0.36, 0.99, brightness);
        color *= mix(vec3(1.0), tint, looking_down * reflection * 0.90);
    }

    // Une densité bornée dépend de l'altitude du trajet. Le ciel et l'eau
    // gardent leurs composantes de brume calculées par le moteur.
    let midpoint = 0.5 * (input.eye.y + input.position.y);
    let valley = 1.0 - smoothstep(68.0, 119.0, midpoint);
    let density = mix(0.0019, 0.0082, valley);
    let scattering = (1.0 - exp(-max(input.distance - 9.0, 0.0) * density)) * daylight * 0.85;
    let fog = input.fog + scattering * (1.0 - input.fog);
    let solar_lobe = pow(max(dot(forward, input.sun), 0.0), 6.0) * dusk * daylight;
    let mist = input.fog_color * mix(vec3(0.89, 1.02, 1.13), vec3(1.18, 1.03, 0.83), solar_lobe);
    return vec4(mix(color, mist, fog), input.alpha);
}
