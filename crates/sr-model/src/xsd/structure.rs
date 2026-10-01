//! Structural validation: content models, attributes, simple values, IDs.
//!
//! Diagnostic codes:
//!
//! | code | meaning |
//! |------|---------|
//! | S01 | root element is not `<scene>` |
//! | S02 | element not allowed here (unknown, misplaced or too many) |
//! | S03 | required child element missing |
//! | S04 | attribute not declared for this element |
//! | S05 | required attribute missing |
//! | S06 | attribute value violates its type |
//! | S07 | text inside element-only content |
//! | S08 | text content violates its type |
//! | S09 | duplicate `xs:ID` value |
//! | S10 | `xs:IDREF` value names no `xs:ID` |
//! | S11 | element or attribute in a foreign namespace |
//! | S12 | nesting deeper than the supported limit |
//! | W01 | non-finite number (`INF`, `-INF`, `NaN` or an overflowing literal) in a numeric attribute |

use std::collections::{BTreeSet, HashMap};

use roxmltree::{Document, Node};

use super::simple::{self, collapse};
use super::{ComplexType, Content, IdKind, Particle, COMPLEX_TYPES, ROOT_ELEMENT, ROOT_TYPE};
use crate::diag::{element_path, Diagnostic, Loc};

const XSI: &str = "http://www.w3.org/2001/XMLSchema-instance";

/// Maximum element nesting depth accepted.
pub const MAX_DEPTH: usize = 256;

/// Maximum structural errors reported for the children of one element.
const MAX_CHILD_ERRORS: usize = 32;

struct IdRefUse<'a, 'i> {
    value: String,
    node: Node<'a, 'i>,
    attr: &'static str,
}

struct Walker<'a, 'i> {
    diags: Vec<Diagnostic>,
    ids: HashMap<String, Node<'a, 'i>>,
    refs: Vec<IdRefUse<'a, 'i>>,
}

/// Every `xs:ID` value of a document with the element that carries it.
pub type IdMap = HashMap<String, (String, Loc)>;

/// Validates the document structure against the XSD, appends diagnostics
/// and returns the `xs:ID` values found.
pub fn validate(doc: &Document<'_>, out: &mut Vec<Diagnostic>) -> IdMap {
    let root = doc.root_element();
    let mut w = Walker { diags: Vec::new(), ids: HashMap::new(), refs: Vec::new() };
    if root.tag_name().name() != ROOT_ELEMENT || root.tag_name().namespace().is_some() {
        w.diags.push(
            Diagnostic::error(
                "S01",
                format!("root element is <{}>; a scene-render document starts with <scene>", qname(root)),
                Loc::of(root),
                element_path(root),
            )
            .with_help("wrap the document in <scene version=\"1.1\"> … </scene>"),
        );
        out.append(&mut w.diags);
        return IdMap::new();
    }
    w.element(root, &COMPLEX_TYPES[ROOT_TYPE], 0);
    w.check_refs();
    out.append(&mut w.diags);
    w.ids.into_iter().map(|(k, n)| (k, (n.tag_name().name().to_string(), Loc::of(n)))).collect()
}

fn qname(n: Node) -> String {
    match n.tag_name().namespace() {
        Some(ns) => format!("{{{ns}}}{}", n.tag_name().name()),
        None => n.tag_name().name().to_string(),
    }
}

/// Levenshtein distance for "did you mean" suggestions.
pub(crate) fn distance(a: &str, b: &str) -> usize {
    let a: Vec<char> = a.chars().collect();
    let b: Vec<char> = b.chars().collect();
    let mut prev: Vec<usize> = (0..=b.len()).collect();
    for i in 1..=a.len() {
        let mut cur = vec![i; b.len() + 1];
        for j in 1..=b.len() {
            let cost = usize::from(!a[i - 1].eq_ignore_ascii_case(&b[j - 1]));
            cur[j] = (prev[j] + 1).min(cur[j - 1] + 1).min(prev[j - 1] + cost);
        }
        prev = cur;
    }
    prev[b.len()]
}

/// The closest candidate within a small edit distance.
pub(crate) fn suggest<'c>(word: &str, candidates: impl IntoIterator<Item = &'c str>) -> Option<&'c str> {
    let limit = (word.chars().count() / 3).clamp(1, 3);
    candidates
        .into_iter()
        .map(|c| (distance(word, c), c))
        .filter(|(d, _)| *d <= limit)
        .min_by_key(|(d, c)| (*d, c.len()))
        .map(|(_, c)| c)
}

fn is_ws(s: &str) -> bool {
    s.bytes().all(|b| matches!(b, b' ' | b'\t' | b'\n' | b'\r'))
}

fn format_names(set: &BTreeSet<&'static str>) -> String {
    let v: Vec<String> = set.iter().map(|n| format!("<{n}>")).collect();
    match v.len() {
        0 => "no child elements".to_string(),
        1 => v[0].clone(),
        _ => format!("one of {}", v.join(", ")),
    }
}

// ------------------------------------------------------------------ particle matching

struct Missing {
    pos: usize,
    names: BTreeSet<&'static str>,
}

struct Matcher<'k, 'a, 'i> {
    kids: &'k [Node<'a, 'i>],
    assign: Vec<Option<usize>>,
    expect: Vec<BTreeSet<&'static str>>,
}

fn first_names(p: &Particle, out: &mut BTreeSet<&'static str>) {
    match p {
        Particle::Element { name, .. } => {
            out.insert(name);
        }
        Particle::Choice { items, .. } => items.iter().for_each(|i| first_names(i, out)),
        Particle::Seq { items, .. } => {
            for i in items.iter() {
                first_names(i, out);
                if min_of(i) > 0 {
                    break;
                }
            }
        }
    }
}

fn min_of(p: &Particle) -> u32 {
    match p {
        Particle::Element { min, .. } | Particle::Seq { min, .. } | Particle::Choice { min, .. } => *min,
    }
}

impl<'k, 'a, 'i> Matcher<'k, 'a, 'i> {
    fn is(&self, pos: usize, name: &str) -> bool {
        self.kids.get(pos).is_some_and(|k| k.tag_name().name() == name && k.tag_name().namespace().is_none())
    }

    fn m(&mut self, p: &Particle, mut pos: usize) -> Result<usize, Missing> {
        match p {
            Particle::Element { name, ty, min, max } => {
                let mut count = 0;
                while count < *max && self.is(pos, name) {
                    self.assign[pos] = Some(*ty);
                    pos += 1;
                    count += 1;
                }
                if count < *max {
                    self.expect[pos].insert(name);
                }
                if count < *min {
                    return Err(Missing { pos, names: BTreeSet::from([*name]) });
                }
                Ok(pos)
            }
            Particle::Seq { items, min, max } => {
                let mut count = 0;
                while count < *max {
                    let start = pos;
                    let saved = self.assign.clone();
                    let mut cur = pos;
                    let mut failed = None;
                    for it in items.iter() {
                        match self.m(it, cur) {
                            Ok(np) => cur = np,
                            Err(e) => {
                                failed = Some(e);
                                break;
                            }
                        }
                    }
                    match failed {
                        Some(e) if count < *min => return Err(e),
                        Some(_) => {
                            self.assign = saved;
                            break;
                        }
                        None => {
                            pos = cur;
                            count += 1;
                            if pos == start {
                                break;
                            }
                        }
                    }
                }
                if count < *min {
                    let mut names = BTreeSet::new();
                    first_names(p, &mut names);
                    return Err(Missing { pos, names });
                }
                Ok(pos)
            }
            Particle::Choice { items, min, max } => {
                let mut count = 0;
                while count < *max {
                    let mut progressed = false;
                    for it in items.iter() {
                        if let Ok(np) = self.m(it, pos) {
                            if np > pos {
                                pos = np;
                                progressed = true;
                                break;
                            }
                        }
                    }
                    if !progressed {
                        break;
                    }
                    count += 1;
                }
                if count < *min {
                    let mut names = BTreeSet::new();
                    first_names(p, &mut names);
                    return Err(Missing { pos, names });
                }
                Ok(pos)
            }
        }
    }
}

// ------------------------------------------------------------------ walker

impl<'a, 'i> Walker<'a, 'i> {
    fn element(&mut self, n: Node<'a, 'i>, ct: &'static ComplexType, depth: usize) {
        if depth > MAX_DEPTH {
            self.diags.push(Diagnostic::error(
                "S12",
                format!("elements are nested more than {MAX_DEPTH} levels deep"),
                Loc::of(n),
                element_path(n),
            ));
            return;
        }
        self.attributes(n, ct);
        match ct.content {
            Content::Empty => {
                for c in n.children() {
                    if c.is_element() {
                        self.diags.push(Diagnostic::error(
                            "S02",
                            format!(
                                "<{}> is not allowed inside <{}>, which takes no child elements",
                                qname(c),
                                n.tag_name().name()
                            ),
                            Loc::of(c),
                            element_path(c),
                        ));
                    } else if c.is_text() && !is_ws(c.text().unwrap_or("")) {
                        self.text_error(n, c);
                    }
                }
            }
            Content::Simple(ty) => {
                let mut text = String::new();
                for c in n.children() {
                    if c.is_element() {
                        self.diags.push(Diagnostic::error(
                            "S02",
                            format!(
                                "<{}> is not allowed inside <{}>, which takes text only",
                                qname(c),
                                n.tag_name().name()
                            ),
                            Loc::of(c),
                            element_path(c),
                        ));
                    } else if c.is_text() {
                        text.push_str(c.text().unwrap_or(""));
                    }
                }
                if let Err(e) = simple::check(ty, &text) {
                    self.diags.push(Diagnostic::error(
                        "S08",
                        format!("text of <{}>: {e}", n.tag_name().name()),
                        Loc::of(n),
                        element_path(n),
                    ));
                }
            }
            Content::Elements(p) => {
                for c in n.children() {
                    if c.is_text() && !is_ws(c.text().unwrap_or("")) {
                        self.text_error(n, c);
                    }
                }
                let mut kids: Vec<Node<'a, 'i>> = n.children().filter(|c| c.is_element()).collect();
                let mut errors = 0;
                loop {
                    let mut m = Matcher {
                        kids: &kids,
                        assign: vec![None; kids.len()],
                        expect: vec![BTreeSet::new(); kids.len() + 1],
                    };
                    let res = m.m(p, 0);
                    let (bad, expected) = match res {
                        Ok(pos) if pos == kids.len() => {
                            let assign = m.assign;
                            for (k, t) in kids.iter().zip(assign) {
                                if let Some(t) = t {
                                    self.element(*k, &COMPLEX_TYPES[t], depth + 1);
                                }
                            }
                            break;
                        }
                        Ok(pos) => (pos, std::mem::take(&mut m.expect[pos])),
                        Err(miss)
                            if miss.pos < kids.len() && !miss.names.contains(kids[miss.pos].tag_name().name()) =>
                        {
                            let mut exp = std::mem::take(&mut m.expect[miss.pos]);
                            exp.extend(miss.names);
                            (miss.pos, exp)
                        }
                        Err(miss) => {
                            let at = kids.get(miss.pos).copied();
                            let msg = match at {
                                Some(k) => format!(
                                    "<{}> is missing {} before <{}>",
                                    n.tag_name().name(),
                                    format_names(&miss.names),
                                    k.tag_name().name()
                                ),
                                None => format!(
                                    "<{}> is missing required child {}",
                                    n.tag_name().name(),
                                    format_names(&miss.names)
                                ),
                            };
                            self.diags.push(Diagnostic::error(
                                "S03",
                                msg,
                                Loc::of(at.unwrap_or(n)),
                                element_path(at.unwrap_or(n)),
                            ));
                            let assign = m.assign;
                            for (k, t) in kids.iter().zip(assign) {
                                if let Some(t) = t {
                                    self.element(*k, &COMPLEX_TYPES[t], depth + 1);
                                }
                            }
                            break;
                        }
                    };
                    let k = kids[bad];
                    let name = k.tag_name().name();
                    let mut d = if k.tag_name().namespace().is_some() {
                        Diagnostic::error(
                            "S11",
                            format!(
                                "<{}> is in namespace {}; scene-render elements have no namespace",
                                name,
                                k.tag_name().namespace().unwrap()
                            ),
                            Loc::of(k),
                            element_path(k),
                        )
                    } else {
                        let allowed_anywhere = allowed_children(p);
                        let reason = if allowed_anywhere.contains(name) {
                            "is out of order or repeated too often"
                        } else {
                            "is not allowed"
                        };
                        Diagnostic::error(
                            "S02",
                            format!(
                                "<{name}> {reason} inside <{}>; expected {}",
                                n.tag_name().name(),
                                format_names(&expected)
                            ),
                            Loc::of(k),
                            element_path(k),
                        )
                    };
                    if !allowed_children(p).contains(name) {
                        if let Some(s) = suggest(name, allowed_children(p).iter().copied()) {
                            d = d.with_help(format!("did you mean <{s}>?"));
                        }
                    }
                    self.diags.push(d);
                    kids.remove(bad);
                    errors += 1;
                    if errors >= MAX_CHILD_ERRORS {
                        break;
                    }
                }
            }
        }
    }

    fn text_error(&mut self, parent: Node<'a, 'i>, text: Node<'a, 'i>) {
        let t = text.text().unwrap_or("").trim();
        let shown: String = t.chars().take(40).collect();
        self.diags.push(Diagnostic::error(
            "S07",
            format!("text {shown:?} is not allowed inside <{}>", parent.tag_name().name()),
            Loc::at(text.range().start + text.text().unwrap_or("").find(|c: char| !c.is_whitespace()).unwrap_or(0)),
            element_path(parent),
        ));
    }

    fn attributes(&mut self, n: Node<'a, 'i>, ct: &'static ComplexType) {
        let ename = n.tag_name().name();
        for a in n.attributes() {
            if a.namespace() == Some(XSI) {
                continue;
            }
            let loc = Loc::at(a.range().start);
            if let Some(ns) = a.namespace() {
                self.diags.push(Diagnostic::error(
                    "S11",
                    format!("attribute {}:{} is in namespace {ns}, which <{ename}> does not accept", ns, a.name()),
                    loc,
                    element_path(n),
                ));
                continue;
            }
            let Some(decl) = ct.attr(a.name()) else {
                let mut d =
                    Diagnostic::error("S04", format!("<{ename}> has no attribute @{}", a.name()), loc, element_path(n));
                if let Some(s) = suggest(a.name(), ct.attrs.iter().map(|d| d.name)) {
                    d = d.with_help(format!("did you mean @{s}?"));
                }
                self.diags.push(d);
                continue;
            };
            let value = a.value();
            if let Err(e) = simple::check(decl.ty, value) {
                self.diags.push(Diagnostic::error(
                    "S06",
                    format!("@{} of <{ename}>: {e}", decl.name),
                    loc,
                    element_path(n),
                ));
                continue;
            }
            if let Some(v) = simple::non_finite(decl.ty, value) {
                self.diags.push(Diagnostic::warning(
                    "W01",
                    format!("@{} of <{ename}> is {v}; the renderer requires finite numbers here", decl.name),
                    loc,
                    element_path(n),
                ));
            }
            match decl.id {
                IdKind::None => {}
                IdKind::Id => {
                    let v = collapse(value);
                    if let Some(first) = self.ids.get(&v) {
                        let first_loc = Loc::of(*first);
                        self.diags.push(
                            Diagnostic::error(
                                "S09",
                                format!(
                                    "ID {v:?} is already used by <{}> at line {}",
                                    first.tag_name().name(),
                                    first_loc.line
                                ),
                                loc,
                                element_path(n),
                            )
                            .with_help("every id must be unique across the whole document"),
                        );
                    } else {
                        self.ids.insert(v, n);
                    }
                }
                IdKind::IdRef => self.refs.push(IdRefUse { value: collapse(value), node: n, attr: decl.name }),
                IdKind::IdRefs => {
                    for t in collapse(value).split(' ') {
                        self.refs.push(IdRefUse { value: t.to_string(), node: n, attr: decl.name });
                    }
                }
            }
        }
        for decl in ct.attrs.iter().filter(|d| d.required) {
            if n.attribute(decl.name).is_none() {
                self.diags.push(Diagnostic::error(
                    "S05",
                    format!("<{ename}> requires @{}", decl.name),
                    Loc::of(n),
                    element_path(n),
                ));
            }
        }
    }

    fn check_refs(&mut self) {
        for r in std::mem::take(&mut self.refs) {
            if self.ids.contains_key(&r.value) {
                continue;
            }
            let loc = r
                .node
                .attributes()
                .find(|a| a.name() == r.attr && a.namespace().is_none())
                .map(|a| Loc::at(a.range().start))
                .unwrap_or_else(|| Loc::of(r.node));
            let mut d = Diagnostic::error(
                "S10",
                format!(
                    "@{} of <{}> refers to {:?}, but no element has that id",
                    r.attr,
                    r.node.tag_name().name(),
                    r.value
                ),
                loc,
                element_path(r.node),
            );
            if let Some(s) = suggest(&r.value, self.ids.keys().map(String::as_str)) {
                d = d.with_help(format!("did you mean {s:?}?"));
            }
            self.diags.push(d);
        }
    }
}

fn allowed_children(p: &Particle) -> BTreeSet<&'static str> {
    fn walk(p: &Particle, out: &mut BTreeSet<&'static str>) {
        match p {
            Particle::Element { name, .. } => {
                out.insert(name);
            }
            Particle::Seq { items, .. } | Particle::Choice { items, .. } => items.iter().for_each(|i| walk(i, out)),
        }
    }
    let mut s = BTreeSet::new();
    walk(p, &mut s);
    s
}
