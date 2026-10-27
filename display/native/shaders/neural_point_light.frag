// Light surface: category ink, with the original round/ring coverage in alpha.
VARYING vec3 nsColour;
VARYING float nsAlpha;
VARYING float nsRing;

void MAIN()
{
    float r = length(gl_PointCoord - 0.5);
    if (r > 0.5)
        discard;
    float a = nsRing > 0.5 ? smoothstep(0.08, 0.0, abs(r - 0.36)) + smoothstep(0.16, 0.0, r) : smoothstep(0.5, 0.0, r);
    float peak = max(max(nsColour.r, nsColour.g), nsColour.b);
    vec3 ink = vec3(0.006, 0.010, 0.016) + nsColour / max(peak, 0.001) * 0.10;
    float coverage = clamp(a * nsAlpha * brightness, 0.0, 1.0);
    FRAGCOLOR = vec4(ink * coverage, coverage);
}
