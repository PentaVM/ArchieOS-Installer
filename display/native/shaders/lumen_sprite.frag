// Ghosts, signals and halos. Halos grow with activity, focus and selection; ghosts and
// signals set `haloGain` to zero and ignore those channels.
VARYING vec2 lfCorner;
VARYING vec4 lfColor;
VARYING vec4 lfData;

void MAIN()
{
    float d = length(lfCorner);
    if (d > 1.0)
        discard;
    float soft = pow(1.0 - d, sharpness);
    float lift = 1.0 + haloGain * (lfData.x * 1.5 + lfData.y * 2.0 + mod(lfData.z, 2.0) * 1.5);
    vec3 tint = mix(lfColor.rgb, tintOverride, tintMix);
    FRAGCOLOR = vec4(tint * soft * alpha * lift, soft * alpha);
}
