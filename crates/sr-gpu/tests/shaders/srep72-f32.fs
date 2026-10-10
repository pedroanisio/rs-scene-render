/*{"ISFVSN":"2","DESCRIPTION":"SREP 72 kit: stores 2049 in a FLOAT pass target and reads it back; red when exact",
"INPUTS":[{"NAME":"inputImage","TYPE":"image"}],"PASSES":[{"TARGET":"buf","FLOAT":true},{}]}*/
void main() {
    if (PASSINDEX == 0) {
        gl_FragColor = vec4(2049.0, 0.0, 0.0, 1.0);
    } else {
        float v = IMG_NORM_PIXEL(buf, isf_FragNormCoord).r;
        vec4 src = IMG_NORM_PIXEL(inputImage, isf_FragNormCoord);
        gl_FragColor = (v == 2049.0) ? vec4(1.0, 0.0, 0.0, src.a) : vec4(0.0, 0.0, 1.0, src.a);
    }
}
