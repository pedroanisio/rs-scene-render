//! Diagnostic codes of the evaluation engine.

/// `(code, summary)` for every evaluation diagnostic.
pub const CODES: &[(&str, &str)] = &[
    ("E01", "An expression or condition does not compile: syntax error, unknown name or function, wrong argument count."),
    ("E02", "An animate, expression or link names a property its element does not have."),
    ("E03", "Expressions and links reference each other in a cycle through prop() or a node source."),
    ("E04", "Keyframes cannot compile: a value does not parse as the property's type, units are mixed, a bezier is invalid, or times decrease."),
    ("E05", "An override or bind targets an id that does not exist in its scope."),
    ("E06", "An override or bind sets an attribute the target does not declare, or a value its type rejects."),
    ("E07", "A parameter value violates its type, range, length, pattern or options, or names no parameter."),
    ("E08", "A required parameter has no value from its default, the variant, the data row or the command line."),
    ("E09", "A data source cannot be read or parsed, or the selected row does not exist."),
    ("E10", "A repeat's @over names a parameter that is not a list, or a variant or layout does not exist."),
    ("E11", "Transform parenting through @parent forms a cycle or names a node outside the composition."),
    ("E12", "An include cannot be loaded, is invalid, or names a symbol its document lacks."),
    ("E13", "A symbol or include instantiates itself, directly or through other instances."),
    ("E14", "A link source is not nodeId.property, param:name, audio:track[:band] or marker:id."),
    ("E15", "A motion path's SVG data does not parse."),
    ("E17", "Tracking data or skin weights cannot be read or parsed (missing file, unknown format, binary FBX, bad rows, unknown bone)."),
    ("E18", "The composition expands into too many nodes: a repeat's @count, an object3D's @instances, or repeats and instances nested in each other."),
    ("E19", "An attribute the schema accepts has no effect, and no inert rule of SREPs 18 and 34 names the case: a link's or key's parameter its follow mode or curve does not read (information; the document renders as if it were absent). The cases the SREPs name are INERT-I1 to INERT-I13."),
    ("E21", "A key takes cubic-bezier from the animation's defaultInterpolation and gives no handles (warning): the engine's default handles (influence 1/3, speed 1) are used."),
    ("E22", "A feature of an accepted SREP that this engine does not implement yet is used (warning, --strict fails): the document is valid and the feature is ignored. The SREP number is in the message."),
    ("E23", "project/@precision=\"f32\" in a document with 3D content: the 2D working textures are 32-bit float, the 3D renderer's buffers stay 16-bit float (information, SREP 72)."),
    ("E16", "A {{placeholder}} names no parameter or repeat variable (warning; the text is left unchanged)."),
];
