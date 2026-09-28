// Deliberately invalid GLSL (undeclared identifier) for the error-handling tests.
vec4 transition (vec2 uv) {
  return getFromColor(uv) * notDeclared;
}
void mainImage(out vec4 fragColor, in vec2 fragCoord) { fragColor = vec4(undefinedThing); }
