// SREP 72, Shadertoy convention: red in the top 10 px rows of the content box (sr_PixelCoord.y), else blue.
void mainImage(out vec4 fragColor, in vec2 fragCoord) {
    vec4 c = texture(iChannel0, fragCoord / iResolution.xy);
    fragColor = (sr_PixelCoord.y >= 0.0 && sr_PixelCoord.y < 10.0) ? vec4(1.0, 0.0, 0.0, c.a) : vec4(0.0, 0.0, c.a, c.a);
}
