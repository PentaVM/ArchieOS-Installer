// A soft point turned to face the camera: ghost particles, travelling signals and the
// glow halo around each orb. The instance's scale is the sprite's radius, times `grow`.
VARYING vec2 lfCorner;
VARYING vec4 lfColor;
VARYING vec4 lfData;

void MAIN()
{
    vec3 centre = (INSTANCE_MODEL_MATRIX * vec4(0.0, 0.0, 0.0, 1.0)).xyz;
    float size = length(INSTANCE_MODEL_MATRIX[0].xyz) * grow;
    vec3 forward = normalize(CAMERA_POSITION - centre);
    vec3 right = normalize(cross(vec3(0.0, 1.0, 0.0), forward) + vec3(1e-5, 0.0, 0.0));
    vec3 up = cross(forward, right);
    vec3 world = centre + (right * VERTEX.x + up * VERTEX.y) * size;
    lfCorner = VERTEX.xy;
    lfColor = INSTANCE_COLOR;
    lfData = INSTANCE_DATA;
    POSITION = VIEWPROJECTION_MATRIX * vec4(world, 1.0);
}
