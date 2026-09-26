// A Fresnel membrane: nearly invisible face-on, bright at the limb. Faint caustics drift
// across it; listening cools the limb and thinking warms it.
VARYING vec3 lfWorld;
VARYING vec3 lfNormal;
VARYING vec3 lfLocal;
VARYING vec3 lfColor;

void MAIN()
{
    vec3 V = normalize(CAMERA_POSITION - lfWorld);
    float facing = abs(dot(normalize(lfNormal), V));
    float edge = mix(pow(1.0 - facing, 3.8), pow(1.0 - facing, 2.8), legacy);
    float caustic = pow(abs(sin(dot(lfLocal, vec3(0.045, 0.071, 0.033)) * 6.0 + time * 0.3)), 26.0);
    vec3 tint = lfColor + cool * listen * 0.8 + warm * think * 0.6;
    vec3 colour = mix(tint * (0.9 + edge * 2.2), tint * (0.35 + edge * 1.7), legacy);
    colour += cool * caustic * edge * 0.9;
    float cover = mix(alpha * edge * 1.4, alpha * (0.1 + edge * 0.9), legacy);
    FRAGCOLOR = vec4(colour, clamp(cover * (1.0 + voice * 2.0 + flash), 0.0, 1.0));
}
