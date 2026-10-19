// XaviCottier: delivered surface emission/pole formulas. Qt's shader dialect is
// the native host exception; linear packed maps are baked by Rust, never JS.
VARYING vec3 vpWorld;
VARYING vec3 vpLocal;
VARYING vec3 vpNormal;
VARYING vec2 vpUv;
VARYING vec3 vpColor;
VARYING vec4 vpState;
VARYING vec2 vpHybridLod;
float poleT(vec3 p) {
    vec3 axis=normalize(warmPosition-coolPosition);
    return smoothstep(0.0,1.0,clamp(dot(p-(warmPosition+coolPosition)*0.5,axis)/max(1.0,fieldRadius)*0.5+0.5,0.0,1.0));
}
void MAIN() {
    vec3 V=normalize(CAMERA_POSITION-vpWorld);
    vec3 N=normalize(vpNormal);
    vec3 faceted=normalize(cross(dFdx(vpWorld),dFdy(vpWorld)));
    if(dot(faceted,V)<0.0)faceted=-faceted;
    if(designStyle!=4 && designStyle!=7 && designStyle!=8)N=faceted;
    vec4 pack=texture(surfaceMap,vpUv);
    vec3 base=texture(albedoMap,vpUv).rgb;
    // Derivative tangent frame, independent of mesh tessellation and UV seams.
    vec3 q0=dFdx(vpWorld),q1=dFdy(vpWorld);
    vec2 st0=dFdx(vpUv),st1=dFdy(vpUv);
    vec3 p1=cross(q1,N),p0=cross(N,q0);
    vec3 T=p1*st0.x+p0*st1.x,B=p1*st0.y+p0*st1.y;
    float inv=inversesqrt(max(1e-8,max(dot(T,T),dot(B,B))));
    N=normalize(N+(T*(pack.r*2.0-1.0)+B*(pack.g*2.0-1.0))*inv*textureDepth*textureContrast*5.0);
    float t=poleT(vpWorld);
    vec3 tint=mix(cool,warm,t);
    base*=vpColor;
    base=mix(base,base*tint*1.6,poleMix);
    float hybridDetail=vpHybridLod.x*hybridReady,hybridNear=vpHybridLod.y*hybridReady;
    vec2 hybridUv=vpUv*max(1.0,detailFrequency);
    vec4 structure=vec4(1.0,0.0,.5,.5),surface=vec4(.5,.5,1.0,.6);
    vec3 optical=vec3(0.0);
    if(hybridDetail>.01){
        structure=texture(hybridStructure,hybridUv);surface=texture(hybridSurface,hybridUv);
        base*=mix(1.0,.28+1.15*structure.r,hybridDetail);
        vec3 micro=surface.xyz*2.0-1.0;
        N=normalize(N+hybridDetail*hybridParameters.w*.65*inv*(T*micro.x+B*micro.y));
    }
    if(hybridNear>.01 && hybridParameters.x>.001){
        vec3 viewDirection=normalize(CAMERA_POSITION-vpWorld);
        vec3 viewSpaceDirection=normalize(mat3(VIEW_MATRIX)*viewDirection);
        vec2 parallax=viewSpaceDirection.xy/max(.25,abs(viewSpaceDirection.z))*hybridParameters.x;
        vec2 inner=hybridUv+parallax*(structure.b-.5);
        float depth=texture(hybridStructure,inner).b;
        float fresnel=pow(1.0-clamp(dot(N,viewDirection),0.0,1.0),3.0);
        float dispersion=hybridParameters.y*(.2+fresnel);
        optical=vec3(texture(hybridStructure,inner+vec2(dispersion,0.0)).b,depth,texture(hybridStructure,inner-vec2(dispersion,0.0)).b);
        base*=mix(1.0,.48+.95*depth,hybridNear*structure.a);
        base+=(optical-vec3(depth))*.36*hybridNear+optical*.12*hybridNear*fresnel;
    }
    float pattern=0.0;
    if(designStyle==1)pattern=clamp((pack.a-.4)/.25,0.0,1.0);
    else if(designStyle==2)pattern=clamp(1.0-(pack.a-.18)/.27,0.0,1.0);
    else if(designStyle==3)pattern=clamp((pack.a-.4)/.22,0.0,1.0);
    else if(designStyle==5)pattern=clamp((pack.a-.12)/.2,0.0,1.0);
    else if(designStyle==6)pattern=clamp((.7-pack.a)/.4,0.0,1.0);
    else pattern=.5+.5*sin(vpLocal.y*12.0);
    float fresnel=1.0-max(0.0,dot(N,V)),glow=0.0;
    if(designStyle==1)glow=pow(fresnel,4.0)*.4+pow(pattern,8.0)*.12;
    else if(designStyle==2)glow=pow(pattern,6.0)*1.8+pow(fresnel,5.0)*.5;
    else if(designStyle==3)glow=pow(fresnel,8.0)*.12;
    else if(designStyle==4){float scan=.5+.5*sin(vpLocal.y*45.0*detailFrequency-time*2.0);base*=.5+.45*scan;glow=pow(fresnel,2.0)*1.5+pow(scan,12.0)*.45;}
    else if(designStyle==5)glow=pow(fresnel,3.0)*1.2+pow(1.0-pattern,14.0)*.7;
    else if(designStyle==6)glow=step(.82,pattern)*(.7+.3*sin(time*1.8+vpLocal.y*4.0))*1.9;
    else if(designStyle==7)glow=pow(fresnel,1.4)*2.0+(.5+.5*sin(vpLocal.y*5.0+vpLocal.x*3.0-time))*.5;
    float selectedNeighborhood=step(32.0,vpState.z);
    float focus=max(selectedNeighborhood,max(vpState.y,mod(vpState.z,2.0)));
    float weight=bloomMode==1?focus:bloomMode==2?max(focus,smoothstep(.35,1.0,abs(t-.5)*2.0)):bloomMode==3?0.0:1.0;
    float r=clamp(roughness*(.45+.8*pack.a)*mix(1.0,.48+surface.a*.85,hybridDetail),.045,1.0);
    vec3 Lw=normalize(warmPosition-vpWorld),Lc=normalize(coolPosition-vpWorld);
    float diffuse=.28+.62*max(0.0,dot(N,Lw))+.3*max(0.0,dot(N,Lc));
    float shine=mix(140.0,12.0,r);
    vec3 colour=base*diffuse+(warm*pow(max(0.0,dot(N,normalize(Lw+V))),shine)*.9+cool*pow(max(0.0,dot(N,normalize(Lc+V))),shine)) *mix(.35,1.2,metalness)*(1.1-r);
    colour+=tint*glow*glowMaskIntensity*emissive*weight;
    colour+=tint*structure.g*(.65+.35*sin(time*1.45+structure.b*12.0))*(.09+.91*max(vpState.x,vpState.y))*hybridParameters.z*hybridDetail*weight;
    // Existing system activity/status signals remain visible through every skin.
    colour+=base*(vpState.x*.25+vpState.y*.5);
    float status=floor(mod(vpState.z,32.0)/4.0);
    vec3 statusTint=status>3.5?vec3(1.6,.25,.22):status>2.5?vec3(.3,1.4,.55):warm;
    colour+=statusTint*step(.5,status)*pow(fresnel,3.0)*.3;
    if(selectionActive && selectedNeighborhood<.5)colour*=1.0-focusDim;
    FRAGCOLOR=vec4(colour,1.0);
}
