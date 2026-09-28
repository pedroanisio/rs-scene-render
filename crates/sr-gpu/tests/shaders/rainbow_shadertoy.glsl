// Time-animated Shadertoy tint: the classic "new shader" palette over the input.
uniform float intensity;
void mainImage(out vec4 fragColor, in vec2 fragCoord) {
    vec2 uv = fragCoord / iResolution.xy;
    vec4 src = texture(iChannel0, uv);
    vec3 col = 0.5 + 0.5 * cos(iTime * 2.0 + uv.xyx * 6.2831853 + vec3(0.0, 2.0, 4.0));
    fragColor = vec4(mix(src.rgb, col, 0.6 * intensity), src.a);
}
