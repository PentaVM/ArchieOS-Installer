// The selection ring, always turned to the camera.
void MAIN()
{
    vec3 centre = (MODEL_MATRIX * vec4(0.0, 0.0, 0.0, 1.0)).xyz;
    float size = length(MODEL_MATRIX[0].xyz);
    vec3 forward = normalize(CAMERA_POSITION - centre);
    vec3 right = normalize(cross(vec3(0.0, 1.0, 0.0), forward) + vec3(1e-5, 0.0, 0.0));
    vec3 up = cross(forward, right);
    vec3 world = centre + (right * VERTEX.x + up * VERTEX.y + forward * VERTEX.z) * size;
    POSITION = VIEWPROJECTION_MATRIX * vec4(world, 1.0);
}
