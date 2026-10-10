/*{"ISFVSN":"2","DESCRIPTION":"SREP 72 kit: red in the first 10 px of the content box (sr_PixelCoord), the input elsewhere",
"INPUTS":[{"NAME":"inputImage","TYPE":"image"}]}*/
void main() {
    vec4 src = IMG_NORM_PIXEL(inputImage, isf_FragNormCoord);
    gl_FragColor = (sr_PixelCoord.x >= 0.0 && sr_PixelCoord.x < 10.0) ? vec4(1.0, 0.0, 0.0, src.a) : vec4(0.0, 0.0, src.a, src.a);
}
