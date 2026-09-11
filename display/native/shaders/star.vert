// The starfield behind the planet.
//
// A large sphere drawn from the inside, so the direction from the centre to each fragment
// is a direction on the sky. Same world-space trick the atmosphere uses: MODEL_MATRIX and
// the vertex are all this needs, and the fragment reconstructs the sky direction from the
// interpolated world position.
VARYING vec3 skyDirection;

void MAIN()
{
    skyDirection = normalize((MODEL_MATRIX * vec4(VERTEX, 1.0)).xyz);
    POSITION = MODELVIEWPROJECTION_MATRIX * vec4(VERTEX, 1.0);
}
