//! `scene-render captions`: the caption pages after paging and the current word per time, as JSON (SREP 52).

use std::path::PathBuf;
use std::process::{Command, Output};

fn scene(name: &str, xml: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("sr-captions-dump-{name}-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("scene.xml");
    std::fs::write(&file, xml).unwrap();
    file
}

fn run(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_scene-render")).args(args).env("NO_COLOR", "1").output().expect("binary runs")
}

const DOC: &str = r#"<scene version="1.2"><project width="640" height="360" fps="24" duration="4"/><composition/><captions>
  <captionTrack id="src" language="en" preset="highlight" lineBreaks="source" maxCharsPerLine="80" maxLines="2"><cue start="0" end="4" text="aa bb&#10;cc dd"/></captionTrack>
  <captionTrack id="grd" language="en" preset="highlight" maxCharsPerLine="80" maxLines="2"><cue start="0" end="4" text="aa bb&#10;cc dd"/></captionTrack>
</captions></scene>"#;

#[test]
fn captions_prints_pages_and_the_current_word_as_json() {
    let file = scene("ok", DOC);
    let f = file.to_str().unwrap();
    let o = run(&["captions", f, "--at", "0.5,1.5", "--at", "3.5"]);
    assert_eq!(o.status.code(), Some(0), "{}", String::from_utf8_lossy(&o.stderr));
    let v: serde_json::Value = serde_json::from_slice(&o.stdout)
        .unwrap_or_else(|e| panic!("stdout is not JSON ({e}): {}", String::from_utf8_lossy(&o.stdout)));
    let tracks = v["tracks"].as_array().unwrap();
    assert_eq!(tracks.len(), 2);
    assert_eq!(tracks[0]["id"], "src");
    assert_eq!(tracks[0]["pages"][0]["text"], "aa bb\ncc dd");
    assert_eq!(tracks[1]["pages"][0]["text"], "aa bb cc dd");
    for t in tracks {
        let words: Vec<_> =
            t["at"].as_array().unwrap().iter().map(|s| (s["time"].as_f64(), s["word"].as_u64())).collect();
        assert_eq!(words, [(Some(0.5), Some(0)), (Some(1.5), Some(1)), (Some(3.5), Some(3))]);
    }
    // deterministic: the same document gives the same bytes
    assert_eq!(run(&["captions", f, "--at", "0.5,1.5", "--at", "3.5"]).stdout, o.stdout);
}

#[test]
fn captions_takes_parameters_like_eval() {
    let file = scene(
        "param",
        r#"<scene version="1.2"><project width="640" height="360" fps="24" duration="4"/>
  <parameters><param id="line" type="string" default="aa"/></parameters><composition/><captions>
  <captionTrack id="c" language="en"><cue start="0" end="4"><word start="0" end="4" text="{{line}}"/></cue></captionTrack></captions></scene>"#,
    );
    let o = run(&["captions", file.to_str().unwrap(), "--param", "line=xx"]);
    assert_eq!(o.status.code(), Some(0), "{}", String::from_utf8_lossy(&o.stderr));
    let v: serde_json::Value = serde_json::from_slice(&o.stdout).unwrap();
    assert_eq!(v["tracks"][0]["pages"][0]["text"], "xx", "{v}");
}

#[test]
fn an_unreadable_track_is_reported_and_fails_the_command() {
    let file = scene(
        "gone",
        r#"<scene version="1.2"><project width="64" height="64" fps="24" duration="1"/><composition/><captions>
  <captionTrack id="gone" language="en" src="no-such-file.srt"/></captions></scene>"#,
    );
    let o = run(&["captions", file.to_str().unwrap(), "--no-assets"]);
    assert_eq!(o.status.code(), Some(1), "{}", String::from_utf8_lossy(&o.stderr));
    let v: serde_json::Value = serde_json::from_slice(&o.stdout).unwrap();
    assert!(v["tracks"][0]["error"].as_str().is_some_and(|e| e.contains("no-such-file.srt")), "{v}");
    assert!(String::from_utf8_lossy(&o.stderr).contains("gone"), "the error is also on stderr");
}

#[test]
fn an_invalid_document_or_a_missing_file_fails_without_json() {
    let bad = scene("bad", r#"<scene version="1.2"><project width="64" height="64" fps="24"/></scene>"#);
    let o = run(&["captions", bad.to_str().unwrap()]);
    assert_eq!(o.status.code(), Some(1));
    assert!(o.stdout.is_empty(), "{}", String::from_utf8_lossy(&o.stdout));
    let o = run(&["captions", "/no/such/dir/scene.xml"]);
    assert_eq!(o.status.code(), Some(2));
    let o = run(&["captions", bad.to_str().unwrap(), "--at", "nan"]);
    assert_eq!(o.status.code(), Some(2), "a time must be a finite number");
}
