//! OUT1: an H.264 or H.265 output whose pixel format halves the chroma along an axis (4:2:0 both, 4:2:2 the width) needs
//! an even frame size along it. libx264 and libx265 refuse an odd one when the encoder starts, after the first frame has
//! been rendered ("width not divisible by 2", FFmpeg 7.1); validate catches it first and says how to fix it.

use sr_model::{validate_str, LoadOptions, Severity};

/// The codes of a document with `project` attributes, an `output` and `extra` sections between the outputs and the
/// composition, and the first OUT1 message.
fn check(project: &str, output: &str, extra: &str) -> (Vec<(String, Severity)>, String) {
    let xml = format!(
        r#"<scene version="1.1"><project {project} fps="30" duration="1"/>{output}{extra}<composition/></scene>"#
    );
    let r = validate_str(&xml, &LoadOptions { verify_assets: false, base_dir: None });
    let message = r.diagnostics.iter().find(|d| d.code == "OUT1").map(|d| d.message.clone()).unwrap_or_default();
    (r.diagnostics.into_iter().map(|d| (d.code, d.severity)).collect(), message)
}

fn out1(project: &str, output: &str, extra: &str) -> bool {
    let (codes, _) = check(project, output, extra);
    codes.iter().any(|(c, s)| c == "OUT1" && *s == Severity::Error)
}

const ODD: &str = r#"width="821" height="442""#;

#[test]
fn an_odd_h264_frame_in_420_is_an_error_that_says_how_to_fix_it() {
    // the reported case: an 821×442 project and an H.264 output with the default pixel format (yuv420p)
    let (codes, message) = check(ODD, r#"<output id="web" path="web.mp4" codec="h264"/>"#, "");
    assert_eq!(codes, [("OUT1".to_string(), Severity::Error)], "{codes:?}");
    for words in ["web", "821", "442", "yuv420p", "even", r#"width="822""#] {
        assert!(message.contains(words), "{words:?} not in {message:?}");
    }
    // an odd height alone, H.265, and the 10-bit and NV12 4:2:0 formats
    assert!(out1(r#"width="820" height="443""#, r#"<output path="a.mp4" codec="h264"/>"#, ""));
    assert!(out1(ODD, r#"<output path="a.mp4" codec="h265"/>"#, ""));
    for f in ["yuv420p10le", "nv12", "yuvj420p"] {
        assert!(out1(ODD, &format!(r#"<output path="a.mp4" codec="h264" pixelFormat="{f}"/>"#), ""), "{f}");
    }
}

#[test]
fn chroma_halved_across_only_needs_an_even_width() {
    let out = |f: &str| format!(r#"<output path="a.mp4" codec="h264" pixelFormat="{f}"/>"#);
    for f in ["yuv422p", "yuv422p10le"] {
        assert!(out1(r#"width="821" height="442""#, &out(f), ""), "{f}: odd width");
        assert!(!out1(r#"width="820" height="443""#, &out(f), ""), "{f}: odd height");
    }
    // 4:4:4 and RGB halve nothing
    for f in ["yuv444p", "yuv444p10le", "rgb24"] {
        assert!(!out1(r#"width="821" height="443""#, &out(f), ""), "{f}");
    }
}

#[test]
fn codecs_that_take_odd_sizes_are_left_alone() {
    // measured with FFmpeg 7.1: libvpx-vp9, ffv1, prores_ks, dnxhd, libwebp and libaom encode 257×145; SVT-AV1 does not,
    // and the encoder hands odd AV1 frames to libaom
    for codec in ["vp9", "ffv1", "prores", "dnxhr", "webp", "gif", "apng", "av1"] {
        let (codes, _) = check(r#"width="821" height="443""#, &format!(r#"<output path="a" codec="{codec}"/>"#), "");
        assert!(!codes.iter().any(|(c, _)| c == "OUT1"), "{codec}: {codes:?}");
    }
    assert!(!out1(r#"width="820" height="442""#, r#"<output path="a.mp4" codec="h264"/>"#, ""));
}

#[test]
fn the_size_is_the_outputs_else_its_layouts_else_the_projects() {
    // the output's own size wins over the project's, along each axis
    assert!(!out1(ODD, r#"<output path="a.mp4" codec="h264" width="822"/>"#, ""));
    assert!(out1(r#"width="820" height="442""#, r#"<output path="a.mp4" codec="h264" height="441"/>"#, ""));
    // a layout's frame is the delivered size, whatever the project's
    let layouts = r#"<layouts><layout id="sq" width="1081" height="1080"/><layout id="even" width="1080" height="1080"/></layouts>"#;
    assert!(out1(r#"width="1920" height="1080""#, r#"<output path="a.mp4" codec="h264" layout="sq"/>"#, layouts));
    assert!(!out1(ODD, r#"<output path="a.mp4" codec="h264" layout="even"/>"#, layouts));
    let (_, message) =
        check(r#"width="1920" height="1080""#, r#"<output path="a.mp4" codec="h264" layout="sq"/>"#, layouts);
    assert!(message.contains("sq"), "the message names the layout the size comes from: {message}");
}
