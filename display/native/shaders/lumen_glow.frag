VARYING vec3 glowNormal;
VARYING vec3 glowView;

void MAIN()
{
    float facing = max(dot(normalize(glowNormal), normalize(glowView)), 0.0);
    float light = pow(facing, sharpness);
    // Added light: the colour carries the whole strength, so it is not squared by alpha.
    FRAGCOLOR = vec4(tint * light * gain * alpha, 1.0);
}
