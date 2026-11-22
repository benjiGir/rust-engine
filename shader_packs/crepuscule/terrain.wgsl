// Lumière chaude au soleil bas et teinte de l'eau en profondeur.
fn voxel_terrain(input: TerrainEffect) -> vec4<f32> {
    let dusk = exp(-7.0 * input.sun.y * input.sun.y);
    let sunlit = smoothstep(0.05, 0.56, input.daylight);
    let daylight = sunlit * (1.0 - input.underwater);
    let facing = max(dot(input.normal, input.sun), 0.0);
    // La teinte dépend de la lumière incidente, commune aux texels d’une face.
    let warmth = daylight * dusk * facing;
    var color = input.lit * mix(vec3(1.0), vec3(1.055, 1.012, 0.97), warmth);

    if input.water {
        let ray = input.position - input.eye;
        let forward = ray / max(length(ray), 0.01);
        let brightness = dot(input.lit, vec3(0.2126, 0.7152, 0.0722));
        let depth = smoothstep(0.15, 4.0, input.water_depth);
        let looking_down = clamp(abs(dot(forward, input.normal)), 0.0, 1.0);
        let tint = mix(vec3(0.93, 1.13, 1.04), vec3(0.66, 1.04, 1.18), depth);
        let reflection = 1.0 - smoothstep(0.36, 0.99, brightness);
        color *= mix(vec3(1.0), tint, looking_down * reflection * 0.90);
    }

    return vec4(mix(color, input.fog_color, input.fog), input.alpha);
}
