// Rayleigh scattering, cheaply, and now lit by the real sun.
//
// Air is only visible where you look through a lot of it, which is at the limb. The fourth
// power concentrates the glow there instead of hazing the whole disc, and the blue is the
// colour the sky already is for the same reason.
//
// What is new is the sun. `sunDirection` is the true direction to the sun in the Earth's
// own frame, computed from UTC — so the limb is blue where the day faces us, warm at the
// terminator where the light is grazing kilometres of air, and dim on the night side. That
// warm band is the sunset seen from orbit, and it is the single detail that most makes a
// rendered Earth read as a photograph rather than a globe.
//
// Alpha never reaches 1. An atmosphere that hides the coastline under it has stopped being
// atmosphere and started being fog.
VARYING vec3 worldNormal;
VARYING vec3 worldPosition;

void MAIN()
{
    vec3 normal = normalize(worldNormal);
    vec3 toEye = normalize(CAMERA_POSITION - worldPosition);
    float rim = pow(1.0 - abs(dot(normal, toEye)), 4.0);

    // How much this piece of limb faces the sun. +1 is noon, 0 is the terminator, -1 is
    // the far side of midnight.
    float sun = dot(normal, normalize(sunDirection));

    // Blue by day, a warm dusk band at the terminator.
    float day = smoothstep(-0.20, 0.30, sun);
    vec3 blue = vec3(0.12, 0.36, 0.75);
    vec3 dusk = vec3(0.90, 0.48, 0.24);
    vec3 colour = mix(dusk, blue, day);

    // The night limb keeps a trace of glow so the planet has an edge in the dark, but it is
    // a trace: the lit and unlit halves must not weigh the same.
    float lit = smoothstep(-0.55, 0.10, sun);
    float alpha = rim * 0.42 * (0.14 + 0.86 * lit);

    FRAGCOLOR = vec4(colour, alpha);
}
