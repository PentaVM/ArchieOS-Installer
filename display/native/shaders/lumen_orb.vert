// Lumenfield orbs. Each instance is an icosphere pushed out of round by two octaves of
// value noise, seeded per entity, so no two orbs share a silhouette and none is a plain
// ball; `shapeAmount` sets how far. The shape breathes slowly with the orb's activity.
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

void MAIN()
{
    // Colour, activity and focus arrive packed in the data channel (see LumenField).
    float packedColour = floor(INSTANCE_DATA.x + 0.5);
    vec3 srgb = vec3(floor(packedColour / 65536.0), floor(mod(packedColour / 256.0, 256.0)), mod(packedColour, 256.0)) / 255.0;
    float packedState = floor(INSTANCE_DATA.y + 0.5);
    float activity = floor(packedState / 256.0) / 255.0;
    float attention = mod(packedState, 256.0) / 255.0;
    vec3 dir = normalize(VERTEX);
    float seed = INSTANCE_DATA.w * 37.0;
    float lump = vnoise(dir * 2.2 + seed) * 0.62 + vnoise(dir * 5.3 + seed * 1.7) * 0.38;
    float breathe = 0.5 + 0.5 * sin(time * (0.7 + activity * 1.6) + seed * 6.0);
    vec3 local = dir * (1.0 + (lump - 0.5) * shapeAmount * (0.8 + 0.2 * breathe));
    lfLocal = dir;
    lfWorld = (INSTANCE_MODEL_MATRIX * vec4(local, 1.0)).xyz;
    lfSmooth = normalize(mat3(INSTANCE_MODEL_MATRIX) * dir);
    lfColor = vec4(pow(srgb, vec3(2.2)), 1.0);
    lfData = vec4(activity, attention, INSTANCE_DATA.z, INSTANCE_DATA.w);
    POSITION = INSTANCE_MODELVIEWPROJECTION_MATRIX * vec4(local, 1.0);
}
