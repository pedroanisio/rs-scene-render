// Outer glow, scene-render effect convention; grow the tile with <param name="padding">.
uniform float radius;   // effect @radius, output pixels
uniform vec4 color;     // effect @color, straight sRGB
void main() {
    vec2 px = 1.0 / resolution;
    vec4 c = texture(inputTexture, uv);
    float a = 0.0;
    for (int i = 0; i < 24; i++) {
        float ang = 6.2831853 * float(i) / 24.0;
        for (int r = 1; r <= 6; r++) {
            float k = float(r) / 6.0;
            a = max(a, texture(inputTexture, uv + vec2(cos(ang), sin(ang)) * px * radius * k).a * (1.0 - k * k));
        }
    }
    vec4 g = vec4(color.rgb, color.a * a);
    float oa = c.a + g.a * (1.0 - c.a);
    vec3 rgb = oa > 0.0 ? (c.rgb * c.a + g.rgb * g.a * (1.0 - c.a)) / oa : vec3(0.0);
    fragColor = vec4(rgb, oa);
}
