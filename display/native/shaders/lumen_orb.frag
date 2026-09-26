// Lumenfield orbs: cut glass with something alive inside.
//
// - Facets: the face normal from screen derivatives, mixed with the smooth one by
//   `facet`, so modules read as cut gems and entities as polished beads.
// - Inside: domain-warped fbm on the orb's own sphere, drifting slowly, with thin bright
//   veins; contrast and relief follow the design's texture settings.
// - Light: the design's warm key and cool back light, wrapped diffuse and a sharp
//   specular, with the metalness and roughness the design exposes.
// - Glow: a core that brightens with the entity's observed activity, and an iridescent
//   rim that brightens with focus and voice. Status tints the rim; selection and hover
//   ring it in the warm accent. Values above 1 are left for bloom to catch.
VARYING vec3 lfWorld;
VARYING vec3 lfLocal;
VARYING vec3 lfSmooth;
VARYING vec4 lfColor;
VARYING vec4 lfData;

float hash3(vec3 p)
{
    p = fract(p * 0.3183099 + 0.1);
    p *= 17.0;
    return fract(p.x * p.y * p.z * (p.x + p.y + p.z));
}

float vnoise(vec3 x)
{
    vec3 i = floor(x);
    vec3 f = fract(x);
    f = f * f * (3.0 - 2.0 * f);
    return mix(mix(mix(hash3(i), hash3(i + vec3(1, 0, 0)), f.x),
                   mix(hash3(i + vec3(0, 1, 0)), hash3(i + vec3(1, 1, 0)), f.x), f.y),
               mix(mix(hash3(i + vec3(0, 0, 1)), hash3(i + vec3(1, 0, 1)), f.x),
                   mix(hash3(i + vec3(0, 1, 1)), hash3(i + vec3(1, 1, 1)), f.x), f.y), f.z);
}

float fbm(vec3 p)
{
    float sum = 0.0;
    float amplitude = 0.5;
    for (int i = 0; i < 4; ++i) {
        sum += amplitude * vnoise(p);
        p = p * 2.03 + vec3(1.7, 9.2, 3.1);
        amplitude *= 0.5;
    }
    return sum;
}

void MAIN()
{
    vec3 V = normalize(CAMERA_POSITION - lfWorld);
    vec3 faceted = normalize(cross(dFdx(lfWorld), dFdy(lfWorld)));
    if (dot(faceted, V) < 0.0)
        faceted = -faceted;
    vec3 N = normalize(mix(normalize(lfSmooth), faceted, facet));

    float seed = lfData.w * 13.0;
    vec3 p = lfLocal * 1.8 + seed;
    float warp = fbm(p * 1.3 + vec3(0.0, time * 0.05, 0.0));
    float n = fbm(p * 2.2 + warp * 1.8);
    float veins = smoothstep(0.08, 0.0, abs(sin((lfLocal.y + n * 0.9) * 9.0 + seed * 6.0)));
    // Relief: the texture tilts the normal a little, as the design's bump map did.
    vec3 tilt = vec3(fbm(p * 2.2 + vec3(0.13, 0.0, 0.0)), fbm(p * 2.2 + vec3(0.0, 0.13, 0.0)),
                     fbm(p * 2.2 + vec3(0.0, 0.0, 0.13))) - n;
    N = normalize(N + tilt * textureDepth * 14.0 * detail);

    float marble = clamp(0.62 + textureContrast * 0.3 * (n - 0.5), 0.2, 1.2);
    vec3 base = lfColor.rgb * marble;
    base = mix(base, base * 1.7 + warm * 0.12, veins * 0.4 * detail);

    vec3 Lw = normalize(warmPosition - lfWorld);
    vec3 Lc = normalize(coolPosition - lfWorld);
    float dw = max(dot(N, Lw), 0.0) * 0.8 + 0.2;
    float dc = max(dot(N, Lc), 0.0) * 0.8 + 0.2;
    float shininess = mix(160.0, 10.0, roughness);
    float sw = pow(max(dot(N, normalize(Lw + V)), 0.0), shininess);
    float sc = pow(max(dot(N, normalize(Lc + V)), 0.0), shininess);
    float specular = mix(0.25, 1.4, metalness) * (1.15 - roughness);
    vec3 colour = base * (ambientLight + warmLight * dw + coolLight * dc);
    colour += (warmLight * sw + coolLight * sc) * specular * mix(vec3(1.0), lfColor.rgb, metalness);

    float ndv = max(dot(N, V), 0.0);
    float fresnel = pow(1.0 - ndv, 3.0);
    float core = pow(ndv, 2.4);
    // Self-light: the orb keeps its own colour whatever the key light does to it, and
    // its heart brightens with what the entity is doing.
    colour += lfColor.rgb * (0.35 + emissive * 0.9);
    colour += lfColor.rgb * core * (0.3 + lfData.x * 1.4) * (0.6 + emissive * 1.6);

    float flags = lfData.z;
    float selected = mod(flags, 2.0);
    float hovered = mod(floor(flags / 2.0), 2.0);
    float status = floor(flags / 4.0);
    vec3 film = 0.5 + 0.5 * cos(6.28318 * (fresnel * 1.1 + vec3(0.0, 0.33, 0.67)) + seed);
    vec3 rimColour = mix(lfColor.rgb * 1.5, film, 0.3);
    if (status > 3.5)
        rimColour = vec3(1.6, 0.25, 0.22);
    else if (status > 2.5)
        rimColour = vec3(0.3, 1.4, 0.55);
    else if (status > 1.5)
        rimColour = vec3(1.6, 1.0, 0.3);
    else if (status > 0.5)
        rimColour = vec3(1.2, 0.85, 0.35);
    colour += rimColour * fresnel * (0.55 + lfData.y * 1.4 + step(0.5, status) * 0.8);
    colour += warm * fresnel * (selected * 2.6 + hovered * 1.3);
    FRAGCOLOR = vec4(colour, 1.0);
}
