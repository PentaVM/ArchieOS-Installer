// The halo around the limb of the planet.
//
// World space rather than view space. Qt Quick 3D has no MODELVIEW_MATRIX — the earlier
// version of this named one and the shader never compiled, so the atmosphere was in the
// scene and invisible. MODEL_MATRIX and CAMERA_POSITION are both documented and both
// available here, and the angle between a surface and the eye is the same question
// asked in either space.
VARYING vec3 worldNormal;
VARYING vec3 worldPosition;

void MAIN()
{
    worldNormal = normalize(mat3(MODEL_MATRIX) * NORMAL);
    worldPosition = (MODEL_MATRIX * vec4(VERTEX, 1.0)).xyz;
    POSITION = MODELVIEWPROJECTION_MATRIX * vec4(VERTEX, 1.0);
}
