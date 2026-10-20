// A glowing ball: the sphere's core and the haze behind it. Brightest where the surface
// faces the eye, fading to nothing at its rim, so it reads as light rather than a solid.
VARYING vec3 glowNormal;
VARYING vec3 glowView;

void MAIN()
{
    vec4 world = MODEL_MATRIX * vec4(VERTEX, 1.0);
    glowNormal = normalize(NORMAL_MATRIX * NORMAL);
    glowView = CAMERA_POSITION - world.xyz;
    POSITION = MODELVIEWPROJECTION_MATRIX * vec4(VERTEX, 1.0);
}
