//! Build-time code generator for the scene-render 1.1 schema.
//!
//! Reads `schema/scene-render-1.1.xsd` and writes two files into `OUT_DIR`:
//!
//! * `schema_tables.rs` — the schema as static data (simple types with their
//!   facets, complex types with their attribute declarations and content
//!   particles). The runtime structural validator interprets these tables.
//! * `model.rs` — the typed document model: one enum per enumerated simple
//!   type, one checked newtype per named numeric restriction, one struct per
//!   complex type, one enum per model group, and the `from_xml` constructors.
//!
//! The generator supports exactly the XSD 1.0 subset the schema uses:
//! named and anonymous simple types (restriction with enumeration, bounds,
//! pattern and maxLength; union; list), complex types with sequence, choice
//! and group references, attribute groups (nested), complexContent extension
//! and simpleContent extension. Any other construct aborts the build, so a
//! schema change that needs new generator support cannot be silently ignored.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::fmt::Write as _;
use std::path::PathBuf;

const XS: &str = "http://www.w3.org/2001/XMLSchema";
const UNBOUNDED: u32 = u32::MAX;

const BUILTINS: &[(&str, &str)] = &[
    ("string", "String"),
    ("double", "Double"),
    ("boolean", "Boolean"),
    ("integer", "Integer"),
    ("int", "Int"),
    ("nonNegativeInteger", "NonNegativeInteger"),
    ("positiveInteger", "PositiveInteger"),
    ("unsignedLong", "UnsignedLong"),
    ("ID", "Id"),
    ("IDREF", "IdRef"),
    ("IDREFS", "IdRefs"),
    ("NCName", "NcName"),
    ("NMTOKEN", "NmToken"),
    ("NMTOKENS", "NmTokens"),
    ("anyURI", "AnyUri"),
    ("dateTime", "DateTime"),
];

/// Named simple types whose Rust representation is hand-written in `values.rs`.
const HANDWRITTEN: &[(&str, &str)] = &[
    ("volumeSourceType", "String"),
    ("fpsType", "crate::values::Fps"),
    ("aspectType", "crate::values::Aspect"),
    ("sha256Type", "crate::values::Sha256"),
    ("timecodeType", "crate::values::Timecode"),
    ("languageTagType", "crate::values::LanguageTag"),
    ("numberListType", "Vec<f64>"),
    ("colorListType", "Vec<crate::values::Color>"),
    ("audioRoleListType", "Vec<String>"),
    ("pointType", "crate::values::Point2"),
    ("pointListType", "Vec<crate::values::Point2>"),
    ("relativeLength", "crate::values::Length"),
    ("positiveRelativeLength", "crate::values::Length"),
    ("lengthType", "crate::values::Length"),
    ("positiveLengthType", "crate::values::Length"),
    ("colorType", "crate::values::Color"),
    ("paintRefType", "crate::values::PaintRef"),
    ("paintType", "crate::values::Paint"),
    ("expressionString", "String"),
];

/// Attribute names too generic to name an anonymous enumeration on their own.
const GENERIC_ATTRS: &[&str] = &[
    "type",
    "kind",
    "mode",
    "shape",
    "preset",
    "format",
    "style",
    "layout",
    "direction",
    "position",
    "rotation",
    "version",
    "bitDepth",
    "interpolationSpace",
    "space",
    "unit",
    "units",
    "order",
    "channel",
    "audio",
    "pin",
    "role",
    "normalize",
    "quality",
    "selector",
    "spread",
    "bounds",
    "enforce",
    "affects",
    "combine",
    "composite",
    "falloff",
    "projection",
    "emoji",
    "wrap",
    "overflow",
    "primitive",
    "align",
    "alignment",
];

const RUST_KEYWORDS: &[&str] = &[
    "as", "break", "const", "continue", "crate", "else", "enum", "extern", "false", "fn", "for", "if", "impl", "in",
    "let", "loop", "match", "mod", "move", "mut", "pub", "ref", "return", "self", "Self", "static", "struct", "super",
    "trait", "true", "type", "unsafe", "use", "where", "while", "async", "await", "dyn", "abstract", "become", "box",
    "do", "final", "macro", "override", "priv", "typeof", "unsized", "virtual", "yield", "try", "gen",
];

// ------------------------------------------------------------------ IR

#[derive(Clone, Debug)]
enum SKind {
    Builtin(&'static str),
    Restr {
        base: usize,
        enums: Vec<String>,
        min_incl: Option<f64>,
        max_incl: Option<f64>,
        min_excl: Option<f64>,
        max_excl: Option<f64>,
        pattern: Option<String>,
        max_len: Option<usize>,
    },
    Union(Vec<usize>),
    List(usize),
}

#[derive(Clone, Debug)]
struct SDef {
    /// XSD name (`xs:double`, `positiveDecimal`) or a synthetic `owner@attr`.
    name: String,
    /// `(owner, attribute)` for anonymous types.
    anon: Option<(String, String)>,
    kind: SKind,
}

#[derive(Clone, Debug)]
struct Attr {
    name: String,
    ty: usize,
    required: bool,
    default: Option<String>,
}

#[derive(Clone, Debug)]
enum Particle {
    Element { name: String, ty: String, min: u32, max: u32 },
    Seq { items: Vec<Particle>, min: u32, max: u32 },
    Choice { items: Vec<Particle>, min: u32, max: u32 },
    Group { name: String, min: u32, max: u32 },
}

#[derive(Clone, Debug)]
enum Content {
    Empty,
    Simple(usize),
    Elements(Particle),
}

#[derive(Clone, Debug)]
struct CDef {
    name: String,
    attrs: Vec<Attr>,
    content: Content,
    /// complexContent extension base, used to share child enums.
    base: Option<String>,
}

struct Schema {
    simple: Vec<SDef>,
    simple_by_name: HashMap<String, usize>,
    complex: Vec<CDef>,
    complex_by_name: HashMap<String, usize>,
    groups: BTreeMap<String, Particle>,
    group_order: Vec<String>,
}

// ------------------------------------------------------------------ parsing

type N<'a, 'i> = roxmltree::Node<'a, 'i>;

fn xs_children<'a, 'i>(n: N<'a, 'i>) -> impl Iterator<Item = N<'a, 'i>> {
    n.children().filter(|c| c.is_element() && c.tag_name().namespace() == Some(XS))
}

fn local<'a>(n: N<'a, '_>) -> &'a str {
    n.tag_name().name()
}

fn occurs(n: N) -> (u32, u32) {
    let min = n.attribute("minOccurs").map(|v| v.parse().unwrap()).unwrap_or(1);
    let max = match n.attribute("maxOccurs") {
        Some("unbounded") => UNBOUNDED,
        Some(v) => v.parse().unwrap(),
        None => 1,
    };
    (min, max)
}

struct Parser<'a, 'i> {
    named_simple: HashMap<String, N<'a, 'i>>,
    attr_groups: HashMap<String, N<'a, 'i>>,
    s: Schema,
}

impl<'a, 'i> Parser<'a, 'i> {
    fn builtin(&mut self, local_name: &str) -> usize {
        let key = format!("xs:{local_name}");
        if let Some(&i) = self.s.simple_by_name.get(&key) {
            return i;
        }
        let variant = BUILTINS
            .iter()
            .find(|(n, _)| *n == local_name)
            .unwrap_or_else(|| panic!("unsupported builtin xs:{local_name}"))
            .1;
        let i = self.s.simple.len();
        self.s.simple.push(SDef { name: key.clone(), anon: None, kind: SKind::Builtin(variant) });
        self.s.simple_by_name.insert(key, i);
        i
    }

    fn type_ref(&mut self, qname: &str) -> usize {
        if let Some(l) = qname.strip_prefix("xs:") {
            return self.builtin(l);
        }
        if let Some(&i) = self.s.simple_by_name.get(qname) {
            return i;
        }
        let node = *self.named_simple.get(qname).unwrap_or_else(|| panic!("unknown simple type {qname}"));
        self.simple_type(node, qname.to_string(), None)
    }

    fn simple_type(&mut self, n: N<'a, 'i>, name: String, anon: Option<(String, String)>) -> usize {
        let body = xs_children(n).find(|c| local(*c) != "annotation").expect("empty simpleType");
        let kind = match local(body) {
            "restriction" => {
                let base = self.type_ref(body.attribute("base").expect("restriction base"));
                let mut enums = Vec::new();
                let (mut min_incl, mut max_incl, mut min_excl, mut max_excl) = (None, None, None, None);
                let mut pattern = None;
                let mut max_len = None;
                for f in xs_children(body) {
                    let v = f.attribute("value").unwrap_or_default();
                    match local(f) {
                        "enumeration" => enums.push(v.to_string()),
                        "minInclusive" => min_incl = Some(v.parse().unwrap()),
                        "maxInclusive" => max_incl = Some(v.parse().unwrap()),
                        "minExclusive" => min_excl = Some(v.parse().unwrap()),
                        "maxExclusive" => max_excl = Some(v.parse().unwrap()),
                        "pattern" => {
                            assert!(pattern.is_none(), "multiple patterns in {name}");
                            pattern = Some(v.to_string())
                        }
                        "maxLength" => max_len = Some(v.parse().unwrap()),
                        "annotation" => {}
                        other => panic!("unsupported facet {other} in {name}"),
                    }
                }
                SKind::Restr { base, enums, min_incl, max_incl, min_excl, max_excl, pattern, max_len }
            }
            "union" => {
                let members = body
                    .attribute("memberTypes")
                    .expect("union memberTypes")
                    .split_whitespace()
                    .map(|m| self.type_ref(m))
                    .collect();
                SKind::Union(members)
            }
            "list" => SKind::List(self.type_ref(body.attribute("itemType").expect("list itemType"))),
            other => panic!("unsupported simpleType body {other}"),
        };
        let i = self.s.simple.len();
        self.s.simple.push(SDef { name: name.clone(), anon, kind });
        self.s.simple_by_name.insert(name, i);
        i
    }

    fn attribute(&mut self, n: N<'a, 'i>, owner: &str) -> Attr {
        let name = n.attribute("name").expect("attribute name").to_string();
        assert!(n.attribute("ref").is_none(), "attribute refs are unsupported");
        let ty = if let Some(t) = n.attribute("type") {
            self.type_ref(t)
        } else if let Some(st) = xs_children(n).find(|c| local(*c) == "simpleType") {
            self.simple_type(st, format!("{owner}@{name}"), Some((owner.to_string(), name.clone())))
        } else {
            panic!("attribute {owner}@{name} has no type")
        };
        Attr {
            name,
            ty,
            required: n.attribute("use") == Some("required"),
            default: n.attribute("default").map(str::to_string),
        }
    }

    fn attrs_of(&mut self, n: N<'a, 'i>, owner: &str, out: &mut Vec<Attr>) {
        for c in xs_children(n) {
            match local(c) {
                "attribute" => {
                    let a = self.attribute(c, owner);
                    out.push(a);
                }
                "attributeGroup" => {
                    let r = c.attribute("ref").expect("attributeGroup ref");
                    let g = *self.attr_groups.get(r).unwrap_or_else(|| panic!("unknown attributeGroup {r}"));
                    self.attrs_of(g, r, out);
                }
                _ => {}
            }
        }
    }

    fn particle(&self, n: N) -> Option<Particle> {
        let (min, max) = occurs(n);
        match local(n) {
            "element" => Some(Particle::Element {
                name: n.attribute("name").expect("local element name").to_string(),
                ty: n.attribute("type").expect("local element type").to_string(),
                min,
                max,
            }),
            "sequence" | "choice" => {
                let items = xs_children(n).filter_map(|c| self.particle(c)).collect();
                Some(if local(n) == "sequence" {
                    Particle::Seq { items, min, max }
                } else {
                    Particle::Choice { items, min, max }
                })
            }
            "group" => Some(Particle::Group { name: n.attribute("ref").expect("group ref").to_string(), min, max }),
            "annotation" | "attribute" | "attributeGroup" => None,
            other => panic!("unsupported particle {other}"),
        }
    }

    fn complex_type(&mut self, n: N<'a, 'i>, name: String) -> CDef {
        let mut attrs = Vec::new();
        let mut content = Content::Empty;
        let mut base = None;
        for c in xs_children(n) {
            match local(c) {
                "sequence" | "choice" | "group" => content = Content::Elements(self.particle(c).unwrap()),
                "complexContent" => {
                    let ext = xs_children(c).find(|e| local(*e) == "extension").expect("complexContent/extension");
                    base = Some(ext.attribute("base").unwrap().to_string());
                    for p in xs_children(ext) {
                        if matches!(local(p), "sequence" | "choice" | "group") {
                            panic!("complexContent extension with added particles is unsupported ({name})");
                        }
                    }
                    self.attrs_of(ext, &name, &mut attrs);
                }
                "simpleContent" => {
                    let ext = xs_children(c).find(|e| local(*e) == "extension").expect("simpleContent/extension");
                    content = Content::Simple(self.type_ref(ext.attribute("base").unwrap()));
                    self.attrs_of(ext, &name, &mut attrs);
                }
                _ => {}
            }
        }
        self.attrs_of(n, &name, &mut attrs);
        if n.attribute("mixed") == Some("true") {
            panic!("mixed content is unsupported ({name})");
        }
        CDef { name, attrs, content, base }
    }
}

fn parse_schema(text: &str) -> Schema {
    let doc = roxmltree::Document::parse(text).expect("XSD is not well-formed");
    let root = doc.root_element();
    let mut p = Parser {
        named_simple: HashMap::new(),
        attr_groups: HashMap::new(),
        s: Schema {
            simple: Vec::new(),
            simple_by_name: HashMap::new(),
            complex: Vec::new(),
            complex_by_name: HashMap::new(),
            groups: BTreeMap::new(),
            group_order: Vec::new(),
        },
    };
    for c in xs_children(root) {
        match local(c) {
            "simpleType" => {
                p.named_simple.insert(c.attribute("name").unwrap().to_string(), c);
            }
            "attributeGroup" => {
                p.attr_groups.insert(c.attribute("name").unwrap().to_string(), c);
            }
            _ => {}
        }
    }
    // Named simple types in declaration order, so table indices are stable.
    for c in xs_children(root).filter(|c| local(*c) == "simpleType") {
        let name = c.attribute("name").unwrap();
        p.type_ref(name);
    }
    let mut pending_complex = Vec::new();
    for c in xs_children(root) {
        match local(c) {
            "complexType" => pending_complex.push((c.attribute("name").unwrap().to_string(), c)),
            "group" => {
                let name = c.attribute("name").unwrap().to_string();
                let body = xs_children(c).find(|x| local(*x) != "annotation").unwrap();
                let part = p.particle(body).unwrap();
                p.s.group_order.push(name.clone());
                p.s.groups.insert(name, part);
            }
            "element" => {
                assert_eq!(c.attribute("name"), Some("scene"), "only the scene root element is supported");
                let ct = xs_children(c).find(|x| local(*x) == "complexType").unwrap();
                pending_complex.push(("scene".to_string(), ct));
            }
            "simpleType" | "attributeGroup" | "annotation" => {}
            other => panic!("unsupported top-level {other}"),
        }
    }
    for (name, node) in pending_complex {
        let def = p.complex_type(node, name.clone());
        p.s.complex_by_name.insert(name, p.s.complex.len());
        p.s.complex.push(def);
    }
    // Resolve complexContent extension: prepend base attributes, inherit content.
    for i in 0..p.s.complex.len() {
        if let Some(b) = p.s.complex[i].base.clone() {
            let bi = p.s.complex_by_name[&b];
            assert!(p.s.complex[bi].base.is_none(), "multi-level extension unsupported");
            let mut attrs = p.s.complex[bi].attrs.clone();
            attrs.append(&mut p.s.complex[i].attrs);
            p.s.complex[i].attrs = attrs;
            p.s.complex[i].content = p.s.complex[bi].content.clone();
        }
    }
    for c in &p.s.complex {
        let mut seen = HashSet::new();
        for a in &c.attrs {
            assert!(seen.insert(a.name.clone()), "duplicate attribute {}@{}", c.name, a.name);
        }
    }
    p.s
}

// ------------------------------------------------------------------ naming

fn upper_first(s: &str) -> String {
    let mut c = s.chars();
    match c.next() {
        Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
        None => String::new(),
    }
}

fn strip_type(s: &str) -> &str {
    s.strip_suffix("Type").unwrap_or(s)
}

fn pascal_from_xsd(s: &str) -> String {
    upper_first(strip_type(s))
}

fn snake(s: &str) -> String {
    let chars: Vec<char> = s.chars().collect();
    let mut out = String::new();
    for (i, &ch) in chars.iter().enumerate() {
        if ch.is_ascii_uppercase() {
            let prev = if i > 0 { Some(chars[i - 1]) } else { None };
            let next = chars.get(i + 1).copied();
            let boundary = match prev {
                Some(p) if p.is_ascii_lowercase() || p.is_ascii_digit() => true,
                Some(p) if p.is_ascii_uppercase() => next.is_some_and(|n| n.is_ascii_lowercase()),
                _ => false,
            };
            if boundary {
                out.push('_');
            }
            out.push(ch.to_ascii_lowercase());
        } else {
            out.push(ch);
        }
    }
    out
}

fn ident(s: &str) -> String {
    if RUST_KEYWORDS.contains(&s) {
        assert!(!matches!(s, "self" | "Self" | "super" | "crate"), "cannot escape {s}");
        format!("r#{s}")
    } else {
        s.to_string()
    }
}

fn plural(s: &str) -> String {
    if s.ends_with("_data") || s == "data" {
        s.to_string()
    } else if s.ends_with('s') {
        format!("{s}es")
    } else {
        format!("{s}s")
    }
}

/// Enumeration literal to a Rust variant name.
fn variant_name(v: &str) -> String {
    let mut out = String::new();
    for seg in v.split(['-', '.', '_', ' ']) {
        if seg.is_empty() {
            continue;
        }
        let starts_digit = seg.chars().next().unwrap().is_ascii_digit();
        if starts_digit {
            match out.chars().last() {
                None => out.push('V'),
                Some(c) if c.is_ascii_digit() => out.push('_'),
                Some(_) => {}
            }
            out.push_str(seg);
        } else {
            out.push_str(&upper_first(seg));
        }
    }
    assert!(!out.is_empty(), "empty variant for {v:?}");
    out
}

/// Field name for a repeated child element: the plural of its name, or
/// `<name>_list` when an attribute already owns the plural.
fn child_field_name(c: &CDef, el: &str) -> String {
    let p = plural(&snake(el));
    if c.attrs.iter().any(|a| snake(&a.name) == p) {
        format!("{}_list", snake(el))
    } else {
        p
    }
}

fn group_enum_name(g: &str) -> String {
    match g {
        "animationElements" => "Animation".into(),
        "nodeBehaviour" => "NodeBehaviour".into(),
        "bodyBehaviour" => "BodyBehaviour".into(),
        "nodeChoice" => "Node".into(),
        other => upper_first(other),
    }
}

// ------------------------------------------------------------------ type mapping

struct Names {
    /// simple type index -> Rust type expression used in model fields
    simple_rust: Vec<String>,
    /// generated enums: name -> (simple index, values, numeric base)
    enums: BTreeMap<String, (usize, Vec<String>, bool)>,
    /// generated numeric newtypes: name -> (simple index, inner Rust type)
    newtypes: BTreeMap<String, (usize, String)>,
    complex_rust: Vec<String>,
}

fn root_builtin(s: &Schema, mut i: usize) -> &'static str {
    loop {
        match &s.simple[i].kind {
            SKind::Builtin(b) => return b,
            SKind::Restr { base, .. } => i = *base,
            SKind::Union(_) => return "Union",
            SKind::List(_) => return "List",
        }
    }
}

fn builtin_rust(b: &str) -> &'static str {
    match b {
        "String" | "AnyUri" | "DateTime" | "Id" | "IdRef" | "NcName" | "NmToken" => "String",
        "IdRefs" | "NmTokens" => "Vec<String>",
        "Double" => "f64",
        "Boolean" => "bool",
        "Integer" => "i64",
        "Int" => "i32",
        "NonNegativeInteger" | "PositiveInteger" | "UnsignedLong" => "u64",
        other => panic!("no Rust mapping for builtin {other}"),
    }
}

fn is_numeric_builtin(b: &str) -> bool {
    matches!(b, "Double" | "Integer" | "Int" | "NonNegativeInteger" | "PositiveInteger" | "UnsignedLong")
}

fn assign_names(s: &Schema) -> Names {
    let mut taken: HashSet<String> = HashSet::new();
    for h in [
        "Fps",
        "Aspect",
        "Sha256",
        "Timecode",
        "LanguageTag",
        "Point2",
        "Length",
        "Color",
        "Paint",
        "PaintRef",
        "Rgba",
        "Loc",
        "ParseValue",
        "ValueError",
        "ModelError",
    ] {
        taken.insert(h.to_string());
    }
    let complex_rust: Vec<String> = s
        .complex
        .iter()
        .map(|c| if c.name == "scene" { "Scene".to_string() } else { pascal_from_xsd(&c.name) })
        .collect();
    for n in &complex_rust {
        assert!(taken.insert(n.clone()), "name clash on struct {n}");
    }
    for g in &s.group_order {
        let n = group_enum_name(g);
        assert!(taken.insert(n.clone()), "name clash on group enum {n}");
    }
    for c in &complex_rust {
        let child = format!("{c}Child");
        taken.insert(child);
    }

    let mut simple_rust = vec![String::new(); s.simple.len()];
    let mut enums = BTreeMap::new();
    let mut newtypes = BTreeMap::new();

    // anonymous enumerations: dedupe by (attribute, values)
    let mut anon_sets: BTreeMap<String, BTreeSet<Vec<String>>> = BTreeMap::new();
    for d in &s.simple {
        if let (Some((_, attr)), SKind::Restr { enums: e, .. }) = (&d.anon, &d.kind) {
            if !e.is_empty() {
                anon_sets.entry(attr.clone()).or_default().insert(e.clone());
            }
        }
    }
    let mut anon_named: HashMap<(String, Vec<String>), String> = HashMap::new();

    for (i, d) in s.simple.iter().enumerate() {
        let rust = match (&d.kind, &d.anon) {
            (SKind::Builtin(b), _) => builtin_rust(b).to_string(),
            (_, None) if HANDWRITTEN.iter().any(|(n, _)| *n == d.name) => {
                HANDWRITTEN.iter().find(|(n, _)| *n == d.name).unwrap().1.to_string()
            }
            (SKind::Restr { enums: e, .. }, anon) if !e.is_empty() => {
                let numeric = is_numeric_builtin(root_builtin(s, i));
                let name = match anon {
                    None => pascal_from_xsd(&d.name),
                    Some((owner, attr)) => {
                        let key = (attr.clone(), e.clone());
                        if let Some(n) = anon_named.get(&key) {
                            simple_rust[i] = n.clone();
                            continue;
                        }
                        let unique = anon_sets[attr].len() == 1 && !GENERIC_ATTRS.contains(&attr.as_str());
                        let mut n = if unique {
                            upper_first(attr)
                        } else {
                            let o = if owner == "scene" { "Scene".to_string() } else { pascal_from_xsd(owner) };
                            let a =
                                if attr == "type" || attr == "kind" { "Kind".to_string() } else { upper_first(attr) };
                            format!("{o}{a}")
                        };
                        if pascal_from_xsd(owner).eq_ignore_ascii_case(attr) {
                            n = format!("{}Form", pascal_from_xsd(owner));
                        }
                        if taken.contains(&n) {
                            n = format!("{}{}", pascal_from_xsd(owner), upper_first(attr));
                        }
                        anon_named.insert(key, n.clone());
                        n
                    }
                };
                assert!(taken.insert(name.clone()), "name clash on enum {name} ({})", d.name);
                enums.insert(name.clone(), (i, e.clone(), numeric));
                name
            }
            (SKind::Restr { base, pattern, .. }, None) if pattern.is_none() => {
                let inner = builtin_rust(root_builtin(s, *base)).to_string();
                if inner == "String" {
                    inner
                } else {
                    let name = pascal_from_xsd(&d.name);
                    assert!(taken.insert(name.clone()), "name clash on newtype {name}");
                    newtypes.insert(name.clone(), (i, inner));
                    name
                }
            }
            (SKind::Restr { base, .. }, _) => {
                // anonymous bounded numbers and pattern-restricted strings keep their base type;
                // the structural validator enforces the facets.
                builtin_rust(root_builtin(s, *base)).to_string()
            }
            (SKind::Union(_), _) | (SKind::List(_), _) => {
                panic!("simple type {} needs a hand-written Rust mapping", d.name)
            }
        };
        simple_rust[i] = rust;
    }
    Names { simple_rust, enums, newtypes, complex_rust }
}

// ------------------------------------------------------------------ tables emission

fn expand_particle(s: &Schema, p: &Particle) -> Particle {
    match p {
        Particle::Element { .. } => p.clone(),
        Particle::Seq { items, min, max } => {
            Particle::Seq { items: items.iter().map(|x| expand_particle(s, x)).collect(), min: *min, max: *max }
        }
        Particle::Choice { items, min, max } => {
            Particle::Choice { items: items.iter().map(|x| expand_particle(s, x)).collect(), min: *min, max: *max }
        }
        Particle::Group { name, min, max } => {
            let inner = expand_particle(s, &s.groups[name]);
            if (*min, *max) == (1, 1) {
                return inner;
            }
            match inner {
                Particle::Choice { items, min: 1, max: 1 } => Particle::Choice { items, min: *min, max: *max },
                Particle::Seq { items, min: 1, max: 1 } => Particle::Seq { items, min: *min, max: *max },
                other => Particle::Seq { items: vec![other], min: *min, max: *max },
            }
        }
    }
}

fn fmt_opt_f64(v: Option<f64>) -> String {
    match v {
        Some(x) => format!("Some({x:?})"),
        None => "None".into(),
    }
}

fn emit_particle(s: &Schema, p: &Particle, out: &mut String) {
    let m = |x: u32| if x == UNBOUNDED { "UNBOUNDED".to_string() } else { x.to_string() };
    match p {
        Particle::Element { name, ty, min, max } => {
            let ti = s.complex_by_name[ty];
            write!(out, "Particle::Element {{ name: {name:?}, ty: {ti}, min: {}, max: {} }}", m(*min), m(*max))
                .unwrap();
        }
        Particle::Seq { items, min, max } | Particle::Choice { items, min, max } => {
            let kind = if matches!(p, Particle::Seq { .. }) { "Seq" } else { "Choice" };
            write!(out, "Particle::{kind} {{ items: &[").unwrap();
            for it in items {
                emit_particle(s, it, out);
                out.push_str(", ");
            }
            write!(out, "], min: {}, max: {} }}", m(*min), m(*max)).unwrap();
        }
        Particle::Group { .. } => unreachable!("groups are expanded"),
    }
}

fn id_kind(s: &Schema, i: usize) -> &'static str {
    match root_builtin(s, i) {
        "Id" => "IdKind::Id",
        "IdRef" => "IdKind::IdRef",
        "IdRefs" => "IdKind::IdRefs",
        _ => "IdKind::None",
    }
}

fn emit_tables(s: &Schema) -> String {
    let mut o = String::new();
    o.push_str("// @generated by build.rs from schema/scene-render-1.1.xsd. Do not edit.\n\n");
    o.push_str("/// Every simple type of the schema; builtins first as they are referenced.\npub static SIMPLE_TYPES: &[SimpleType] = &[\n");
    for d in &s.simple {
        let kind = match &d.kind {
            SKind::Builtin(b) => format!("SimpleKind::Builtin(Builtin::{b})"),
            SKind::Restr { base, enums, min_incl, max_incl, min_excl, max_excl, pattern, max_len } => format!(
                "SimpleKind::Restriction(Restriction {{ base: {base}, enums: &{enums:?}, min_inclusive: {}, max_inclusive: {}, min_exclusive: {}, max_exclusive: {}, pattern: {}, max_length: {} }})",
                fmt_opt_f64(*min_incl),
                fmt_opt_f64(*max_incl),
                fmt_opt_f64(*min_excl),
                fmt_opt_f64(*max_excl),
                match pattern {
                    Some(p) => format!("Some({p:?})"),
                    None => "None".into(),
                },
                match max_len {
                    Some(l) => format!("Some({l})"),
                    None => "None".into(),
                },
            ),
            SKind::Union(m) => format!("SimpleKind::Union(&{m:?})"),
            SKind::List(it) => format!("SimpleKind::List({it})"),
        };
        writeln!(o, "    SimpleType {{ name: {:?}, kind: {kind} }},", d.name).unwrap();
    }
    o.push_str("];\n\n");

    o.push_str("/// Every complex type of the schema, extensions applied and groups expanded.\npub static COMPLEX_TYPES: &[ComplexType] = &[\n");
    for c in &s.complex {
        write!(o, "    ComplexType {{ name: {:?}, attrs: &[", c.name).unwrap();
        for a in &c.attrs {
            write!(
                o,
                "AttrDecl {{ name: {:?}, ty: {}, required: {}, default: {}, id: {} }}, ",
                a.name,
                a.ty,
                a.required,
                match &a.default {
                    Some(d) => format!("Some({d:?})"),
                    None => "None".into(),
                },
                id_kind(s, a.ty)
            )
            .unwrap();
        }
        o.push_str("], content: ");
        match &c.content {
            Content::Empty => o.push_str("Content::Empty"),
            Content::Simple(t) => write!(o, "Content::Simple({t})").unwrap(),
            Content::Elements(p) => {
                o.push_str("Content::Elements(&");
                emit_particle(s, &expand_particle(s, p), &mut o);
                o.push(')');
            }
        }
        o.push_str(" },\n");
    }
    o.push_str("];\n\n");
    writeln!(o, "/// Index of the anonymous complex type of the `scene` root element.").unwrap();
    writeln!(o, "pub const ROOT_TYPE: usize = {};", s.complex_by_name["scene"]).unwrap();
    o
}

// ------------------------------------------------------------------ model emission

struct Model<'s> {
    s: &'s Schema,
    n: &'s Names,
    out: String,
}

enum Shape {
    Empty,
    Simple,
    /// (element name, element type, min, max) in sequence order
    Fields(Vec<(String, String, u32, u32)>),
    /// Vec<group enum>
    GroupVec(String),
    /// Vec<child enum>; the enum is owned by `owner` (the base type for extensions)
    ChildVec(String),
}

impl<'s> Model<'s> {
    fn shape(&self, ci: usize) -> Shape {
        let c = &self.s.complex[ci];
        match &c.content {
            Content::Empty => Shape::Empty,
            Content::Simple(_) => Shape::Simple,
            Content::Elements(p) => match p {
                Particle::Seq { items, min: 1, max: 1 }
                    if items.iter().all(|i| matches!(i, Particle::Element { .. })) =>
                {
                    Shape::Fields(
                        items
                            .iter()
                            .map(|i| match i {
                                Particle::Element { name, ty, min, max } => (name.clone(), ty.clone(), *min, *max),
                                _ => unreachable!(),
                            })
                            .collect(),
                    )
                }
                Particle::Group { name, .. } => Shape::GroupVec(group_enum_name(name)),
                Particle::Choice { .. } => {
                    let owner = match &c.base {
                        Some(b) => self.n.complex_rust[self.s.complex_by_name[b]].clone(),
                        None => self.n.complex_rust[ci].clone(),
                    };
                    Shape::ChildVec(format!("{owner}Child"))
                }
                other => panic!("unsupported content shape in {}: {other:?}", c.name),
            },
        }
    }

    fn struct_of(&self, xsd_type: &str) -> &str {
        &self.n.complex_rust[self.s.complex_by_name[xsd_type]]
    }

    /// Flattened alternatives of a choice: element alternatives and group alternatives.
    fn choice_alts(&self, p: &Particle) -> (Vec<(String, String)>, Vec<String>) {
        let mut els = Vec::new();
        let mut groups = Vec::new();
        let items = match p {
            Particle::Choice { items, .. } => items,
            other => panic!("expected choice, got {other:?}"),
        };
        for it in items {
            match it {
                Particle::Element { name, ty, .. } => els.push((name.clone(), self.struct_of(ty).to_string())),
                Particle::Group { name, .. } => groups.push(name.clone()),
                other => panic!("unsupported choice alternative {other:?}"),
            }
        }
        (els, groups)
    }

    fn emit_choice_enum(&mut self, enum_name: &str, doc: &str, p: &Particle) {
        let (els, groups) = self.choice_alts(p);
        let mut o = String::new();
        writeln!(o, "/// {doc}").unwrap();
        o.push_str("#[derive(Debug, Clone, PartialEq, serde::Serialize)]\n#[allow(clippy::large_enum_variant)]\n");
        writeln!(o, "pub enum {enum_name} {{").unwrap();
        for (el, st) in &els {
            writeln!(o, "    #[serde(rename = {el:?})]\n    {}({st}),", variant_name(el)).unwrap();
        }
        for g in &groups {
            let ge = group_enum_name(g);
            writeln!(o, "    #[serde(untagged)]\n    {ge}({ge}),").unwrap();
        }
        o.push_str("}\n\n");
        writeln!(o, "impl {enum_name} {{").unwrap();
        writeln!(o, "    /// XML element names this enum accepts.").unwrap();
        o.push_str("    pub const ELEMENTS: &'static [&'static str] = &[");
        for (el, _) in &els {
            write!(o, "{el:?}, ").unwrap();
        }
        o.push_str("];\n\n");
        o.push_str("    pub(crate) fn from_xml(n: XNode<'_, '_>) -> Result<Option<Self>, ModelError> {\n");
        if !els.is_empty() {
            o.push_str("        match n.tag_name().name() {\n");
            for (el, st) in &els {
                writeln!(o, "            {el:?} => return Ok(Some(Self::{}({st}::from_xml(n)?))),", variant_name(el))
                    .unwrap();
            }
            o.push_str("            _ => {}\n        }\n");
        }
        for g in &groups {
            let ge = group_enum_name(g);
            writeln!(o, "        if let Some(v) = {ge}::from_xml(n)? {{\n            return Ok(Some(Self::{ge}(v)));\n        }}").unwrap();
        }
        o.push_str("        Ok(None)\n    }\n\n");
        o.push_str("    /// The XML element name of this child.\n    pub fn element_name(&self) -> &'static str {\n        match self {\n");
        for (el, _) in &els {
            writeln!(o, "            Self::{}(_) => {el:?},", variant_name(el)).unwrap();
        }
        for g in &groups {
            let ge = group_enum_name(g);
            writeln!(o, "            Self::{ge}(v) => v.element_name(),").unwrap();
        }
        o.push_str("        }\n    }\n\n");
        o.push_str("    /// Value of `@id` of the wrapped element, if any.\n    pub fn id(&self) -> Option<&str> {\n        match self {\n");
        for (el, _) in &els {
            writeln!(o, "            Self::{}(v) => v.id(),", variant_name(el)).unwrap();
        }
        for g in &groups {
            let ge = group_enum_name(g);
            writeln!(o, "            Self::{ge}(v) => v.id(),").unwrap();
        }
        o.push_str("        }\n    }\n}\n\n");
        let arms = |call: &str| -> String {
            let mut a = String::new();
            for (el, _) in &els {
                writeln!(a, "            Self::{}(v) => v.{call},", variant_name(el)).unwrap();
            }
            for g in &groups {
                writeln!(a, "            Self::{}(v) => v.{call},", group_enum_name(g)).unwrap();
            }
            a
        };
        writeln!(o, "impl crate::element::Element for {enum_name} {{").unwrap();
        o.push_str("    fn element_name(&self) -> &'static str {\n        self.element_name()\n    }\n");
        for (sig, call) in [
            ("fn xsd_type(&self) -> usize", "xsd_type()"),
            ("fn loc(&self) -> Loc", "loc()"),
            ("fn get_attr(&self, name: &str) -> Option<crate::element::AttrValue>", "get_attr(name)"),
            (
                "fn set_attr(&mut self, name: &str, raw: &str) -> Result<(), crate::element::SetAttrError>",
                "set_attr(name, raw)",
            ),
            ("fn text(&self) -> Option<&str>", "text()"),
            ("fn set_text(&mut self, text: String)", "set_text(text)"),
            ("fn visit<'s>(&'s self, f: &mut dyn FnMut(&'s dyn crate::element::Element))", "visit(f)"),
            ("fn visit_mut(&mut self, f: &mut dyn FnMut(&mut dyn crate::element::Element))", "visit_mut(f)"),
            ("fn as_any(&self) -> &dyn std::any::Any", "as_any()"),
        ] {
            writeln!(o, "    {sig} {{\n        match self {{\n{}        }}\n    }}", arms(call)).unwrap();
        }
        o.push_str("    fn element_id(&self) -> Option<&str> {\n        self.id()\n    }\n}\n\n");
        self.out.push_str(&o);
    }

    fn element_impl(&self, ci: usize, shape: &Shape) -> String {
        let c = &self.s.complex[ci];
        let name = &self.n.complex_rust[ci];
        let mut o = String::new();
        writeln!(o, "impl crate::element::Element for {name} {{").unwrap();
        writeln!(o, "    fn element_name(&self) -> &'static str {{\n        {:?}\n    }}", c.name).unwrap();
        writeln!(o, "    fn xsd_type(&self) -> usize {{\n        {ci}\n    }}").unwrap();
        o.push_str("    fn loc(&self) -> Loc {\n        self.loc\n    }\n");
        o.push_str("    fn element_id(&self) -> Option<&str> {\n        self.id()\n    }\n");
        o.push_str("    fn get_attr(&self, name: &str) -> Option<crate::element::AttrValue> {\n        use crate::element::ToAttr;\n        match name {\n");
        for a in &c.attrs {
            let f = ident(&snake(&a.name));
            if a.required || a.default.is_some() {
                writeln!(o, "            {:?} => Some(self.{f}.to_attr()),", a.name).unwrap();
            } else {
                writeln!(o, "            {:?} => self.{f}.as_ref().map(|v| v.to_attr()),", a.name).unwrap();
            }
        }
        o.push_str("            _ => None,\n        }\n    }\n");
        o.push_str("    fn set_attr(&mut self, name: &str, raw: &str) -> Result<(), crate::element::SetAttrError> {\n        #[allow(unused_imports)]\n        use crate::element::parse_into;\n        match name {\n");
        for a in &c.attrs {
            let f = ident(&snake(&a.name));
            if a.required || a.default.is_some() {
                writeln!(o, "            {:?} => self.{f} = parse_into(name, raw)?,", a.name).unwrap();
            } else {
                writeln!(o, "            {:?} => self.{f} = Some(parse_into(name, raw)?),", a.name).unwrap();
            }
        }
        writeln!(o, "            _ => return Err(crate::element::SetAttrError::Unknown {{ element: {:?}, name: name.to_string() }}),", c.name).unwrap();
        o.push_str("        }\n        #[allow(unreachable_code)]\n        Ok(())\n    }\n");
        if matches!(shape, Shape::Simple) {
            o.push_str("    fn text(&self) -> Option<&str> {\n        Some(&self.value)\n    }\n");
            o.push_str("    fn set_text(&mut self, text: String) {\n        self.value = text;\n    }\n");
        }
        for (m, r, amp) in
            [("visit", "&dyn crate::element::Element", ""), ("visit_mut", "&mut dyn crate::element::Element", "mut ")]
        {
            let self_ref = if m == "visit" { "&self" } else { "&mut self" };
            let iter = if m == "visit" { "iter" } else { "iter_mut" };
            let asref = if m == "visit" { "as_ref" } else { "as_mut" };
            let used = !matches!(shape, Shape::Empty | Shape::Simple);
            let fname = if used { "f" } else { "_f" };
            if m == "visit" {
                writeln!(o, "    fn visit<'s>(&'s self, {fname}: &mut dyn FnMut(&'s dyn crate::element::Element)) {{")
                    .unwrap();
            } else {
                writeln!(o, "    fn {m}({self_ref}, {fname}: &mut dyn FnMut({r})) {{").unwrap();
            }
            match shape {
                Shape::Fields(fs) => {
                    for (el, _ty, min, max) in fs {
                        let base = snake(el);
                        if *max > 1 {
                            writeln!(
                                o,
                                "        for c in self.{}.{iter}() {{\n            f(c);\n        }}",
                                ident(&child_field_name(c, el))
                            )
                            .unwrap();
                        } else if *min == 0 {
                            writeln!(
                                o,
                                "        if let Some(c) = self.{}.{asref}() {{\n            f(c);\n        }}",
                                ident(&base)
                            )
                            .unwrap();
                        } else {
                            writeln!(o, "        f(&{amp}self.{});", ident(&base)).unwrap();
                        }
                    }
                }
                Shape::GroupVec(_) | Shape::ChildVec(_) => {
                    writeln!(o, "        for c in self.children.{iter}() {{\n            f(c);\n        }}").unwrap();
                }
                _ => {}
            }
            o.push_str("    }\n");
        }
        o.push_str("    fn as_any(&self) -> &dyn std::any::Any {\n        self\n    }\n}\n\n");
        o
    }

    fn field_ty(&self, a: &Attr) -> String {
        let t = &self.n.simple_rust[a.ty];
        if a.required || a.default.is_some() {
            t.clone()
        } else {
            format!("Option<{t}>")
        }
    }

    fn emit_struct(&mut self, ci: usize) {
        let c = self.s.complex[ci].clone();
        let name = self.n.complex_rust[ci].clone();
        let shape = self.shape(ci);
        let mut fields_seen = HashSet::new();
        fields_seen.insert("loc".to_string());
        let mut o = String::new();
        writeln!(o, "/// `{}` (XSD complex type).", if c.name == "scene" { "<scene>" } else { &c.name }).unwrap();
        o.push_str("#[derive(Debug, Clone, PartialEq, serde::Serialize)]\n");
        writeln!(o, "pub struct {name} {{").unwrap();
        o.push_str("    /// Source position of the element.\n    #[serde(skip)]\n    pub loc: Loc,\n");
        for a in &c.attrs {
            let f = snake(&a.name);
            assert!(fields_seen.insert(f.clone()), "field clash {name}.{f}");
            let ty = self.field_ty(a);
            if let Some(d) = &a.default {
                writeln!(o, "    /// `@{}` (default `{}`).", a.name, d).unwrap();
            } else if a.required {
                writeln!(o, "    /// `@{}` (required).", a.name).unwrap();
            } else {
                writeln!(o, "    /// `@{}` (optional).", a.name).unwrap();
            }
            write!(o, "    #[serde(rename = {:?}", a.name).unwrap();
            if ty.starts_with("Option<") {
                o.push_str(", skip_serializing_if = \"Option::is_none\"");
            }
            o.push_str(")]\n");
            writeln!(o, "    pub {}: {ty},", ident(&f)).unwrap();
        }
        match &shape {
            Shape::Empty => {}
            Shape::Simple => {
                assert!(fields_seen.insert("value".into()), "field clash {name}.value");
                o.push_str("    /// Text content.\n    #[serde(rename = \"$text\")]\n    pub value: String,\n");
            }
            Shape::Fields(fs) => {
                for (el, ty, min, max) in fs {
                    let st = self.struct_of(ty).to_string();
                    let base = snake(el);
                    let (fname, fty) = if *max > 1 {
                        (child_field_name(&c, el), format!("Vec<{st}>"))
                    } else if *min == 0 {
                        (base, format!("Option<{st}>"))
                    } else {
                        (base, st)
                    };
                    assert!(fields_seen.insert(fname.clone()), "field clash {name}.{fname}");
                    writeln!(o, "    /// `<{el}>` child element(s).").unwrap();
                    write!(o, "    #[serde(rename = {el:?}").unwrap();
                    if fty.starts_with("Option<") {
                        o.push_str(", skip_serializing_if = \"Option::is_none\"");
                    } else if fty.starts_with("Vec<") {
                        o.push_str(", skip_serializing_if = \"Vec::is_empty\"");
                    }
                    o.push_str(")]\n");
                    writeln!(o, "    pub {}: {fty},", ident(&fname)).unwrap();
                }
            }
            Shape::GroupVec(e) | Shape::ChildVec(e) => {
                assert!(fields_seen.insert("children".into()), "field clash {name}.children");
                writeln!(o, "    /// Child elements in document order.").unwrap();
                o.push_str("    #[serde(skip_serializing_if = \"Vec::is_empty\")]\n");
                writeln!(o, "    pub children: Vec<{e}>,").unwrap();
            }
        }
        o.push_str("}\n\n");

        // constructor
        writeln!(o, "impl {name} {{").unwrap();
        writeln!(o, "    /// XSD attribute names of this element, in declaration order.").unwrap();
        o.push_str("    pub const ATTRIBUTES: &'static [&'static str] = &[");
        for a in &c.attrs {
            write!(o, "{:?}, ", a.name).unwrap();
        }
        o.push_str("];\n\n");
        o.push_str("    pub(crate) fn from_xml(n: XNode<'_, '_>) -> Result<Self, ModelError> {\n");
        match &shape {
            Shape::Fields(fs) => {
                for (el, ty, _min, max) in fs {
                    let st = self.struct_of(ty);
                    let v = format!("f_{}", snake(el));
                    if *max > 1 {
                        writeln!(o, "        let mut {v}: Vec<{st}> = Vec::new();").unwrap();
                    } else {
                        writeln!(o, "        let mut {v}: Option<{st}> = None;").unwrap();
                    }
                }
                o.push_str("        for c in n.children().filter(|c| c.is_element()) {\n            match c.tag_name().name() {\n");
                for (el, ty, _min, max) in fs {
                    let st = self.struct_of(ty);
                    let v = format!("f_{}", snake(el));
                    if *max > 1 {
                        writeln!(o, "                {el:?} => {v}.push({st}::from_xml(c)?),").unwrap();
                    } else {
                        writeln!(o, "                {el:?} => {v} = Some({st}::from_xml(c)?),").unwrap();
                    }
                }
                o.push_str("                _ => return Err(ModelError::unexpected(c)),\n            }\n        }\n");
            }
            Shape::GroupVec(e) | Shape::ChildVec(e) => {
                writeln!(o, "        let mut children: Vec<{e}> = Vec::new();").unwrap();
                o.push_str("        for c in n.children().filter(|c| c.is_element()) {\n");
                writeln!(o, "            match {e}::from_xml(c)? {{").unwrap();
                o.push_str("                Some(v) => children.push(v),\n                None => return Err(ModelError::unexpected(c)),\n            }\n        }\n");
            }
            Shape::Empty | Shape::Simple => {}
        }
        o.push_str("        Ok(Self {\n            loc: Loc::of(n),\n");
        for a in &c.attrs {
            let f = ident(&snake(&a.name));
            if a.required {
                writeln!(o, "            {f}: req(n, {:?})?,", a.name).unwrap();
            } else if let Some(d) = &a.default {
                writeln!(o, "            {f}: def!(n, {:?}, {d:?}, {})?,", a.name, self.n.simple_rust[a.ty]).unwrap();
            } else {
                writeln!(o, "            {f}: opt(n, {:?})?,", a.name).unwrap();
            }
        }
        match &shape {
            Shape::Simple => o.push_str("            value: text_content(n),\n"),
            Shape::Fields(fs) => {
                for (el, _ty, min, max) in fs {
                    let base = snake(el);
                    let v = format!("f_{base}");
                    if *max > 1 {
                        writeln!(o, "            {}: {v},", ident(&child_field_name(&c, el))).unwrap();
                    } else if *min == 0 {
                        writeln!(o, "            {}: {v},", ident(&base)).unwrap();
                    } else {
                        writeln!(
                            o,
                            "            {}: {v}.ok_or_else(|| ModelError::missing(n, {el:?}))?,",
                            ident(&base)
                        )
                        .unwrap();
                    }
                }
            }
            Shape::GroupVec(_) | Shape::ChildVec(_) => o.push_str("            children,\n"),
            Shape::Empty => {}
        }
        o.push_str("        })\n    }\n\n");
        o.push_str("    /// Value of `@id`, when the element declares and carries one.\n    pub fn id(&self) -> Option<&str> {\n");
        match c.attrs.iter().find(|a| a.name == "id") {
            Some(a) if a.required || a.default.is_some() => o.push_str("        Some(self.id.as_str())\n"),
            Some(_) => o.push_str("        self.id.as_deref()\n"),
            None => o.push_str("        None\n"),
        }
        o.push_str("    }\n}\n\n");
        o.push_str(&self.element_impl(ci, &shape));
        self.out.push_str(&o);

        if let (Shape::ChildVec(e), None, Content::Elements(p)) = (&shape, &c.base, &c.content) {
            let doc = format!("Child elements of `{}`.", c.name);
            let e = e.clone();
            let p = p.clone();
            self.emit_choice_enum(&e, &doc, &p);
        }
    }

    fn emit_enums(&mut self) {
        let enums = self.n.enums.clone();
        for (name, (si, values, numeric)) in &enums {
            let d = &self.s.simple[*si];
            let mut o = String::new();
            match &d.anon {
                Some((owner, attr)) => writeln!(o, "/// Values of `{owner}/@{attr}`.").unwrap(),
                None => writeln!(o, "/// XSD simple type `{}`.", d.name).unwrap(),
            }
            o.push_str("#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]\n");
            writeln!(o, "pub enum {name} {{").unwrap();
            let mut seen = HashSet::new();
            for v in values {
                let vn = variant_name(v);
                assert!(seen.insert(vn.clone()), "variant clash {name}::{vn}");
                writeln!(o, "    /// `{v}`\n    {vn},").unwrap();
            }
            o.push_str("}\n\n");
            writeln!(o, "impl {name} {{").unwrap();
            o.push_str("    /// Every value in schema order.\n    pub const ALL: &'static [Self] = &[");
            for v in values {
                write!(o, "Self::{}, ", variant_name(v)).unwrap();
            }
            o.push_str(
                "];\n\n    /// The XML literal.\n    pub fn as_str(self) -> &'static str {\n        match self {\n",
            );
            for v in values {
                writeln!(o, "            Self::{} => {v:?},", variant_name(v)).unwrap();
            }
            o.push_str("        }\n    }\n}\n\n");
            writeln!(o, "impl ParseValue for {name} {{").unwrap();
            o.push_str("    fn parse_value(s: &str) -> Result<Self, ValueError> {\n");
            if *numeric {
                o.push_str("        let v = crate::xsd::parse_xsd_integer(s.trim()).ok_or_else(|| ValueError::new(\"expected an integer\"))?;\n        match v {\n");
                for x in values {
                    writeln!(o, "            {x} => Ok(Self::{}),", variant_name(x)).unwrap();
                }
            } else {
                o.push_str("        match s {\n");
                for x in values {
                    writeln!(o, "            {x:?} => Ok(Self::{}),", variant_name(x)).unwrap();
                }
            }
            writeln!(
                o,
                "            _ => Err(ValueError::new(concat!(\"expected one of: \", {:?}))),",
                values.join(", ")
            )
            .unwrap();
            o.push_str("        }\n    }\n}\n\n");
            writeln!(o, "impl std::fmt::Display for {name} {{\n    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {{\n        f.write_str(self.as_str())\n    }}\n}}\n").unwrap();
            writeln!(o, "impl serde::Serialize for {name} {{\n    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {{\n        s.serialize_str(self.as_str())\n    }}\n}}\n").unwrap();
            writeln!(o, "impl crate::element::ToAttr for {name} {{\n    fn to_attr(&self) -> crate::element::AttrValue {{\n        crate::element::AttrValue::Str(self.as_str().to_string())\n    }}\n}}\n").unwrap();
            self.out.push_str(&o);
        }
    }

    fn emit_newtypes(&mut self) {
        let nts = self.n.newtypes.clone();
        for (name, (si, inner)) in &nts {
            let d = &self.s.simple[*si];
            let (mut min_i, mut max_i, mut min_e, mut max_e) = (None, None, None, None);
            // collect facets along the restriction chain
            let mut cur = *si;
            let base_builtin: &str;
            loop {
                match &self.s.simple[cur].kind {
                    SKind::Restr { base, min_incl, max_incl, min_excl, max_excl, .. } => {
                        min_i = min_i.or(*min_incl);
                        max_i = max_i.or(*max_incl);
                        min_e = min_e.or(*min_excl);
                        max_e = max_e.or(*max_excl);
                        cur = *base;
                    }
                    SKind::Builtin(b) => {
                        base_builtin = *b;
                        break;
                    }
                    _ => panic!("newtype over non-restriction"),
                }
            }
            if base_builtin == "PositiveInteger" && min_i.is_none() {
                min_i = Some(1.0);
            }
            let mut o = String::new();
            writeln!(o, "/// XSD simple type `{}`: a checked `{inner}`.", d.name).unwrap();
            o.push_str(
                "#[derive(Debug, Clone, Copy, PartialEq, PartialOrd, serde::Serialize)]\n#[serde(transparent)]\n",
            );
            writeln!(o, "pub struct {name}({inner});\n").unwrap();
            writeln!(o, "impl {name} {{").unwrap();
            let lit = |v: f64| -> String {
                if inner == "f64" {
                    format!("{v:?}")
                } else {
                    format!("{}", v as i128)
                }
            };
            writeln!(
                o,
                "    /// Inclusive lower bound, if any.\n    pub const MIN_INCLUSIVE: Option<{inner}> = {};",
                min_i.map(|v| format!("Some({})", lit(v))).unwrap_or("None".into())
            )
            .unwrap();
            writeln!(
                o,
                "    /// Inclusive upper bound, if any.\n    pub const MAX_INCLUSIVE: Option<{inner}> = {};",
                max_i.map(|v| format!("Some({})", lit(v))).unwrap_or("None".into())
            )
            .unwrap();
            writeln!(
                o,
                "    /// Exclusive lower bound, if any.\n    pub const MIN_EXCLUSIVE: Option<{inner}> = {};",
                min_e.map(|v| format!("Some({})", lit(v))).unwrap_or("None".into())
            )
            .unwrap();
            writeln!(
                o,
                "    /// Exclusive upper bound, if any.\n    pub const MAX_EXCLUSIVE: Option<{inner}> = {};",
                max_e.map(|v| format!("Some({})", lit(v))).unwrap_or("None".into())
            )
            .unwrap();
            writeln!(o, "\n    /// Checked constructor: fails when `v` violates a facet of `{}`.", d.name).unwrap();
            writeln!(o, "    pub fn new(v: {inner}) -> Result<Self, ValueError> {{").unwrap();
            if let Some(m) = min_i {
                writeln!(
                    o,
                    "        if !(v >= {}) {{ return Err(ValueError::new(\"value is below the minimum {}\")); }}",
                    lit(m),
                    lit(m)
                )
                .unwrap();
            }
            if let Some(m) = max_i {
                writeln!(
                    o,
                    "        if !(v <= {}) {{ return Err(ValueError::new(\"value is above the maximum {}\")); }}",
                    lit(m),
                    lit(m)
                )
                .unwrap();
            }
            if let Some(m) = min_e {
                writeln!(
                    o,
                    "        if !(v > {}) {{ return Err(ValueError::new(\"value must be greater than {}\")); }}",
                    lit(m),
                    lit(m)
                )
                .unwrap();
            }
            if let Some(m) = max_e {
                writeln!(
                    o,
                    "        if !(v < {}) {{ return Err(ValueError::new(\"value must be less than {}\")); }}",
                    lit(m),
                    lit(m)
                )
                .unwrap();
            }
            o.push_str("        Ok(Self(v))\n    }\n\n");
            writeln!(o, "    /// The wrapped value.\n    pub fn get(self) -> {inner} {{\n        self.0\n    }}\n}}\n")
                .unwrap();
            writeln!(o, "impl ParseValue for {name} {{\n    fn parse_value(s: &str) -> Result<Self, ValueError> {{\n        Self::new(<{inner} as ParseValue>::parse_value(s)?)\n    }}\n}}\n").unwrap();
            writeln!(
                o,
                "impl From<{name}> for {inner} {{\n    fn from(v: {name}) -> {inner} {{\n        v.0\n    }}\n}}\n"
            )
            .unwrap();
            writeln!(o, "impl crate::element::ToAttr for {name} {{\n    fn to_attr(&self) -> crate::element::AttrValue {{\n        crate::element::AttrValue::Num(self.0 as f64)\n    }}\n}}\n").unwrap();
            self.out.push_str(&o);
        }
    }

    fn emit_groups(&mut self) {
        for g in self.s.group_order.clone() {
            let p = self.s.groups[&g].clone();
            let e = group_enum_name(&g);
            self.emit_choice_enum(&e, &format!("Model group `{g}`."), &p);
        }
    }

    fn emit(mut self) -> String {
        self.out.push_str("// @generated by build.rs from schema/scene-render-1.1.xsd. Do not edit.\n\n");
        self.emit_enums();
        self.emit_newtypes();
        self.emit_groups();
        for ci in 0..self.s.complex.len() {
            self.emit_struct(ci);
        }
        self.out.push_str("/// Builds the struct of complex type `ty` (index into `COMPLEX_TYPES`) from `n`.\n#[cfg(test)]\npub(crate) fn build_complex(ty: usize, n: XNode<'_, '_>) -> Result<(), ModelError> {\n    match ty {\n");
        for ci in 0..self.s.complex.len() {
            let name = &self.n.complex_rust[ci];
            writeln!(self.out, "        {ci} => {name}::from_xml(n).map(|_| ()),").unwrap();
        }
        self.out.push_str("        _ => unreachable!(),\n    }\n}\n");
        self.out
    }
}

/// Emits the Schematron asserts (pattern, context, id, test, message) so the
/// `explain` command quotes the rule text verbatim.
fn emit_schematron(text: &str) -> String {
    const SCH: &str = "http://purl.oclc.org/dsdl/schematron";
    let doc = roxmltree::Document::parse(text).expect("Schematron is not well-formed");
    let mut o = String::from("// @generated by build.rs from schema/scene-render-1.1.sch. Do not edit.\n\n");
    o.push_str("/// Every Schematron assert: (pattern id, rule context, assert id, XPath test, message).\n");
    o.push_str("pub static SCHEMATRON_ASSERTS: &[SchematronAssert] = &[\n");
    for pat in doc.descendants().filter(|n| n.tag_name().namespace() == Some(SCH) && n.tag_name().name() == "pattern") {
        let pid = pat.attribute("id").unwrap_or("");
        for rule in pat.children().filter(|n| n.tag_name().name() == "rule") {
            let ctx = rule.attribute("context").unwrap_or("");
            for a in rule.children().filter(|n| n.tag_name().name() == "assert") {
                let mut msg = String::new();
                for c in a.children() {
                    if c.is_text() {
                        msg.push_str(c.text().unwrap_or(""));
                    } else if c.tag_name().name() == "value-of" {
                        msg.push_str(&format!("{{{}}}", c.attribute("select").unwrap_or("")));
                    }
                }
                let msg = msg.split_whitespace().collect::<Vec<_>>().join(" ");
                writeln!(
                    o,
                    "    SchematronAssert {{ pattern: {pid:?}, context: {ctx:?}, id: {:?}, test: {:?}, message: {msg:?} }},",
                    a.attribute("id").unwrap(),
                    a.attribute("test").unwrap()
                )
                .unwrap();
            }
        }
    }
    o.push_str("];\n");
    o
}

/// Extracts `(id, pattern, context, test, message)` for every Schematron assert.
fn emit_sch(text: &str) -> String {
    const SCH: &str = "http://purl.oclc.org/dsdl/schematron";
    let doc = roxmltree::Document::parse(text).expect("Schematron is not well-formed");
    let mut o = String::from("// @generated by build.rs from schema/scene-render-1.1.sch. Do not edit.\n\n");
    o.push_str("/// Every Schematron assert: (id, pattern, rule context, XPath test, message).\n");
    o.push_str("pub static SCH_ASSERTS: &[(&str, &str, &str, &str, &str)] = &[\n");
    for pat in doc.descendants().filter(|n| n.has_tag_name((SCH, "pattern"))) {
        let pid = pat.attribute("id").unwrap_or("");
        for rule in pat.children().filter(|n| n.has_tag_name((SCH, "rule"))) {
            let ctx = rule.attribute("context").unwrap_or("");
            for a in rule.children().filter(|n| n.has_tag_name((SCH, "assert"))) {
                let mut msg = String::new();
                for c in a.children() {
                    if c.is_text() {
                        msg.push_str(c.text().unwrap_or(""));
                    } else if c.has_tag_name((SCH, "value-of")) {
                        msg.push_str(&format!("{{{}}}", c.attribute("select").unwrap_or("")));
                    }
                }
                let msg = msg.split_whitespace().collect::<Vec<_>>().join(" ");
                writeln!(
                    o,
                    "    ({:?}, {pid:?}, {ctx:?}, {:?}, {msg:?}),",
                    a.attribute("id").unwrap(),
                    a.attribute("test").unwrap()
                )
                .unwrap();
            }
        }
    }
    o.push_str("];\n");
    o
}

fn main() {
    let sch_path =
        PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap()).join("../../schema/scene-render-1.1.sch");
    println!("cargo:rerun-if-changed={}", sch_path.display());
    let sch = std::fs::read_to_string(&sch_path).expect("read Schematron");
    std::fs::write(PathBuf::from(std::env::var("OUT_DIR").unwrap()).join("sch_asserts.rs"), emit_sch(&sch)).unwrap();
    let manifest = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap());
    let xsd_path = manifest.join("../../schema/scene-render-1.1.xsd");
    println!("cargo:rerun-if-changed={}", xsd_path.display());
    println!("cargo:rerun-if-changed=build.rs");
    let text = std::fs::read_to_string(&xsd_path).expect("read XSD");
    let schema = parse_schema(&text);
    let names = assign_names(&schema);
    let out = PathBuf::from(std::env::var("OUT_DIR").unwrap());
    std::fs::write(out.join("schema_tables.rs"), emit_tables(&schema)).unwrap();
    let sch_path = manifest.join("../../schema/scene-render-1.1.sch");
    println!("cargo:rerun-if-changed={}", sch_path.display());
    let sch = std::fs::read_to_string(&sch_path).expect("read Schematron");
    std::fs::write(out.join("schematron.rs"), emit_schematron(&sch)).unwrap();
    let model = Model { s: &schema, n: &names, out: String::new() }.emit();
    std::fs::write(out.join("model.rs"), model).unwrap();
}
