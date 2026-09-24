// A single GPU path for the field's deterministic ribbons and data glints.
// The only motion is a uniform; the CPU never reprojects or paints these points.
VARYING vec3 fieldPosition;
VARYING float facing;

void MAIN()
{
    fieldPosition = VERTEX;
    vec3 worldPosition = (MODEL_MATRIX * vec4(VERTEX, 1.0)).xyz;
    vec3 worldNormal = normalize(mat3(MODEL_MATRIX) * NORMAL);
    facing = abs(dot(worldNormal, normalize(CAMERA_POSITION - worldPosition)));
    POSITION = MODELVIEWPROJECTION_MATRIX * vec4(VERTEX, 1.0);
}
