#[test]
fn caption_line_break_policy_has_a_compatible_default_and_checked_opt_in() {
    for (attrs, want) in [("", "greedy"), (r#"lineBreaks="greedy""#, "greedy"), (r#"lineBreaks="source""#, "source")] {
        let xml = format!(
            r#"<scene version="1.1"><project width="320" height="240" fps="24" duration="4"/><composition/><captions><captionTrack id="cc" language="en" {attrs}><cue start="0" end="4" text="a&#10;b"/></captionTrack></captions></scene>"#
        );
        let doc = sr_model::load_str(&xml, &sr_model::LoadOptions::without_assets()).unwrap();
        assert_eq!(doc.scene.captions.as_ref().unwrap().caption_tracks[0].line_breaks.as_str(), want);
        let bad = if attrs.is_empty() {
            xml.replace("language=\"en\"", "language=\"en\" lineBreaks=\"unknown\"")
        } else {
            xml.replace(attrs, "lineBreaks=\"unknown\"")
        };
        assert!(sr_model::load_str(&bad, &sr_model::LoadOptions::without_assets()).is_err());
    }
}
