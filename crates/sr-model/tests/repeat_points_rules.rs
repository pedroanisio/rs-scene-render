//! SREP 26: `repeat/points` in the schema and its rules: the amended C17, the version gate V10 and C74–C80.
//! The cases follow sr-core's tests/test_srep_0026.py.

fn codes_in(version: &str, points: &str, attrs: &str) -> Vec<String> {
    let xml = format!(
        r#"<scene version="{version}"><project width="100" height="100" fps="1" duration="1"/><parameters><param id="d" type="list" default="a,b"/></parameters><composition><repeat id="r"{attrs}>{points}<shape id="s" shape="rect" width="10" height="10"/></repeat></composition></scene>"#
    );
    match sr_model::load_str(&xml, &sr_model::LoadOptions::without_assets()) {
        Ok(_) => Vec::new(),
        Err(sr_model::LoadError::Invalid(r)) => {
            let mut c: Vec<String> = r.diagnostics.iter().filter(|d| d.is_error()).map(|d| d.code.clone()).collect();
            c.sort();
            c.dedup();
            c
        }
        Err(e) => panic!("{e:?}"),
    }
}

fn codes(points: &str, attrs: &str) -> Vec<String> {
    codes_in("1.2", points, attrs)
}

const GRID: &str = r#"<points type="grid" columns="3" rows="2"/>"#;

#[test]
fn accepted_points() {
    for p in [
        GRID,
        r#"<points id="p" type="grid" columns="2" rows="2" spacingX="20" spacingY="30" seed="7"/>"#,
        r#"<points type="along-path" path="M0,0 L10,0" count="3" orient="true"/>"#,
        r#"<points type="scatter" count="5" width="100" height="50" seed="7"/>"#,
        r#"<points type="scatter" count="5" path="M0,0 H100 V100 H0 Z" fillRule="evenodd"/>"#,
        r#"<points type="vertices" path="M0,0 L10,0 L5,5 Z"/>"#,
        r#"<points type="list" at="0,0 10,10 -5.5,1e1"/>"#,
        r#"<points type="grid"><animate property="spacingX"><key time="0" value="50"/><key time="1" value="150"/></animate></points>"#,
    ] {
        assert_eq!(codes(p, ""), Vec::<String>::new(), "{p}");
    }
}

#[test]
fn a_repeat_without_points_is_unchanged() {
    assert!(codes("", r#" count="3""#).is_empty());
    assert!(codes("", r#" over="d""#).is_empty());
    assert_eq!(codes("", ""), ["C17"]);
    assert_eq!(codes("", r#" count="3" over="d""#), ["C17"]);
}

#[test]
fn c17_points_count_and_over_are_exclusive() {
    assert!(codes(GRID, "").is_empty());
    assert_eq!(codes(GRID, r#" count="3""#), ["C17"]);
    assert_eq!(codes(GRID, r#" over="d""#), ["C17"]);
    assert_eq!(codes(GRID, r#" count="3" over="d""#), ["C17"]);
}

#[test]
fn v10_points_need_version_1_2() {
    assert_eq!(codes_in("1.1", GRID, ""), ["V10"]);
    assert!(codes_in("1.0", GRID, "").contains(&"V10".to_string()));
    assert!(codes_in("1.2", GRID, "").is_empty());
    assert!(codes_in("1.3", GRID, "").is_empty());
    assert!(!codes_in("1.1", "", r#" count="2""#).contains(&"V10".to_string()));
}

#[test]
fn c74_at_most_one_points_child() {
    assert_eq!(codes(&format!("{GRID}{GRID}"), ""), ["C74"]);
}

#[test]
fn c80_from_and_step_must_be_neutral() {
    for (attrs, want) in [
        ("", None),
        (r#" from="0" step="1""#, None),
        (r#" from="1""#, Some("C80")),
        (r#" step="2""#, Some("C80")),
        (r#" from="2" step="3""#, Some("C80")),
    ] {
        let got = codes(GRID, attrs);
        match want {
            None => assert!(got.is_empty(), "{attrs}: {got:?}"),
            Some(c) => assert_eq!(got, [c], "{attrs}"),
        }
    }
}

#[test]
fn points_checks() {
    for (rule, p) in [
        ("C75", r#"<points type="along-path" count="3"/>"#),
        ("C75", r#"<points type="along-path" path="M0,0 L1,1"/>"#),
        ("C76", r#"<points type="scatter" width="10" height="10"/>"#),
        ("C76", r#"<points type="scatter" count="3"/>"#),
        ("C76", r#"<points type="scatter" count="3" width="10"/>"#),
        ("C76", r#"<points type="scatter" count="3" path="M0,0 H9 V9 Z" width="10" height="10"/>"#),
        ("C77", r#"<points type="vertices"/>"#),
        ("C78", r#"<points type="list"/>"#),
        (
            "C79",
            r#"<points type="grid"><animate property="columns"><key time="0" value="1"/><key time="1" value="3"/></animate></points>"#,
        ),
    ] {
        assert_eq!(codes(p, ""), [rule], "{p}");
    }
}

#[test]
fn structure_rejections() {
    for p in [
        "<points/>",
        r#"<points type="spiral"/>"#,
        r#"<points type="grid" columns="0"/>"#,
        r#"<points type="list" at="1;2"/>"#,
        r#"<points type="grid" orient="maybe"/>"#,
        r#"<points type="scatter" count="-1" width="1" height="1"/>"#,
        r#"<points type="scatter" count="1" width="-1" height="1"/>"#,
    ] {
        let c = codes(p, "");
        assert!(!c.is_empty() && c.iter().all(|c| c.starts_with('S')), "{p}: {c:?}");
    }
}

#[test]
fn the_model_reads_points() {
    let xml = r#"<scene version="1.2"><project width="100" height="100" fps="1" duration="1"/><composition><repeat id="r"><points id="p" type="list" at="1,2 -3.5,4e1" seed="9"/><shape id="s" shape="rect" width="10" height="10"/></repeat></composition></scene>"#;
    let doc = sr_model::load_str(xml, &sr_model::LoadOptions::without_assets()).unwrap();
    let Some(sr_model::model::Node::Repeat(r)) = doc.node("r") else { panic!("no repeat") };
    let p = r
        .children
        .iter()
        .find_map(|c| match c {
            sr_model::model::RepeatChild::Points(p) => Some(p),
            _ => None,
        })
        .expect("points child");
    let at: Vec<(f64, f64)> = p.at.iter().flatten().map(|q| (q.x, q.y)).collect();
    assert_eq!(at, vec![(1.0, 2.0), (-3.5, 40.0)]);
    assert_eq!(p.seed, Some(9));
}
