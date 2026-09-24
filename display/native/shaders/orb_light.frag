VARYING vec3 fieldPosition;
VARYING float facing;

void MAIN()
{
    // A low frequency sweep adds motion without changing any node or sample position.
    // The visible paths still come only from the TFOG field and observed graph.
    float sweep = 0.86 + 0.14 * sin(fieldPosition.y * 16.0 + phase * 1.7);
    float front = mix(backStrength, 1.0, smoothstep(0.05, 0.8, facing));
    FRAGCOLOR = vec4(tint.rgb * intensity * sweep * front, 1.0);
}
