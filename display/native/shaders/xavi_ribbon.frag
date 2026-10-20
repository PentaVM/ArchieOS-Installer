// Delivered Xavi ribbon patterns/pulses; parent->child UV direction is preserved.
VARYING vec3 vpWorld;
VARYING vec2 vRibbon;
VARYING float vSeed;
void MAIN() {
    float t=smoothstep(0.0,1.0,clamp(dot(vpWorld-(warmPosition+coolPosition)*.5,normalize(warmPosition-coolPosition))/max(1.0,fieldRadius)*.5+.5,0.0,1.0));
    vec3 colour=mix(cool,warm,t);
    float y=vRibbon.y,edge=1.0-smoothstep(.30,.5,abs(vRibbon.x-.5)),pattern=1.0,light=1.0;
    if(linkStyle==5)pattern=smoothstep(.10,.16,fract(y*12.0-time*.35));
    if(linkStyle==7){float packet=fract(y*16.0-time*.8);pattern=step(.28,packet);light=1.0+1.8*step(.8,packet);}
    if(linkStyle==3 && activeLink){float center=.5+.24*sin(y*37.7+time*.7);edge=exp(-pow((vRibbon.x-center)*15.0,2.0))+exp(-pow((vRibbon.x-(1.0-center))*15.0,2.0));}
    if(linkStyle==4)light=1.0+.5*(step(y,.025)+step(.975,y));
    if(linkStyle==6){colour=mix(colour,colour.bgr,.25*sin(y*30.0));light=.8+.7*pow(max(0.0,sin(y*20.0-time*1.5)),12.0);}
    if(linkStyle==8 && activeLink){edge=pow(max(0.0,1.0-abs(vRibbon.x-.5)*2.0),1.8);light=1.0+1.6*pow(max(0.0,sin(y*18.0-time*2.0)),3.0);}
    if(pulseStyle>0 && activeLink){
        float d=abs(y-fract(time*pulseSpeed*.35+vSeed)),packet=0.0;
        if(pulseStyle==1)packet=exp(-d*d*180.0);
        else if(pulseStyle==2)packet=exp(-d*d*1800.0);
        else if(pulseStyle==3)packet=step(d,.015)*.4;
        else if(pulseStyle==4)packet=step(d,.065)*(.5+.5*sin(y*100.0));
        else if(pulseStyle==5)packet=pow(max(0.0,1.0-d*25.0),3.0);
        else if(pulseStyle==6)packet=step(d,.028);
        else packet=exp(-d*d*280.0)*(.8+.2*sin(time*3.0));
        light+=packet*pulseIntensity*3.0;pattern*=.12+packet*.88;
    }
    // The base route remains visible. Emission eligibility follows source modes.
    float weight=bloomMode==1?focusWeight:bloomMode==2?max(focusWeight,smoothstep(.35,1.0,abs(t-.5)*2.0)):bloomMode==3?0.0:1.0;
    FRAGCOLOR=vec4(colour*(1.0+(light-1.0)*weight)*intensity,alpha*edge*pattern);
}
