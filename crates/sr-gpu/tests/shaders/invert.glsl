// Invert, scene-render effect convention: inputTexture/uv/fragColor are declared by the harness.
uniform float amount; // = 1.0
void main() {
    vec4 c = texture(inputTexture, uv);
    fragColor = vec4(mix(c.rgb, 1.0 - c.rgb, amount), c.a);
}
