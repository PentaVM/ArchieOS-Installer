// A sky of stars, computed rather than textured.
//
// Deterministic: the same direction is the same star on every machine and every frame.
// This is not a photograph of the real sky — it is a field, and it says so by being
// procedural. What it buys is the thing a flat clear-colour background cannot: depth. An
// object with nothing behind it reads as a diagram; an object against stars reads as being
// somewhere.
//
// Restraint is the whole discipline here. Stars are small, sparse and mostly faint, with a
// rare bright one. A sky full of large bright points is a screensaver, not space.
VARYING vec3 skyDirection;

// A cheap 3D hash. Not cryptographic and does not need to be — it needs to be stable and
// well-scattered, and this is both.
float hash(vec3 p)
{
    p = fract(p * 0.3183099 + vec3(0.71, 0.113, 0.419));
    p *= 17.0;
    return fract(p.x * p.y * p.z * (p.x + p.y + p.z));
}

void MAIN()
{
    vec3 dir = normalize(skyDirection);

    // The deep background: a near-black with the faintest cool lift, so the space between
    // stars is not a dead flat colour.
    vec3 space = vec3(0.015, 0.020, 0.032);

    // Quantise the direction into cells and drop at most one star per cell. Two octaves so
    // the field is not a visible grid: a layer of faint stars and a sparse layer of
    // brighter ones. The cells are coarse and the points are given real size on purpose —
    // a star rendered smaller than a pixel is averaged away by antialiasing, which is how
    // the first pass ended up with an empty sky.
    vec3 star = vec3(0.0);
    float scales[2];
    scales[0] = 150.0;
    scales[1] = 68.0;
    for (int i = 0; i < 2; ++i) {
        vec3 q = dir * scales[i];
        vec3 cell = floor(q);
        float present = hash(cell);
        // Only a small fraction of cells hold a star at all — sparse, so the field reads as
        // stars and not as dust.
        float threshold = i == 0 ? 0.986 : 0.980;
        if (present > threshold) {
            // A small, crisp dot with a bright core. Larger soft blobs turn to grey haze
            // under the bloom; a hard point stays a point.
            vec3 offset = fract(q) - 0.5;
            float d2 = dot(offset, offset);
            float point = smoothstep(0.045, 0.0, d2);
            point = point * point;
            // Brightness and a faint colour, both from the hash so they are stable. Most
            // stars sit just under the bloom threshold and stay crisp; only the rare
            // brightest ones cross it and earn a soft halo.
            float magnitude = hash(cell + 4.0);
            float brightness = (i == 0 ? 0.9 : 1.9) * (0.35 + 0.65 * magnitude * magnitude);
            // Most stars are white; a few lean warm or cool, gently.
            float tint = hash(cell + 9.0);
            vec3 colour = mix(vec3(0.75, 0.82, 1.0), vec3(1.0, 0.9, 0.78),
                              smoothstep(0.5, 1.0, tint));
            star += point * brightness * colour;
        }
    }

    FRAGCOLOR = vec4(space + star, 1.0);
}
