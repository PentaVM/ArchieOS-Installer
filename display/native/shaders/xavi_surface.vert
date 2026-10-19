// Native upload adapter for Xavi's Rust mesh, preserving observed instance state.
VARYING vec3 vpWorld;
VARYING vec3 vpLocal;
VARYING vec3 vpNormal;
VARYING vec2 vpUv;
VARYING vec3 vpColor;
VARYING vec4 vpState;
VARYING vec2 vpHybridLod;
void MAIN() {
    float colour = floor(INSTANCE_DATA.x + 0.5);
    vec3 srgb = vec3(floor(colour / 65536.0), floor(mod(colour / 256.0,256.0)),mod(colour,256.0)) / 255.0;
    float state = floor(INSTANCE_DATA.y + 0.5);
    vpColor = pow(srgb,vec3(2.2));
    vpState = vec4(floor(state / 256.0)/255.0,mod(state,256.0)/255.0,INSTANCE_DATA.z,INSTANCE_DATA.w);
    vpLocal = VERTEX;
    vpWorld = (INSTANCE_MODEL_MATRIX * vec4(VERTEX,1.0)).xyz;
    vpNormal = normalize(mat3(INSTANCE_MODEL_MATRIX) * NORMAL);
    vpUv = UV0;
    vec4 viewPosition = VIEW_MATRIX * vec4(vpWorld,1.0);
    float pixels=viewportHeight*abs(PROJECTION_MATRIX[1][1])*length(INSTANCE_MODEL_MATRIX[0].xyz)/max(.2,-viewPosition.z);
    vpHybridLod=vec2(smoothstep(7.0,30.0,pixels),smoothstep(38.0,112.0,pixels));
    POSITION = INSTANCE_MODELVIEWPROJECTION_MATRIX * vec4(VERTEX,1.0);
}
