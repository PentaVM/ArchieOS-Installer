#version 440
// The hologram's grade (Xavi's desktop sphere). His shell laid a CSS filter over the
// sphere's finished canvas: hue-rotate, saturate, contrast, brightness. This pass does the
// same, in that order, on the finished frame of the 3D view, then adds the hologram's
// screen: faint scan lines and a small red/cyan split.

layout(location = 0) in vec2 qt_TexCoord0;
layout(location = 0) out vec4 fragColor;

layout(std140, binding = 0) uniform buf {
    mat4 qt_Matrix;
    float qt_Opacity;
    float hue;
    float saturation;
    float contrast;
    float brightness;
    float scanlines;
    float split;
    float glow;
    float glowTaps;
    vec2 size;
};
layout(binding = 1) uniform sampler2D source;

// The Filter Effects matrices, each step clamped as feColorMatrix does.
vec3 hueRotate(vec3 c, float degrees)
{
    float a = radians(degrees);
    float k = cos(a);
    float s = sin(a);
    vec3 r = vec3(0.213 + k * 0.787 - s * 0.213, 0.715 - k * 0.715 - s * 0.715, 0.072 - k * 0.072 + s * 0.928);
    vec3 g = vec3(0.213 - k * 0.213 + s * 0.143, 0.715 + k * 0.285 + s * 0.140, 0.072 - k * 0.072 - s * 0.283);
    vec3 b = vec3(0.213 - k * 0.213 - s * 0.787, 0.715 - k * 0.715 + s * 0.715, 0.072 + k * 0.928 + s * 0.072);
    return clamp(vec3(dot(r, c), dot(g, c), dot(b, c)), 0.0, 1.0);
}

vec3 saturateBy(vec3 c, float s)
{
    vec3 r = vec3(0.213 + 0.787 * s, 0.715 - 0.715 * s, 0.072 - 0.072 * s);
    vec3 g = vec3(0.213 - 0.213 * s, 0.715 + 0.285 * s, 0.072 - 0.072 * s);
    vec3 b = vec3(0.213 - 0.213 * s, 0.715 - 0.715 * s, 0.072 + 0.928 * s);
    return clamp(vec3(dot(r, c), dot(g, c), dot(b, c)), 0.0, 1.0);
}

vec3 graded(vec3 c)
{
    c = hueRotate(clamp(c, 0.0, 1.0), hue);
    c = saturateBy(c, saturation);
    c = clamp((c - 0.5) * contrast + 0.5, 0.0, 1.0);
    return clamp(c * brightness, 0.0, 1.0);
}

// The design's bloom (Three.js UnrealBloomPass), light enough to live in this pass: the
// bright part of a ring of taps around the pixel, added before the grade. `glowTaps` is
// 8 or 16, as the render budget allows.
vec3 bloomAt(vec2 uv)
{
    if (glow <= 0.0)
        return vec3(0.0);
    vec2 pixel = 1.0 / max(vec2(1.0), size);
    float reach = max(1.0, size.y / 1080.0);
    vec3 sum = vec3(0.0);
    for (int i = 0; i < 16; ++i) {
        if (float(i) >= glowTaps)
            break;
        float a = float(i) * 2.3999632;
        float r = (1.5 + 4.5 * fract(float(i) * 0.618034)) * reach;
        vec3 c = texture(source, uv + vec2(cos(a), sin(a)) * r * pixel).rgb;
        sum += max(c - 0.08, 0.0);
    }
    return sum / max(1.0, glowTaps) * glow;
}

void main()
{
    vec2 shift = vec2(split / max(1.0, size.x), 0.0);
    vec4 centre = texture(source, qt_TexCoord0);
    vec3 bloom = bloomAt(qt_TexCoord0);
    vec3 colour = graded(centre.rgb + bloom);
    if (split > 0.0) {
        colour.r = graded(texture(source, qt_TexCoord0 + shift).rgb + bloom).r;
        colour.b = graded(texture(source, qt_TexCoord0 - shift).rgb + bloom).b;
    }
    // One darker row in three, as the hologram's screen.
    float row = mod(floor(qt_TexCoord0.y * size.y), 3.0);
    colour *= 1.0 - scanlines * 0.42 * step(1.5, row);
    fragColor = vec4(colour, 1.0) * qt_Opacity;
}
