//! SREP 17: the `pdf` asset, shapes on its regions, and rules V9, C66–C69, R51, R52.

const ZERO: &str = "0000000000000000000000000000000000000000000000000000000000000000";

fn doc(version: &str, pdf_attrs: &str, body: &str) -> String {
    format!(
        r#"<scene version="{version}"><project width="640" height="360" fps="24" duration="1"/><assets><image id="img" src="i.png" width="4" height="4"/><pdf id="paper" src="p.pdf" sha256="{ZERO}" cache="p.png" cacheSha256="{ZERO}" width="200" height="100"{pdf_attrs}><region id="g" x="20" y="10" width="60" height="20"/><region id="t" text="Abstract" occurrence="2" x="1" y="2" width="3" height="4"/></pdf><pdf id="other" src="q.pdf" sha256="{ZERO}" cache="q.png" cacheSha256="{ZERO}" width="10" height="10"><region id="og" x="0" y="0" width="1" height="1"/></pdf></assets><composition><layer id="L" asset="paper"/><layer id="O" asset="other"/><layer id="I" asset="img"/>{body}</composition></scene>"#
    )
}

fn codes_of(xml: &str) -> Vec<String> {
    match sr_model::load_str(xml, &sr_model::LoadOptions::without_assets()) {
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

fn codes(body: &str) -> Vec<String> {
    codes_of(&doc("1.2", "", body))
}

#[test]
fn valid_documents() {
    for body in [
        "",
        r#"<shape id="s" shape="rect" region="g" regionLayer="L"/>"#,
        r#"<shape id="s" shape="ellipse" region="g" regionLayer="L" regionPadding="5" width="1" height="1" rotation="3"/>"#,
        r#"<shape id="s" shape="rect" width="10" height="10"/>"#,
        r#"<group id="gr" x="5"><shape id="s" shape="rect" region="t" regionLayer="L"/></group>"#,
    ] {
        assert_eq!(codes(body), Vec::<String>::new(), "{body}");
    }
    assert!(codes_of(&doc("1.2", r##" page="3" dpi="300" background="#00000000" annotations="true""##, "")).is_empty());
}

#[test]
fn v9_pdf_assets_need_version_1_2() {
    assert_eq!(codes_of(&doc("1.1", "", "")), ["V9"]);
    assert!(codes_of(&doc("1.3", "", "")).is_empty());
}

#[test]
fn c66_the_source_is_pinned() {
    let xml = doc("1.2", "", "").replacen(&format!(r#" sha256="{ZERO}""#), "", 1);
    assert_eq!(codes_of(&xml), ["C66"]);
}

#[test]
fn region_shape_rules() {
    // without a layer, R52 cannot hold either (its XPath compares against no layer)
    assert_eq!(codes(r#"<shape id="s" shape="rect" region="g"/>"#), ["C67", "R52"]);
    assert_eq!(codes(r#"<shape id="s" shape="rect" region="g" regionLayer="L" parent="L"/>"#), ["C68"]);
    assert_eq!(
        codes(r#"<shape id="s" shape="rect" region="g" regionLayer="L"><transformConstraint type="copy-position" target="L"/></shape>"#),
        ["C68"]
    );
    // a region that is not one of a pdf's
    assert_eq!(codes(r#"<shape id="s" shape="rect" region="L" regionLayer="L"/>"#), ["R51", "R52"]);
    // a layer that shows another pdf, or an image
    assert_eq!(codes(r#"<shape id="s" shape="rect" region="g" regionLayer="O"/>"#), ["R52"]);
    assert_eq!(codes(r#"<shape id="s" shape="rect" region="g" regionLayer="I"/>"#), ["R52"]);
}

#[test]
fn c69_width_and_height_unless_on_a_region() {
    assert_eq!(codes(r#"<shape id="s" shape="rect"/>"#), ["C69"]);
    assert_eq!(codes(r#"<shape id="s" shape="rect" width="10"/>"#), ["C69"]);
    assert!(codes(r#"<shape id="s" shape="rect" region="g" regionLayer="L"/>"#).is_empty());
}

#[test]
fn structure() {
    for (attrs, body) in [
        (r#" dpi="17""#, ""),
        (r#" dpi="1201""#, ""),
        (r#" page="0""#, ""),
        (r#" annotations="maybe""#, ""),
    ] {
        let c = codes_of(&doc("1.2", attrs, body));
        assert!(!c.is_empty() && c.iter().all(|c| c.starts_with('S')), "{attrs} {body}: {c:?}");
    }
}

#[test]
fn a_dangling_region_is_a_dangling_idref_and_names_no_region() {
    let c = codes(r#"<shape id="s" shape="rect" region="nosuch" regionLayer="L"/>"#);
    assert_eq!(c, ["R51", "R52", "S10"]);
}

#[test]
fn the_model_reads_pdf_assets() {
    let d = sr_model::load_str(&doc("1.2", "", ""), &sr_model::LoadOptions::without_assets()).unwrap();
    let Some(sr_model::model::AssetsChild::Pdf(p)) = d.asset("paper") else { panic!("no pdf asset") };
    assert_eq!((p.page, p.dpi, p.width, p.height), (1, 150.0, 200, 100));
    assert!(!p.annotations);
    assert_eq!(p.regions.len(), 2);
    assert_eq!(p.regions[1].text.as_deref(), Some("Abstract"));
    assert_eq!(p.regions[1].occurrence, 2);
}

/// The renderer reads the cache, never the PDF: a missing source is not an error at render time, and a cache
/// whose SHA-256 differs from cacheSha256 is (A02).
#[test]
fn assets_check_the_cache_not_the_source() {
    let dir = std::env::temp_dir().join(format!("sr-model-pdf-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let png = {
        let img = image::RgbaImage::from_pixel(4, 2, image::Rgba([0, 0, 255, 255]));
        let mut b = Vec::new();
        img.write_to(&mut std::io::Cursor::new(&mut b), image::ImageFormat::Png).unwrap();
        b
    };
    std::fs::write(dir.join("p.png"), &png).unwrap();
    use sha2::Digest;
    let sha: String = sha2::Sha256::digest(&png).iter().map(|b| format!("{b:02x}")).collect();
    let xml = |cache_sha: &str| {
        format!(
            r#"<scene version="1.2"><project width="64" height="64" fps="24" duration="1"/><assets><pdf id="paper" src="absent.pdf" sha256="{ZERO}" cache="p.png" cacheSha256="{cache_sha}" width="4" height="2"/></assets><composition><layer id="L" asset="paper"/></composition></scene>"#
        )
    };
    let opts = sr_model::LoadOptions { verify_assets: true, base_dir: Some(dir.clone()) };
    assert!(sr_model::load_str(&xml(&sha), &opts).is_ok(), "{:?}", sr_model::load_str(&xml(&sha), &opts).err());
    match sr_model::load_str(&xml(ZERO), &opts) {
        Err(sr_model::LoadError::Invalid(r)) => assert!(r.diagnostics.iter().any(|d| d.code == "A02"), "{r}"),
        other => panic!("{:?}", other.map(|_| ())),
    }
}
