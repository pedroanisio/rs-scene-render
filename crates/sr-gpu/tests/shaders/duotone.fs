/*{
  "DESCRIPTION": "Two-pass ISF duotone: pass 0 renders a quarter-size luma buffer, pass 1 maps it.",
  "ISFVSN": "2",
  "INPUTS": [
    {"NAME": "inputImage", "TYPE": "image"},
    {"NAME": "dark", "TYPE": "color", "DEFAULT": [0.1, 0.0, 0.3, 1.0]},
    {"NAME": "light", "TYPE": "color", "DEFAULT": [1.0, 0.8, 0.2, 1.0]},
    {"NAME": "amount", "TYPE": "float", "DEFAULT": 1.0}
  ],
  "PASSES": [
    {"TARGET": "lumaBuf", "WIDTH": "floor($WIDTH/4.0)", "HEIGHT": "floor($HEIGHT/4.0)"},
    {}
  ]
}*/
void main() {
    if (PASSINDEX == 0) {
        vec4 c = IMG_NORM_PIXEL(inputImage, isf_FragNormCoord);
        float l = dot(c.rgb, vec3(0.2126, 0.7152, 0.0722));
        gl_FragColor = vec4(l, l, l, c.a);
    } else {
        vec4 src = IMG_THIS_PIXEL(inputImage);
        float l = IMG_NORM_PIXEL(lumaBuf, isf_FragNormCoord).r;
        vec3 tone = mix(dark.rgb, light.rgb, l);
        gl_FragColor = vec4(mix(src.rgb, tone, amount), src.a);
    }
}
