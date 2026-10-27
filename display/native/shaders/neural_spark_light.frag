VARYING float nsFade;
VARYING vec3 nsColour;

void MAIN()
{
    float r = length(gl_PointCoord - 0.5);
    if (r > 0.5)
        discard;
    float peak = max(max(nsColour.r, nsColour.g), nsColour.b);
    vec3 ink = vec3(0.006, 0.010, 0.016) + nsColour / max(peak, 0.001) * 0.10;
    float coverage = clamp(smoothstep(0.5, 0.0, r) * nsFade * brightness, 0.0, 1.0);
    FRAGCOLOR = vec4(ink * coverage, coverage);
}
