// Soft bands drifting along the bar. `values[0]` is the volume.
fn shade(uv: vec2f, size: vec2f) -> vec4f {
    let x = uv.x * 6.2832;
    let wave = sin(x * 1.3 + time * 0.6) * 0.22 + sin(x * 2.9 - time * 1.1) * 0.08;
    let band = smoothstep(0.45, 0.0, abs(uv.y - 0.5 - wave));
    let base = vec3f(0.12, 0.12, 0.18);
    let glow = mix(vec3f(0.54, 0.71, 0.98), vec3f(0.80, 0.65, 0.97), uv.x);
    let level = 0.35 + 0.65 * values[0];
    return vec4f(mix(base, glow, band * level), 0.92);
}
