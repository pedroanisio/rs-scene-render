//! SREP 22: the engine's capability manifest, `capabilities.json`, and the `SUP-*` findings a render report gets from
//! it.

use sr_deliver::capabilities::{findings, manifest, Construct, Entry, Manifest, Status, FORMAT, MANIFEST};

/// Every complex type an element of this name is declared with, anywhere in the schema.
fn element_types(name: &str) -> Vec<&'static sr_model::xsd::ComplexType> {
    use sr_model::xsd::{Content, Particle, COMPLEX_TYPES};
    let mut out = Vec::new();
    fn walk(p: &Particle, name: &str, out: &mut Vec<usize>) {
        match p {
            Particle::Element { name: n, ty, .. } => {
                if *n == name && !out.contains(ty) {
                    out.push(*ty);
                }
            }
            Particle::Seq { items, .. } | Particle::Choice { items, .. } => {
                items.iter().for_each(|i| walk(i, name, out));
            }
        }
    }
    let mut idx = Vec::new();
    for ct in COMPLEX_TYPES.iter() {
        if let Content::Elements(p) = ct.content {
            walk(p, name, &mut idx);
        }
    }
    out.extend(idx.into_iter().map(|i| &COMPLEX_TYPES[i]));
    out
}

/// Whether the schema has the construct: the element, the attribute on it, the value in the attribute's type.
fn in_schema(c: &Construct) -> bool {
    element_types(c.element).iter().any(|ct| match c.attribute {
        None => true,
        Some(a) => ct.attr(a).is_some_and(|d| c.value.is_none_or(|v| sr_model::xsd::simple::check(d.ty, v).is_ok())),
    })
}

#[test]
fn the_published_manifest_is_well_formed() {
    let m = manifest();
    assert_eq!(m.format, FORMAT);
    assert_eq!(m.engine.name, "rs-scene-render");
    assert_eq!(m.engine.version, env!("CARGO_PKG_VERSION"), "the manifest names this release");
    // the schema this engine vendors, as schema/UPSTREAM records its release
    let upstream =
        std::fs::read_to_string(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../schema/UPSTREAM"))
            .unwrap();
    let named = upstream
        .match_indices(&format!("release {}", m.schema))
        .any(|(i, t)| !upstream[i + t.len()..].starts_with(|c: char| c.is_ascii_digit() || c == '.'));
    assert!(named, "schema {} is not the vendored release in schema/UPSTREAM", m.schema);
    assert!(m.schema.starts_with(sr_model::SCHEMA_VERSION));
    let mut seen = std::collections::HashSet::new();
    for e in &m.entries {
        let c = Construct::parse(&e.construct).unwrap_or_else(|| panic!("{}: not a construct", e.construct));
        assert!(in_schema(&c), "{}: not in the scene schema", e.construct);
        assert_ne!(e.status, Status::Exact, "{}: an exact construct is not listed", e.construct);
        assert!(seen.insert(&e.construct), "{}: listed twice", e.construct);
    }
    // entries are sorted by construct, so the file diffs cleanly
    let names: Vec<&str> = m.entries.iter().map(|e| e.construct.as_str()).collect();
    let mut sorted = names.clone();
    sorted.sort_unstable();
    assert_eq!(names, sorted);
    // the JSON carries no field outside the format
    let v: serde_json::Value = serde_json::from_str(MANIFEST).unwrap();
    let keys: std::collections::BTreeSet<&str> = v.as_object().unwrap().keys().map(String::as_str).collect();
    assert_eq!(keys, ["engine", "entries", "format", "schema"].into_iter().collect());
}

#[test]
fn constructs_parse_in_the_three_forms_only() {
    let c = |s| Construct::parse(s);
    assert_eq!(c("pattern"), Some(Construct { element: "pattern", attribute: None, value: None }));
    assert_eq!(
        c("layer/@frameBlend"),
        Some(Construct { element: "layer", attribute: Some("frameBlend"), value: None })
    );
    assert_eq!(
        c("layer/@frameBlend=optical-flow"),
        Some(Construct { element: "layer", attribute: Some("frameBlend"), value: Some("optical-flow") })
    );
    for bad in ["", "a/b", "a/@", "a/@b=", "/@b", "a b"] {
        assert_eq!(c(bad), None, "{bad:?}");
    }
}

fn test_manifest() -> Manifest {
    let entry = |construct: &str, status| Entry {
        construct: construct.into(),
        status,
        definition: None,
        note: None,
        when: Vec::new(),
    };
    Manifest {
        format: FORMAT.into(),
        engine: sr_deliver::capabilities::Engine { name: "rs-scene-render".into(), version: "0.0.0".into() },
        schema: "1.3".into(),
        entries: vec![
            entry("layer/@frameBlend=optical-flow", Status::Reported),
            entry("pattern", Status::Approximate),
            entry("shape/@radius", Status::Unsupported),
        ],
    }
}

#[test]
fn srep_0022_reported() {
    // a scene using a construct the manifest lists as reported: SUP-REPORTED naming the construct, once
    let xml = r#"<scene version="1.2"><project width="8" height="8" fps="1" duration="1"/><assets><video id="v" src="a.mp4" width="8" height="8" fps="1" duration="1"/></assets>
      <composition><layer id="a" asset="v" frameBlend="optical-flow"/><layer id="b" asset="v" frameBlend="optical-flow"/><layer id="c" asset="v" frameBlend="frame-mix"/></composition></scene>"#;
    let f = findings(&test_manifest(), xml);
    assert_eq!(f.len(), 1, "{f:?}");
    assert_eq!((f[0].code.as_str(), f[0].severity), ("SUP-REPORTED", sr_model::Severity::Warning));
    assert!(f[0].message.contains("layer/@frameBlend=optical-flow"), "{}", f[0].message);
    assert_eq!(f[0].path, "/scene/composition/layer[1]");
}

#[test]
fn approximate_constructs_are_sup_approx_and_unused_ones_nothing() {
    let xml = r##"<scene version="1.2"><project width="8" height="8" fps="1" duration="1"/><paints><pattern id="p" asset="img"/></paints><composition/></scene>"##;
    let f = findings(&test_manifest(), xml);
    assert_eq!(f.iter().map(|f| f.code.as_str()).collect::<Vec<_>>(), ["SUP-APPROX"]);
    // an unsupported construct is not reported (the document could not use it), nor is anything the document lacks
    let none = r#"<scene version="1.2"><project width="8" height="8" fps="1" duration="1"/><composition><shape id="s" shape="rect" width="4" height="4" radius="2"/></composition></scene>"#;
    assert!(findings(&test_manifest(), none).is_empty());
}

/// The note of a construct an accepted SREP adds and this engine does not implement yet.
fn pending_note(srep: u32) -> String {
    format!("SREP {srep} is accepted and not implemented by this engine yet")
}

#[test]
fn every_pending_construct_is_listed_as_reported_and_nothing_else_is_pending() {
    use sr_eval::pending::PENDING;
    let m = manifest();
    let listed = |construct: &str| m.entries.iter().find(|e| e.construct == construct);
    let mut pending_constructs = Vec::new();
    for p in PENDING {
        let constructs =
            p.elements.iter().map(|e| e.to_string()).chain(p.attributes.iter().map(|(e, a, _)| format!("{e}/@{a}")));
        for c in constructs {
            let e = listed(&c).unwrap_or_else(|| panic!("{c} (SREP {}) is not in capabilities.json", p.srep));
            assert_eq!(e.status, Status::Reported, "{c}");
            assert!(e.note.as_deref().is_some_and(|n| n.starts_with(&pending_note(p.srep))), "{c}: {:?}", e.note);
            pending_constructs.push(c);
        }
    }
    // an implemented SREP leaves both the table and the manifest
    for e in &m.entries {
        if e.note.as_deref().is_some_and(|n| n.contains("not implemented by this engine yet")) {
            assert!(pending_constructs.contains(&e.construct), "{} is listed as pending but has no row", e.construct);
        }
    }
}

#[test]
fn pathtrace_limitations_are_reported_on_the_affected_constructs_only() {
    // Conditional entries identify each affected construct only when the path tracer is also used.
    let scene = |camera: &str| {
        format!(
            r#"<scene version="1.2"><project width="32" height="32" fps="1" duration="1"/><materials><material id="m" unevenness="0.4"/></materials>
      <composition><camera id="c" {camera}/><object3D id="floor" primitive="plane" width="10" height="10" shadowCatcher="true" material="m"/></composition></scene>"#
        )
    };
    assert!(findings(&manifest(), &scene("")).is_empty(), "default renderer (raster)");
    assert!(findings(&manifest(), &scene(r#"renderer="raster""#)).is_empty());
    let f = findings(&manifest(), &scene(r#"renderer="pathtrace""#));
    assert_eq!(f.iter().map(|f| f.code.as_str()).collect::<Vec<_>>(), ["SUP-APPROX", "SUP-REPORTED"]);
    assert!(f[0].message.contains("material/@unevenness"), "{}", f[0].message);
    assert_eq!(f[0].path, "/scene/materials/material");
    assert!(f[1].message.contains("object3D/@shadowCatcher=true"), "{}", f[1].message);
    assert_eq!(f[1].path, "/scene/composition/object3D");
    let exact =
        r#"<scene><composition><camera renderer="pathtrace"/><object3D primitive="plane"/></composition></scene>"#;
    assert!(findings(&manifest(), exact).is_empty(), "the path tracer alone is not a limitation");
}

#[test]
fn a_conditional_capability_requires_every_companion_construct() {
    let mut m = test_manifest();
    m.entries.retain(|e| e.construct == "pattern");
    m.entries[0].when = vec!["camera/@renderer=pathtrace".into(), "object3D/@shadowCatcher=true".into()];
    assert!(findings(&m, r#"<scene><pattern/><camera renderer="pathtrace"/></scene>"#).is_empty());
    assert_eq!(
        findings(&m, r#"<scene><pattern/><camera renderer="pathtrace"/><object3D shadowCatcher="true"/></scene>"#)
            .len(),
        1
    );
}
