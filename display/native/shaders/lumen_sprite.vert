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
    float packedColour = floor(INSTANCE_DATA.x + 0.5);
    vec3 srgb = vec3(floor(packedColour / 65536.0), floor(mod(packedColour / 256.0, 256.0)), mod(packedColour, 256.0)) / 255.0;
    float packedState = floor(INSTANCE_DATA.y + 0.5);
    lfColor = vec4(pow(srgb, vec3(2.2)), 1.0);
    lfData = vec4(floor(packedState / 256.0) / 255.0, mod(packedState, 256.0) / 255.0, INSTANCE_DATA.z, INSTANCE_DATA.w);
    // What the system is using swells its halo, up to two and a half times.
    world = centre + (world - centre) * (1.0 + haloGain * lfData.y * 1.7);
    POSITION = VIEWPROJECTION_MATRIX * vec4(world, 1.0);
}
