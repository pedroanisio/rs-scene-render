//! `resolve` end to end: the example provider (built with this crate) for the
//! protocol, a local HTTP server standing in for OpenAI, and the real local
//! providers when they are installed.

use std::io::{BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};

use sr_resolve::{resolve, Options, Resolution, Status};

const ZERO: &str = "0000000000000000000000000000000000000000000000000000000000000000";

fn setup() {
    std::env::set_var("SR_PROVIDER_EXAMPLE", env!("CARGO_BIN_EXE_scene-render-provider-example"));
    std::env::set_var("SR_EXAMPLE_LOG", "1");
}

/// A fresh folder with `scene.scene.xml`.
fn project(name: &str, xml: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("sr-resolve-test-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    std::fs::write(d.join("scene.scene.xml"), xml).unwrap();
    d
}

fn opts(dir: &Path) -> Options {
    Options { store: Some(dir.join("store")), ..Default::default() }
}

fn calls(dir: &Path) -> usize {
    std::fs::read_to_string(dir.join(".example-calls")).map(|s| s.lines().count()).unwrap_or(0)
}

fn status(rows: &[Resolution], id: &str) -> Status {
    rows.iter().find(|r| r.id == id).unwrap_or_else(|| panic!("{id} missing: {rows:?}")).status
}

fn narrated(prompt: &str) -> String {
    format!(
        r#"<scene version="1.2">
  <!-- narrated title: keep this comment -->
  <project width="320" height="180" fps="10" duration="6"/>
  <assets>
    <generated id="vo" kind="speech" provider="example" model="m" prompt="{prompt}"
               cache="gen/vo.wav" cacheSha256="{ZERO}"/>
  </assets>
  <composition/>
  <audioMix sampleRate="48000"><audioTrack id="voice" asset="vo" start="2"/></audioMix>
  <captions>
    <captionTrack id="subs" language="en" transcribe="voice" provider="example" prompt="Hello"
                  cache="gen/subs.json" cacheSha256="{ZERO}"/>
  </captions>
</scene>
"#
    )
}

#[test]
fn test_resolve_preserves_edits_made_during_generation() {
    let xml = narrated("hello").replace("provider=\"example\"", "provider=\"editprobe\"");
    let d = project("concurrent-edit", &xml);
    let provider = d.join("provider.py");
    std::fs::write(
        &provider,
        r#"import json, sys, pathlib
r = json.load(sys.stdin)
p = pathlib.Path(r['baseDir']) / 'scene.scene.xml'
p.write_text(p.read_text() + '\n<!-- concurrent edit -->\n')
pathlib.Path(r['output']).write_bytes(b'generated data')
print(json.dumps({'ok': True}))
"#,
    )
    .unwrap();
    std::env::set_var("SR_PROVIDER_EDITPROBE", format!("python3 {}", provider.display()));
    let result = resolve(&d.join("scene.scene.xml"), &Options { only: vec!["vo".into()], ..opts(&d) });
    std::env::remove_var("SR_PROVIDER_EDITPROBE");
    assert!(result.as_ref().is_err_and(|e| e.contains("changed")), "{result:?}");
    assert_eq!(
        std::fs::read_to_string(d.join("scene.scene.xml")).unwrap(),
        format!("{xml}\n<!-- concurrent edit -->\n")
    );
}

#[test]
fn speech_and_its_captions_resolve_and_pin() {
    setup();
    let d = project("pin", &narrated("hello there world"));
    let doc = d.join("scene.scene.xml");
    // the placeholder digests fail validation
    assert!(sr_model::load_file(&doc, &sr_model::LoadOptions::default()).is_err());
    let rows = resolve(&doc, &opts(&d)).unwrap();
    assert_eq!((status(&rows, "vo"), status(&rows, "subs")), (Status::Made, Status::Made), "{rows:?}");
    // the document now validates, and only the digests changed
    let text = std::fs::read_to_string(&doc).unwrap();
    assert!(sr_model::load_file(&doc, &sr_model::LoadOptions::default()).is_ok());
    let strip = |s: &str| {
        s.split("cacheSha256=")
            .map(|p| p.split_once('"').map_or(p, |x| x.1).split_once('"').map_or(p, |x| x.1).to_string())
            .collect::<String>()
    };
    assert_eq!(strip(&text), strip(&narrated("hello there world")));
    // the transcriber heard the voice where it plays: from 2 s, three words of 0.3 s
    let t: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(d.join("gen/subs.json")).unwrap()).unwrap();
    let w = &t["segments"][0]["words"][0];
    assert!(
        (w["start"].as_f64().unwrap() - 2.0).abs() < 0.011 && (w["end"].as_f64().unwrap() - 2.9).abs() < 0.011,
        "{t}"
    );
    // sidecars record the request
    let side: serde_json::Value =
        serde_json::from_slice(&std::fs::read(d.join("gen/vo.wav.resolve.json")).unwrap()).unwrap();
    assert_eq!(side["provider"], "example");
    assert_eq!(side["version"], "example 1");
    // nothing is made twice
    assert_eq!(calls(&d), 2);
    let again = resolve(&doc, &opts(&d)).unwrap();
    assert!(again.iter().all(|r| r.status == Status::UpToDate), "{again:?}");
    assert_eq!(calls(&d), 2);
}

#[test]
fn an_outputs_own_voice_is_transcribed_in_output_time() {
    setup();
    // the output plays 3..6 s of the composition, and its own voice from output time 1 s
    let xml = format!(
        r#"<scene version="1.2">
  <project width="320" height="180" fps="10" duration="6"/>
  <output id="short" path="short.mp4" codec="h264">
    <segment from="3" to="6"/>
    <audioTrack id="short-vo" asset="vo" start="1"/>
    <captionTrack id="short-subs" language="en" transcribe="short-vo" provider="example" prompt="Hello"
                  cache="gen/short-subs.json" cacheSha256="{ZERO}"/>
  </output>
  <assets>
    <generated id="vo" kind="speech" provider="example" model="m" prompt="one two"
               cache="gen/vo.wav" cacheSha256="{ZERO}"/>
  </assets>
  <composition/>
</scene>
"#
    );
    let d = project("own", &xml);
    let doc = d.join("scene.scene.xml");
    let rows = resolve(&doc, &opts(&d)).unwrap();
    assert_eq!(status(&rows, "short-subs"), Status::Made, "{rows:?}");
    assert!(sr_model::load_file(&doc, &sr_model::LoadOptions::default()).is_ok());
    let t: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(d.join("gen/short-subs.json")).unwrap()).unwrap();
    let w = &t["segments"][0]["words"][0];
    assert!((w["start"].as_f64().unwrap() - 1.0).abs() < 0.011, "{t}");
}

#[test]
fn changes_remake_what_they_affect_and_check_changes_nothing() {
    setup();
    let d = project("change", &narrated("hello there world"));
    let doc = d.join("scene.scene.xml");
    resolve(&doc, &opts(&d)).unwrap();
    let text = std::fs::read_to_string(&doc).unwrap().replace("hello there world", "a longer line of speech");
    std::fs::write(&doc, &text).unwrap();
    // check: the speech is stale; the captions still match the audio on disk
    let check = resolve(&doc, &Options { check: true, ..opts(&d) }).unwrap();
    assert_eq!((status(&check, "vo"), status(&check, "subs")), (Status::Stale, Status::UpToDate));
    assert_eq!(std::fs::read_to_string(&doc).unwrap(), text, "check writes nothing");
    assert_eq!(calls(&d), 2);
    // resolving remakes the speech, and so the captions of the longer audio
    let rows = resolve(&doc, &opts(&d)).unwrap();
    assert_eq!((status(&rows, "vo"), status(&rows, "subs")), (Status::Made, Status::Made));
    let t: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(d.join("gen/subs.json")).unwrap()).unwrap();
    assert!((t["segments"][0]["end"].as_f64().unwrap() - 3.5).abs() < 0.011, "{t}");
    // a hand-edited digest is repinned without making anything
    let text = std::fs::read_to_string(&doc).unwrap();
    let sha = rows[0].sha256.clone().unwrap();
    std::fs::write(&doc, text.replace(&sha, ZERO)).unwrap();
    let rows = resolve(&doc, &opts(&d)).unwrap();
    assert_eq!(status(&rows, "vo"), Status::Pinned);
    assert!(std::fs::read_to_string(&doc).unwrap().contains(&sha));
    assert_eq!(calls(&d), 4);
}

#[test]
fn the_store_restores_results_without_the_provider() {
    setup();
    let d = project("store", &narrated("stored words"));
    let doc = d.join("scene.scene.xml");
    resolve(&doc, &opts(&d)).unwrap();
    let transcript = std::fs::read(d.join("gen/subs.json")).unwrap();
    std::fs::remove_dir_all(d.join("gen")).unwrap();
    let rows = resolve(&doc, &opts(&d)).unwrap();
    assert!(rows.iter().all(|r| r.status == Status::Restored), "{rows:?}");
    assert_eq!(std::fs::read(d.join("gen/subs.json")).unwrap(), transcript);
    assert_eq!(calls(&d), 2);
    // --force makes them again
    let rows = resolve(&doc, &Options { force: true, ..opts(&d) }).unwrap();
    assert!(rows.iter().all(|r| r.status == Status::Made), "{rows:?}");
    assert_eq!(calls(&d), 4);
    // --only limits the run
    let rows = resolve(&doc, &Options { force: true, only: vec!["subs".into()], ..opts(&d) }).unwrap();
    assert_eq!(rows.len(), 1);
    assert!(resolve(&doc, &Options { only: vec!["nope".into()], ..opts(&d) }).is_err());
}

#[test]
fn images_and_errors() {
    setup();
    let d = project(
        "images",
        &format!(
            r#"<scene version="1.2"><project width="64" height="64" fps="10" duration="1"/><assets>
  <generated id="pic" kind="image" provider="example" model="m" prompt="a red door" width="32" height="16" cache="pic.png" cacheSha256="{ZERO}"/>
  <generated id="cloudy" kind="speech" provider="openai" model="tts-1" prompt="hi" cache="c.wav" cacheSha256="{ZERO}"/>
  <generated id="odd" kind="speech" provider="no-such-provider" model="m" prompt="hi" cache="o.wav" cacheSha256="{ZERO}"/>
</assets><composition/></scene>"#
        ),
    );
    let rows = resolve(&d.join("scene.scene.xml"), &opts(&d)).unwrap();
    assert_eq!(status(&rows, "pic"), Status::Made);
    assert_eq!(image::open(d.join("pic.png")).unwrap().width(), 32);
    let cloudy = rows.iter().find(|r| r.id == "cloudy").unwrap();
    assert!(cloudy.status == Status::Error && cloudy.message.contains("--allow-cloud"), "{cloudy:?}");
    let odd = rows.iter().find(|r| r.id == "odd").unwrap();
    assert!(odd.status == Status::Error && odd.message.contains("scene-render-provider-no-such-provider"), "{odd:?}");
}

#[test]
fn test_store_invalid_entries_regenerate_without_pinning_corruption() {
    setup();
    for damage in ["bytes", "missing-sidecar", "malformed-sidecar", "wrong-key"] {
        let d = project(
            &format!("store-integrity-{damage}"),
            &format!(
                r#"<scene version="1.2"><project width="16" height="16" fps="1" duration="1"/>
          <assets><generated id="g" kind="image" provider="example" model="m" prompt="same image" width="8" height="8" cache="image.png" cacheSha256="{ZERO}"/></assets><composition/></scene>"#
            ),
        );
        let doc = d.join("scene.scene.xml");
        assert_eq!(status(&resolve(&doc, &opts(&d)).unwrap(), "g"), Status::Made);
        let original = std::fs::read(d.join("image.png")).unwrap();
        let pinned = std::fs::read_to_string(&doc).unwrap();
        let side: serde_json::Value =
            serde_json::from_slice(&std::fs::read(d.join("image.png.resolve.json")).unwrap()).unwrap();
        let stored = d.join("store").join(format!("{}.png", side["key"].as_str().unwrap()));
        let metadata = stored.with_file_name(format!("{}.resolve.json", stored.file_name().unwrap().to_string_lossy()));
        match damage {
            "bytes" => std::fs::write(&stored, b"corrupted cache").unwrap(),
            "missing-sidecar" => std::fs::remove_file(&metadata).unwrap(),
            "malformed-sidecar" => std::fs::write(&metadata, b"{").unwrap(),
            "wrong-key" => {
                let mut side = side;
                side["key"] = "wrong".into();
                std::fs::write(&metadata, serde_json::to_vec(&side).unwrap()).unwrap();
            }
            _ => unreachable!(),
        }
        std::fs::remove_file(d.join("image.png")).unwrap();
        let rows = resolve(&doc, &opts(&d)).unwrap();
        assert_eq!(status(&rows, "g"), Status::Made, "{damage}: {rows:?}");
        assert_eq!(std::fs::read(d.join("image.png")).unwrap(), original);
        assert_eq!(std::fs::read_to_string(&doc).unwrap(), pinned, "{damage}: changed pinned content");
        assert_eq!(calls(&d), 2);
        std::fs::remove_dir_all(d).unwrap();
    }
}

#[test]
fn the_store_keeps_output_formats_distinct() {
    setup();
    let d = project("store-formats", "");
    let doc = d.join("scene.scene.xml");
    for (ext, format) in [("png", image::ImageFormat::Png), ("jpg", image::ImageFormat::Jpeg)] {
        std::fs::write(&doc, format!(r#"<scene version="1.2"><project width="16" height="16" fps="1" duration="1"/>
          <assets><generated id="g" kind="image" provider="example" model="m" prompt="same image" width="16" height="16" cache="image.{ext}" cacheSha256="{ZERO}"/></assets><composition/></scene>"#)).unwrap();
        let rows = resolve(&doc, &opts(&d)).unwrap();
        assert_eq!(status(&rows, "g"), Status::Made, "{ext}: {rows:?}");
        let cache = d.join(format!("image.{ext}"));
        let bytes = std::fs::read(&cache).unwrap();
        assert_eq!(image::guess_format(&bytes).unwrap(), format);
        std::fs::remove_file(&cache).unwrap();
        let restored = resolve(&doc, &opts(&d)).unwrap();
        assert_eq!(status(&restored, "g"), Status::Restored);
        assert_eq!(std::fs::read(&cache).unwrap(), bytes);
    }
    assert_eq!(calls(&d), 2);
}

#[cfg(unix)]
#[test]
fn cache_symlinks_cannot_escape_the_project() {
    use std::os::unix::fs::symlink;
    setup();
    let d = project("symlink-cache", "");
    let outside = project("symlink-outside", "");
    let victim = outside.join("victim.png");
    std::fs::write(&victim, b"untouched").unwrap();
    symlink(&outside, d.join("linked")).unwrap();
    symlink(&victim, d.join("leaf.png")).unwrap();
    symlink(outside.join("absent.png"), d.join("dangling.png")).unwrap();
    let assets: String = [("parent", "linked/victim.png"), ("leaf", "leaf.png"), ("dangling", "dangling.png")]
        .iter().map(|(id, cache)| format!(r#"<generated id="{id}" kind="image" provider="example" model="m" prompt="test" width="8" height="8" cache="{cache}" cacheSha256="{ZERO}"/>"#)).collect();
    let doc = d.join("scene.scene.xml");
    std::fs::write(&doc, format!(r#"<scene version="1.2"><project width="16" height="16" fps="1" duration="1"/><assets>{assets}</assets><composition/></scene>"#)).unwrap();
    let rows = resolve(&doc, &opts(&d)).unwrap();
    assert_eq!(std::fs::read(&victim).unwrap(), b"untouched", "resolve overwrote an outside file");
    assert!(!outside.join("absent.png").exists());
    assert!(rows.iter().all(|r| r.status == Status::Error), "{rows:?}");
    assert_eq!(calls(&d), 0, "reject escaped paths before starting a provider");
}

#[cfg(unix)]
#[test]
fn temporary_symlinks_cannot_redirect_resolver_writes() {
    use std::os::unix::fs::symlink;
    setup();
    let d = project(
        "symlink-temporary",
        &format!(
            r#"<scene version="1.2"><project width="16" height="16" fps="1" duration="1"/>
      <assets><generated id="g" kind="image" provider="example" model="m" prompt="test" width="8" height="8" cache="image.png" cacheSha256="{ZERO}"/></assets><composition/></scene>"#
        ),
    );
    let outside = project("symlink-temporary-outside", "");
    for name in ["image.png.resolve.json.part", "scene.scene.xml.part"] {
        std::fs::write(outside.join(name), b"untouched").unwrap();
        symlink(outside.join(name), d.join(name)).unwrap();
    }
    let rows = resolve(&d.join("scene.scene.xml"), &opts(&d)).unwrap();
    assert_eq!(status(&rows, "g"), Status::Made, "{rows:?}");
    for name in ["image.png.resolve.json.part", "scene.scene.xml.part"] {
        assert_eq!(std::fs::read(outside.join(name)).unwrap(), b"untouched", "{name}");
    }
}

#[cfg(unix)]
#[test]
fn caches_support_project_and_internal_directory_symlinks() {
    use std::os::unix::fs::symlink;
    setup();
    let d = project(
        "symlink-safe",
        &format!(
            r#"<scene version="1.2"><project width="16" height="16" fps="1" duration="1"/>
      <assets><generated id="g" kind="image" provider="example" model="m" prompt="test" width="8" height="8" cache="linked/new/image.png" cacheSha256="{ZERO}"/></assets><composition/></scene>"#
        ),
    );
    std::fs::create_dir(d.join("real")).unwrap();
    symlink(d.join("real"), d.join("linked")).unwrap();
    let alias = project("symlink-safe-alias", "");
    symlink(&d, alias.join("project")).unwrap();
    let rows = resolve(&alias.join("project/scene.scene.xml"), &opts(&d)).unwrap();
    assert_eq!(status(&rows, "g"), Status::Made, "{rows:?}");
    assert!(d.join("real/new/image.png").is_file());
}

#[test]
fn caches_outside_the_project_are_refused() {
    setup();
    let outside = std::env::temp_dir().join(format!("sr-resolve-escape-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&outside);
    let gen = |id: &str, cache: &str| {
        format!(
            r#"<generated id="{id}" kind="image" provider="example" model="m" prompt="{id}" width="8" height="8" cache="{cache}" cacheSha256="{ZERO}"/>"#
        )
    };
    let name = outside.file_name().unwrap().to_string_lossy().into_owned();
    let escapes = [
        ("up", format!("../{name}/up.png")),
        ("abs", outside.join("abs.png").display().to_string()),
        ("uri", format!("file://{}", outside.join("uri.png").display())),
        ("back", format!("gen/../../{name}/back.png")),
    ];
    let mut assets: String = escapes.iter().map(|(id, c)| gen(id, c)).collect();
    assets += &gen("in", "gen/in.png");
    assets += &gen("round", "./gen/../round.png");
    let d = project(
        "escape",
        &format!(
            r#"<scene version="1.2"><project width="64" height="64" fps="10" duration="1"/><assets>{assets}</assets><composition/></scene>"#
        ),
    );
    let doc = d.join("scene.scene.xml");
    for check in [true, false] {
        let rows = resolve(&doc, &Options { check, ..opts(&d) }).unwrap();
        for (id, _) in &escapes {
            let r = rows.iter().find(|r| r.id == *id).unwrap();
            assert!(r.status == Status::Error && r.message.contains("outside"), "{r:?}");
        }
        assert_eq!(status(&rows, "in"), if check { Status::Stale } else { Status::Made });
        assert_eq!(status(&rows, "round"), if check { Status::Stale } else { Status::Made });
    }
    assert!(!outside.exists(), "nothing is written outside the project");
    assert!(d.join("gen/in.png").is_file() && d.join("round.png").is_file());
}

#[test]
fn a_view_over_the_tile_budget_is_refused_before_its_tiles_are_listed() {
    // the whole world at zoom 10 is a million tiles
    let d = project(
        "tile-budget",
        &format!(
            r#"<scene version="1.2"><project width="512" height="512" fps="2" duration="1"/><assets>
  <tiles id="osm" url="http://127.0.0.1:9/{{z}}/{{x}}/{{y}}.png" minZoom="10" maxZoom="10" cache="gen/osm.pmtiles" cacheSha256="{ZERO}" attribution="Test tiles"/>
  <map id="m" width="512" height="512" projection="web-mercator" centerLon="0" centerLat="0"><basemap tiles="osm"/></map>
</assets><composition><layer id="l" asset="m"/></composition></scene>"#
        ),
    );
    let started = std::time::Instant::now();
    let rows = resolve(&d.join("scene.scene.xml"), &Options { check: true, ..opts(&d) }).unwrap();
    assert!(rows[0].status == Status::Error && rows[0].message.contains("SR_TILES_MAX"), "{rows:?}");
    assert!(started.elapsed().as_secs() < 60);
}

/// How the test server answers: (path, headers, body) → (status, body).
type Respond = fn(&str, &str, &[u8]) -> (u16, Vec<u8>);
/// Requests the server saw: (path, headers, body).
type Seen = Vec<(String, String, Vec<u8>)>;

/// A one-shot HTTP server: answers each request with `respond(path, headers, body)`.
fn serve(n: usize, respond: Respond) -> (String, std::thread::JoinHandle<Seen>) {
    let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}/v1", l.local_addr().unwrap());
    let h = std::thread::spawn(move || {
        let mut seen = Vec::new();
        for s in l.incoming().take(n) {
            let mut s = s.unwrap();
            let mut r = BufReader::new(s.try_clone().unwrap());
            let mut line = String::new();
            r.read_line(&mut line).unwrap();
            let path = line.split_whitespace().nth(1).unwrap_or("").to_string();
            let mut headers = String::new();
            let mut len = 0;
            loop {
                let mut h = String::new();
                r.read_line(&mut h).unwrap();
                if h.trim().is_empty() {
                    break;
                }
                if let Some(v) = h.to_ascii_lowercase().strip_prefix("content-length:") {
                    len = v.trim().parse().unwrap();
                }
                headers.push_str(&h);
            }
            let mut body = vec![0; len];
            r.read_exact(&mut body).unwrap();
            let (code, out) = respond(&path, &headers, &body);
            write!(s, "HTTP/1.1 {code} X\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", out.len()).unwrap();
            s.write_all(&out).unwrap();
            seen.push((path, headers, body));
        }
        seen
    });
    (url, h)
}

#[test]
fn openai_speech_and_images_through_a_local_server() {
    fn respond(path: &str, headers: &str, _body: &[u8]) -> (u16, Vec<u8>) {
        if !headers.contains("Bearer test-key") {
            return (401, br#"{"error":{"message":"bad key"}}"#.to_vec());
        }
        if path.ends_with("/audio/speech") {
            // 0.1 s of silence, 16-bit mono 24 kHz
            let pcm = vec![0u8; 4800];
            let mut w = b"RIFF".to_vec();
            w.extend((36 + pcm.len() as u32).to_le_bytes());
            w.extend(b"WAVEfmt ");
            w.extend(16u32.to_le_bytes());
            w.extend([1, 0, 1, 0]);
            w.extend(24000u32.to_le_bytes());
            w.extend(48000u32.to_le_bytes());
            w.extend([2, 0, 16, 0]);
            w.extend(b"data");
            w.extend((pcm.len() as u32).to_le_bytes());
            w.extend(pcm);
            return (200, w);
        }
        // a 1 × 1 PNG
        let png = "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mP8z8BQDwAEhQGAhKmMIQAAAABJRU5ErkJggg==";
        (200, format!(r#"{{"data":[{{"b64_json":"{png}"}}]}}"#).into_bytes())
    }
    let (url, server) = serve(2, respond);
    std::env::set_var("OPENAI_BASE_URL", &url);
    std::env::set_var("OPENAI_API_KEY", "test-key");
    let d = project(
        "openai",
        &format!(
            r#"<scene version="1.2"><project width="64" height="64" fps="10" duration="1"/><assets>
  <generated id="say" kind="speech" provider="openai" model="gpt-4o-mini-tts" voice="coral" prompt="Welcome" cache="say.wav" cacheSha256="{ZERO}"/>
  <generated id="art" kind="image" provider="openai" model="gpt-image-1" prompt="a lighthouse" width="1024" height="1536" cache="art.png" cacheSha256="{ZERO}"/>
</assets><composition/></scene>"#
        ),
    );
    let rows = resolve(&d.join("scene.scene.xml"), &Options { allow_cloud: true, ..opts(&d) }).unwrap();
    assert!(rows.iter().all(|r| r.status == Status::Made), "{rows:?}");
    let seen = server.join().unwrap();
    let speech: serde_json::Value = serde_json::from_slice(&seen[0].2).unwrap();
    assert_eq!(
        (seen[0].0.as_str(), &speech["voice"], &speech["input"]),
        ("/v1/audio/speech", &serde_json::json!("coral"), &serde_json::json!("Welcome"))
    );
    let img: serde_json::Value = serde_json::from_slice(&seen[1].2).unwrap();
    assert_eq!((&img["size"], &img["model"]), (&serde_json::json!("1024x1536"), &serde_json::json!("gpt-image-1")));
    assert_eq!(image::open(d.join("art.png")).unwrap().width(), 1);
    assert!(sr_model::load_file(d.join("scene.scene.xml"), &sr_model::LoadOptions::default()).is_ok());
}

/// piper speaks a sentence; whisper.cpp writes its captions. Runs when both are installed
/// (`piper` with a voice in `SR_PIPER_VOICES`, `whisper-cli` with its models).
#[test]
fn piper_speech_captioned_by_whisper() {
    let voices = std::env::var_os("SR_PIPER_VOICES").map(PathBuf::from);
    let has_voice = voices.as_ref().is_some_and(|v| v.join("en_US-lessac-medium.onnx").is_file());
    let whisper = std::env::var_os("SR_WHISPER").map(PathBuf::from).filter(|p| p.is_file());
    if !has_voice || whisper.is_none() {
        eprintln!("skipped: needs SR_PIPER_VOICES with en_US-lessac-medium and SR_WHISPER");
        return;
    }
    let d = project(
        "piper-whisper",
        &format!(
            r#"<scene version="1.2"><project width="64" height="64" fps="10" duration="8"/><assets>
  <generated id="vo" kind="speech" provider="piper" model="en_US-lessac-medium" prompt="The quick brown fox jumps over the lazy dog." cache="vo.wav" cacheSha256="{ZERO}"/>
</assets><composition/>
<audioMix><audioTrack id="voice" asset="vo" start="1.5"/></audioMix>
<captions><captionTrack id="subs" language="en-US" transcribe="voice" model="base.en" cache="subs.json" cacheSha256="{ZERO}"/></captions>
</scene>"#
        ),
    );
    let rows = resolve(&d.join("scene.scene.xml"), &opts(&d)).unwrap();
    assert!(rows.iter().all(|r| r.status == Status::Made), "{rows:?}");
    let t: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(d.join("subs.json")).unwrap()).unwrap();
    let words: Vec<(String, f64)> = t["segments"]
        .as_array()
        .unwrap()
        .iter()
        .flat_map(|s| s["words"].as_array().unwrap().iter())
        .map(|w| (w["text"].as_str().unwrap().to_lowercase(), w["start"].as_f64().unwrap()))
        .collect();
    let text: String = words.iter().map(|w| w.0.as_str()).collect::<Vec<_>>().join(" ");
    assert!(text.contains("fox") && text.contains("dog"), "{text}");
    // composition time: nothing before the track starts, words in order
    assert!(words[0].1 >= 1.5, "{words:?}");
    assert!(words.windows(2).all(|w| w[1].1 >= w[0].1), "{words:?}");
    // the renderer reads the captions
    let doc = sr_model::load_file(d.join("scene.scene.xml"), &sr_model::LoadOptions::default()).unwrap();
    assert!(doc.scene.captions.is_some());
}

/// AudioForge renders a sting. Runs when `SR_AUDIOFORGE` names its program.
#[test]
fn audioforge_renders_a_cue() {
    let Some(exe) = std::env::var_os("SR_AUDIOFORGE").map(PathBuf::from).filter(|p| p.is_file()) else {
        eprintln!("skipped: needs SR_AUDIOFORGE");
        return;
    };
    let list =
        std::process::Command::new(&exe).args(["cue", "list", "--kind", "sound-effect", "--json"]).output().unwrap();
    let cues: serde_json::Value = serde_json::from_slice(&list.stdout).unwrap();
    let cue =
        cues.as_array().and_then(|a| a.first()).and_then(|c| c["id"].as_str()).expect("a sound-effect cue").to_string();
    let d = project(
        "audioforge",
        &format!(
            r#"<scene version="1.2"><project width="64" height="64" fps="10" duration="3"/><assets>
  <generated id="hit" kind="sound-effect" provider="audioforge" model="{cue}" prompt="" seed="3" cache="hit.wav" cacheSha256="{ZERO}"/>
</assets><composition/><audioMix sampleRate="44100" bitDepth="16"><audioTrack id="t" asset="hit"/></audioMix></scene>"#
        ),
    );
    let rows = resolve(&d.join("scene.scene.xml"), &opts(&d)).unwrap();
    assert_eq!(status(&rows, "hit"), Status::Made, "{rows:?}");
    let info = sr_media::probe(&d.join("hit.wav")).unwrap();
    assert_eq!(info.audio.first().map(|a| a.sample_rate), Some(44100));
}

#[test]
fn online_tiles_are_fetched_for_the_views_and_pinned() {
    // a tile service answering every z/x/y with a PNG of colour (60·z, 40·x, 40·y)
    let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}/tiles/{{z}}/{{x}}/{{y}}.png", l.local_addr().unwrap());
    let seen = std::sync::Arc::new(std::sync::Mutex::new(Vec::<(String, String)>::new()));
    let log = seen.clone();
    std::thread::spawn(move || {
        for s in l.incoming() {
            let mut s = s.unwrap();
            let mut r = BufReader::new(s.try_clone().unwrap());
            let mut line = String::new();
            r.read_line(&mut line).unwrap();
            let path = line.split_whitespace().nth(1).unwrap_or("").to_string();
            let mut headers = String::new();
            loop {
                let mut h = String::new();
                r.read_line(&mut h).unwrap();
                if h.trim().is_empty() {
                    break;
                }
                headers.push_str(&h);
            }
            let zxy: Vec<u32> = path
                .trim_start_matches("/tiles/")
                .trim_end_matches(".png")
                .split('/')
                .filter_map(|v| v.parse().ok())
                .collect();
            let mut png = Vec::new();
            image::RgbImage::from_pixel(
                256,
                256,
                image::Rgb([60 * zxy[0] as u8, 40 * zxy[1] as u8, 40 * zxy[2] as u8]),
            )
            .write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
            .unwrap();
            // Record before replying: resolve may finish as soon as the response body arrives.
            log.lock().unwrap().push((path, headers));
            write!(
                s,
                "HTTP/1.1 200 OK\r\nContent-Type: image/png\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                png.len()
            )
            .unwrap();
            s.write_all(&png).unwrap();
        }
    });
    let d = project(
        "tiles",
        &format!(
            r#"<scene version="1.2"><project width="512" height="512" fps="2" duration="1"/><assets>
  <tiles id="osm" url="{url}" cache="gen/osm.pmtiles" cacheSha256="{ZERO}" attribution="Test tiles"/>
  <map id="m" width="512" height="512" projection="web-mercator" centerLon="0" centerLat="0"><basemap tiles="osm"/></map>
</assets><composition><layer id="l" asset="m"/></composition></scene>"#
        ),
    );
    let doc = d.join("scene.scene.xml");
    // the network needs permission
    let refused = resolve(&doc, &opts(&d)).unwrap();
    assert!(refused[0].status == Status::Error && refused[0].message.contains("--allow-cloud"), "{refused:?}");
    assert!(seen.lock().unwrap().is_empty());
    let rows = resolve(&doc, &Options { allow_cloud: true, ..opts(&d) }).unwrap();
    assert_eq!(status(&rows, "osm"), Status::Made, "{rows:?}");
    // zoom 0 of a 512-pixel Web Mercator world shows the four 256-pixel tiles of z1
    let got: Vec<String> = seen.lock().unwrap().iter().map(|s| s.0.clone()).collect();
    assert_eq!(got, ["/tiles/1/0/0.png", "/tiles/1/0/1.png", "/tiles/1/1/0.png", "/tiles/1/1/1.png"]);
    assert!(seen.lock().unwrap()[0].1.contains("User-Agent: scene-render/"));
    // the archive holds them; the document validates
    let a = sr_geo::pmtiles::Archive::open(&d.join("gen/osm.pmtiles")).unwrap();
    assert!(a.tile(1, 1, 1).unwrap().is_some() && a.tile(2, 0, 0).unwrap().is_none());
    assert!(sr_model::load_file(&doc, &sr_model::LoadOptions::default()).is_ok());
    assert_eq!(resolve(&doc, &Options { allow_cloud: true, ..opts(&d) }).unwrap()[0].status, Status::UpToDate);
    assert_eq!(seen.lock().unwrap().len(), 4);
}

#[test]
fn destination_plan_preserves_document_and_inputs_before_generation() {
    setup();
    for (label, destination) in [
        ("document", "scene.scene.xml"),
        ("lock", "scene.scene.xml.resolve.lock"),
        ("input", "input.wav"),
        ("sidecar", "gen/vo.wav.resolve.json"),
        ("duplicate", "gen/vo.wav"),
    ] {
        let xml = narrated("hello").replace("gen/subs.json", destination);
        let xml = if label == "input" {
            xml.replace("<assets>", "<assets><audio id=\"original\" src=\"input.wav\"/>")
        } else {
            xml
        };
        sr_model::load_str(&xml, &sr_model::LoadOptions { verify_assets: false, ..Default::default() }).unwrap();
        let d = project(&format!("destination-{label}"), &xml);
        std::fs::write(d.join("input.wav"), b"protected input").unwrap();
        let result = resolve(&d.join("scene.scene.xml"), &opts(&d));
        assert!(result.is_err(), "{label}: {result:?}");
        assert_eq!(std::fs::read_to_string(d.join("scene.scene.xml")).unwrap(), xml, "{label}");
        assert_eq!(std::fs::read(d.join("input.wav")).unwrap(), b"protected input");
        assert_eq!(calls(&d), 0, "{label}: provider ran before validation");
        assert!(!d.join("gen/vo.wav").exists());
    }
}

#[test]
fn different_requests_cannot_share_a_cache_even_with_only_filter() {
    setup();
    let xml = format!(
        r#"<scene version="1.2"><project width="16" height="16" fps="10" duration="1"/>
    <assets><generated id="a" kind="image" provider="example" model="m" prompt="red" width="2" height="2" cache="shared.png" cacheSha256="{ZERO}"/>
    <generated id="b" kind="image" provider="example" model="m" prompt="green" width="2" height="2" cache="shared.png" cacheSha256="{ZERO}"/></assets><composition/></scene>"#
    );
    let d = project("cache-collision", &xml);
    std::fs::write(d.join("shared.png"), b"previous cache").unwrap();
    for only in [vec![], vec!["a".into()]] {
        let result = resolve(&d.join("scene.scene.xml"), &Options { only, ..opts(&d) });
        assert!(result.is_err(), "{result:?}");
        assert_eq!(std::fs::read(d.join("shared.png")).unwrap(), b"previous cache");
        assert_eq!(std::fs::read_to_string(d.join("scene.scene.xml")).unwrap(), xml);
        assert_eq!(calls(&d), 0);
    }
}

#[cfg(unix)]
#[test]
fn destination_plan_detects_document_symlink_aliases() {
    setup();
    let xml = narrated("hello").replace("gen/subs.json", "alias.json");
    let d = project("destination-alias", &xml);
    std::os::unix::fs::symlink(d.join("scene.scene.xml"), d.join("alias.json")).unwrap();
    assert!(resolve(&d.join("scene.scene.xml"), &opts(&d)).is_err());
    assert_eq!(std::fs::read_to_string(d.join("scene.scene.xml")).unwrap(), xml);
    assert_eq!(calls(&d), 0);
}
