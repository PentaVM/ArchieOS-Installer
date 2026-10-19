// The membrane: a sphere swollen where orbs gather, tinted by the nearest one. Its
// colour arrives in UV0 (rg) and UV1 (ba). It breathes slowly, and more while Archie
// speaks.
VARYING vec3 lfWorld;
VARYING vec3 lfNormal;
VARYING vec3 lfLocal;
VARYING vec3 lfColor;
VARYING float lfAlpha;

void MAIN()
{
    float swell = 0.35 + voice * 1.4;
    vec3 p = VERTEX;
    if(shellKind < 0 || shellKind == 1)p += NORMAL * sin(time * flow * 0.55 + VERTEX.y * 0.07) * swell;
    if(shellKind == 2){float a=time*flow*.035;mat2 rotation=mat2(cos(a),sin(a),-sin(a),cos(a));p.xz=rotation*p.xz;}
    lfLocal = VERTEX;
    lfColor = vec3(UV0.x, UV0.y, UV1.x);
    lfAlpha = UV1.y;
    lfWorld = (MODEL_MATRIX * vec4(p, 1.0)).xyz;
    lfNormal = normalize(mat3(MODEL_MATRIX) * NORMAL);
    POSITION = MODELVIEWPROJECTION_MATRIX * vec4(p, 1.0);
}
