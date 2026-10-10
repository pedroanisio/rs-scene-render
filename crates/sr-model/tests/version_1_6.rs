//! Document version 1.6: the base for the SREPs accepted on 2026-10-09, with no syntax of its own yet.

fn load(version: &str) -> Result<sr_model::Document, sr_model::LoadError> {
    let xml = format!(
        r##"<scene version="{version}"><project width="64" height="64" fps="10" duration="1"/><composition><shape id="s" shape="rect" width="10" height="10" fill="#FFFFFF"/></composition></scene>"##
    );
    sr_model::load_str(&xml, &sr_model::LoadOptions::without_assets())
}

#[test]
fn version_1_6_is_accepted_and_reported() {
    assert_eq!(sr_model::SCHEMA_VERSION, "1.6");
    let d = load("1.6").expect("a 1.6 document validates");
    assert_eq!(d.version().as_str(), "1.6");
    assert!(load("1.5").is_ok());
}

#[test]
fn a_version_after_1_6_is_not() {
    for v in ["1.7", "2.0", "1.60"] {
        assert!(load(v).is_err(), "{v} is accepted");
    }
}

/// Every document of the conformance corpus validates the same way as 1.5 and as 1.6: the new version changes no rule.
#[test]
fn the_corpus_validates_alike_at_1_5_and_1_6() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/corpus");
    let mut compared = 0;
    for dir in ["valid", "invalid"] {
        let mut files: Vec<_> = std::fs::read_dir(root.join(dir))
            .unwrap()
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.to_string_lossy().ends_with(".scene.xml"))
            .collect();
        files.sort();
        for f in files {
            let xml = std::fs::read_to_string(&f).unwrap();
            let Some(at) = xml.find("<scene") else { continue };
            let head_end = at + xml[at..].find('>').unwrap();
            let set = |v: &str| {
                let head = &xml[at..head_end];
                let Some(i) = head.find("version=\"") else { return None };
                let j = i + 9 + head[i + 9..].find('"').unwrap();
                Some(format!("{}{v}{}", &xml[..at + i + 9], &xml[at + j..]))
            };
            let (Some(a), Some(b)) = (set("1.5"), set("1.6")) else { continue };
            let opts = sr_model::LoadOptions { verify_assets: false, base_dir: f.parent().map(Into::into) };
            let codes = |x: &str| {
                let mut c: Vec<String> =
                    sr_model::validate_str(x, &opts).diagnostics.iter().map(|d| d.code.clone()).collect();
                c.sort();
                c
            };
            assert_eq!(codes(&a), codes(&b), "{}", f.display());
            compared += 1;
        }
    }
    assert!(compared > 100, "{compared} documents compared");
}
