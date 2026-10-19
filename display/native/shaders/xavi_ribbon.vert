// Segment endpoints occupy position/normal; Rust controls style, Qt uploads buffers.
VARYING vec3 vpWorld;
VARYING vec2 vRibbon;
VARYING float vSeed;
void MAIN() {
    vec3 a=(MODEL_MATRIX*vec4(VERTEX,1.0)).xyz,b=(MODEL_MATRIX*vec4(NORMAL,1.0)).xyz;
    vec3 av=(VIEW_MATRIX*vec4(a,1.0)).xyz,bv=(VIEW_MATRIX*vec4(b,1.0)).xyz;
    float t0=0.0,t1=1.0;
    vRibbon=UV0;
    vSeed=UV1.x;
    if(av.z>-.11 && bv.z>-.11){POSITION=vec4(2.0,2.0,2.0,1.0);vpWorld=a;return;}
    if(av.z>-.11){t0=(-.11-av.z)/(bv.z-av.z);a=mix(a,b,t0);}
    else if(bv.z>-.11){t1=(-.11-av.z)/(bv.z-av.z);b=mix(a,b,t1);}
    vRibbon.y=mix(t0,t1,UV0.y);
    vpWorld=mix(a,b,UV0.y);
    vec4 ca=VIEWPROJECTION_MATRIX*vec4(a,1.0),cb=VIEWPROJECTION_MATRIX*vec4(b,1.0);
    vec4 cp=mix(ca,cb,UV0.y);
    vec2 delta=(cb.xy/max(.01,cb.w)-ca.xy/max(.01,ca.w))*viewportSize;
    vec2 perpendicular=vec2(-delta.y,delta.x)/max(.001,length(delta));
    float width=lineWidth*(activeLink && (linkStyle==3 || linkStyle==8)?3.0:1.0);
    cp.xy+=perpendicular*(UV0.x-.5)*width*2.0/viewportSize*cp.w;
    POSITION=cp;
}
