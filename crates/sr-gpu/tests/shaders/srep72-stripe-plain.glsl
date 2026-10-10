// SREP 72, scene-render effect convention: red in the first 10 px of the content box (sr_PixelCoord), else blue.
void main() {
    vec4 c = texture(inputTexture, uv);
    fragColor = (sr_PixelCoord.x >= 0.0 && sr_PixelCoord.x < 10.0) ? vec4(1.0, 0.0, 0.0, c.a) : vec4(0.0, 0.0, c.a, c.a);
}
