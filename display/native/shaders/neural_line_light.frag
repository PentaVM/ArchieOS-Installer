// Light surface: weighted category ink. Fog reduces coverage rather than adding
// the pale background color into every line's RGB contribution.
VARYING vec3 nsColour;
VARYING float nsFog;

void MAIN()
{
    float peak = max(max(nsColour.r, nsColour.g), nsColour.b);
    vec3 ink = vec3(0.006, 0.010, 0.016) + nsColour / max(peak, 0.001) * 0.10;
    float coverage = alpha * (1.0 - nsFog) * clamp(peak * 5.0, 0.0, 1.0);
    FRAGCOLOR = vec4(ink * coverage, coverage);
}
