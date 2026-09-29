//! The typed scene-render 1.1 model, generated from the XSD.
//!
//! Naming rules (applied by `build.rs`):
//!
//! * complex type `fooType` → struct `Foo`; the root element → [`Scene`];
//! * named enumerations `fooType` → enum `Foo`; anonymous enumerations are
//!   named after their attribute (`FrameBlend`) or, when the attribute name
//!   is generic, after owner and attribute (`EffectKind`, `MaskMode`);
//! * named numeric restrictions → checked newtypes (`UnitDecimal`);
//! * attributes → `snake_case` fields; defaults are applied, optional
//!   attributes without a default are `Option`;
//! * sequences of distinct elements → one field per element; repeated
//!   choices → `children: Vec<…Child>` in document order; model groups →
//!   shared enums ([`Animation`], [`NodeBehaviour`], [`BodyBehaviour`], [`Node`]).

#![allow(missing_docs, dead_code, unused_imports, unused_variables, clippy::all)]

use crate::diag::Loc;
use crate::parse::{def, opt, req, text_content, ModelError, ParseValue, ValueError};

type XNode<'a, 'i> = roxmltree::Node<'a, 'i>;

include!(concat!(env!("OUT_DIR"), "/model.rs"));

impl Scene {
    /// Builds the typed model from a document that passed structural validation.
    pub(crate) fn build(doc: &roxmltree::Document<'_>) -> Result<Scene, ModelError> {
        Scene::from_xml(doc.root_element())
    }
}

impl Node {
    /// Child nodes of containers (`group`, `sequence`, `repeat`), in document order.
    pub fn child_nodes(&self) -> Box<dyn Iterator<Item = &Node> + '_> {
        match self {
            Node::Group(g) => Box::new(g.children.iter().filter_map(GroupChild::as_node)),
            Node::Sequence(s) => Box::new(s.children.iter().filter_map(GroupChild::as_node)),
            Node::Repeat(r) => Box::new(r.children.iter().filter_map(|c| match c {
                RepeatChild::Node(n) => Some(n),
                _ => None,
            })),
            _ => Box::new(std::iter::empty()),
        }
    }

    /// The `i`-th child node.
    fn child_slot(&self, i: usize) -> Option<&Node> {
        self.child_nodes().nth(i)
    }

    /// The node at `path` below this node (see [`crate::document::NodePath`]).
    pub fn descend(&self, path: &[u32]) -> Option<&Node> {
        let mut cur = self;
        for &i in path {
            cur = cur.child_slot(i as usize)?;
        }
        Some(cur)
    }
}

impl GroupChild {
    /// The node, when this child is a node rather than a behaviour.
    pub fn as_node(&self) -> Option<&Node> {
        match self {
            GroupChild::Node(n) => Some(n),
            _ => None,
        }
    }
}

#[cfg(test)]
mod coverage {
    //! Every attribute of every complex type round-trips from a lexical value
    //! through the structural validator and into the typed model, and every
    //! schema default parses.

    use crate::xsd::simple::check;
    use crate::xsd::{root_builtin, Builtin, SimpleKind, COMPLEX_TYPES, SIMPLE_TYPES};

    fn sample(ty: usize) -> String {
        let t = &SIMPLE_TYPES[ty];
        let named = match t.name {
            "fpsType" => Some("30000/1001"),
            "aspectType" => Some("16:9"),
            "sha256Type" => Some("0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef"),
            "timecodeType" => Some("01:00:00;02"),
            "languageTagType" => Some("en-US"),
            "pointType" => Some("0.5,-1e2"),
            "relativeLength" => Some("-50%"),
            "positiveRelativeLength" => Some("10vw"),
            "colorType" => Some("#ff8800"),
            "paintRefType" => Some("url(#p)"),
            "numberListType" => Some("1 2.5 -3"),
            _ => None,
        };
        if let Some(v) = named {
            return v.into();
        }
        match t.kind {
            SimpleKind::Builtin(b) => match b {
                Builtin::String | Builtin::AnyUri => "media/x.png".into(),
                Builtin::Double => "1.5".into(),
                Builtin::Boolean => "true".into(),
                Builtin::Integer | Builtin::Int => "-3".into(),
                Builtin::NonNegativeInteger | Builtin::PositiveInteger | Builtin::UnsignedLong => "3".into(),
                Builtin::Id | Builtin::IdRef | Builtin::NcName => "x1".into(),
                Builtin::IdRefs | Builtin::NmTokens => "a b".into(),
                Builtin::NmToken => "n-1".into(),
                Builtin::DateTime => "2026-09-27T10:00:00Z".into(),
            },
            SimpleKind::Restriction(r) => {
                if let Some(e) = r.enums.first() {
                    return e.to_string();
                }
                if let Some(p) = r.pattern {
                    // lon,lat pairs (geoPointsType); otherwise a token-like string
                    return if p.contains(',') { "0,0 1,1".into() } else { "brand-1".into() };
                }
                let integer = !matches!(root_builtin(ty), Some(Builtin::Double | Builtin::String));
                let lo = r.min_inclusive.or(r.min_exclusive.map(|m| m + 1.0));
                let hi = r.max_inclusive.or(r.max_exclusive.map(|m| m - 1.0));
                let v = match (lo, hi) {
                    (Some(a), Some(b)) => (a + b) / 2.0,
                    (Some(a), None) => a + 1.0,
                    (None, Some(b)) => b.min(1.0),
                    (None, None) => return sample(r.base),
                };
                if integer {
                    format!("{}", v.round() as i64)
                } else {
                    format!("{v}")
                }
            }
            SimpleKind::Union(m) => sample(m[0]),
            SimpleKind::List(item) => format!("{} {}", sample(item), sample(item)),
        }
    }

    fn build(ci: usize, attrs: &[(&str, String)]) {
        let body: String = attrs.iter().map(|(k, v)| format!(" {k}=\"{v}\"")).collect();
        let xml = format!("<e{body}/>");
        let doc = roxmltree::Document::parse(&xml).unwrap();
        if let Err(e) = super::build_complex(ci, doc.root_element()) {
            panic!("{}: {xml}: {e}", COMPLEX_TYPES[ci].name);
        }
    }

    #[test]
    fn every_attribute_parses_into_the_model() {
        let mut attrs_checked = 0;
        for (ci, ct) in COMPLEX_TYPES.iter().enumerate() {
            if ci == crate::xsd::ROOT_TYPE {
                continue; // needs <project> and <composition>; covered by the corpus
            }
            let all: Vec<(&str, String)> = ct
                .attrs
                .iter()
                .map(|a| {
                    let v = sample(a.ty);
                    assert!(
                        check(a.ty, &v).is_ok(),
                        "sample {v:?} for {}@{} fails its own type: {:?}",
                        ct.name,
                        a.name,
                        check(a.ty, &v)
                    );
                    (a.name, v)
                })
                .collect();
            attrs_checked += all.len();
            build(ci, &all);
            let required: Vec<(&str, String)> =
                ct.attrs.iter().zip(&all).filter(|(a, _)| a.required).map(|(_, kv)| kv.clone()).collect();
            build(ci, &required);
        }
        assert!(attrs_checked > 1000, "{attrs_checked}");
    }

    #[test]
    fn every_default_is_valid_for_its_type() {
        for ct in COMPLEX_TYPES {
            for a in ct.attrs {
                if let Some(d) = a.default {
                    assert!(check(a.ty, d).is_ok(), "{}@{} default {d:?}: {:?}", ct.name, a.name, check(a.ty, d));
                }
            }
        }
    }
}
