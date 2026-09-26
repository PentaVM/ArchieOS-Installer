// The membrane: a sphere swollen where orbs gather, tinted by the nearest one. Its
// colour arrives in UV0 (rg) and UV1 (ba). It breathes slowly, and more while Archie
// speaks.
VARYING vec3 lfWorld;
VARYING vec3 lfNormal;
VARYING vec3 lfLocal;
VARYING vec3 lfColor;

void MAIN()
{
    float swell = 0.35 + voice * 1.4;
    vec3 p = VERTEX + NORMAL * sin(time * 0.55 + VERTEX.y * 0.07) * swell;
    lfLocal = VERTEX;
    lfColor = vec3(UV0.x, UV0.y, UV1.x);
    lfWorld = (MODEL_MATRIX * vec4(p, 1.0)).xyz;
    lfNormal = normalize(mat3(MODEL_MATRIX) * NORMAL);
    POSITION = MODELVIEWPROJECTION_MATRIX * vec4(p, 1.0);
}
