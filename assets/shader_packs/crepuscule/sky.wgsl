// Teinte du ciel et lumière diffusée par les nuages près de l'horizon.
fn voxel_sky(input: SkyEffect) -> vec3<f32> {
    let daylight = smoothstep(-0.18, 0.06, input.sun.y);
    let horizon = exp(-7.0 * abs(input.ray.y));
    let sunward = max(dot(input.ray, input.sun), 0.0);
    let low_sun = exp(-8.0 * input.sun.y * input.sun.y);
    let dusk = low_sun * daylight * horizon * (0.25 + 0.75 * pow(sunward, 4.0));
    let open = 1.0 - input.overcast * 0.72;
    let zenith = vec3(0.77, 0.91, 1.13);
    let glow = vec3(0.29, 0.106, 0.028) * pow(sunward, 9.0) * low_sun * daylight * open;
    let moon_up = smoothstep(0.03, 0.31, -input.sun.y);
    let moonward = max(dot(input.ray, -input.sun), 0.0);
    let moon_glow = vec3(0.046, 0.078, 0.135) * pow(moonward, 31.0) * moon_up * open;
    let sky = input.color * mix(zenith, vec3(1.29, 1.04, 0.82), dusk * open) + glow + moon_glow;
    return mix(sky, input.color, input.underwater);
}
