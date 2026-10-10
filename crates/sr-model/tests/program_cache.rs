//! SREP 66, Determinism 6: a build program's output is cached by its key, and a damaged cache file is not trusted.
//! One test in its own binary, because the cache directory comes from the process environment.

use std::path::PathBuf;

use sr_model::{load_str, LoadOptions};

#[test]
fn outputs_are_cached_by_key_and_a_damaged_entry_is_recomputed() {
    let modules = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../sr-wasm/tests/modules");
    let dir = std::env::temp_dir().join(format!("sr-program-cache-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::env::set_var("SR_PROGRAM_CACHE", &dir);
    let sha: String = {
        use sha2::Digest;
        sha2::Sha256::digest(std::fs::read(modules.join("srep66-rect.wasm")).unwrap())
            .iter()
            .map(|x| format!("{x:02x}"))
            .collect()
    };
    let xml = format!(
        r#"<scene version="1.6"><project width="64" height="64" fps="24" duration="1"/><composition><program id="p" src="srep66-rect.wasm" sha256="{sha}"/></composition></scene>"#
    );
    let opts = LoadOptions { verify_assets: true, base_dir: Some(modules) };
    load_str(&xml, &opts).expect("first load");
    let key = sr_wasm::cache_key(&sha, &sr_wasm::Inputs::default(), sr_wasm::Limits::default());
    let entry = dir.join(&key);
    let stored = std::fs::read(&entry).expect("the output is cached under its key");
    assert!(String::from_utf8_lossy(&stored).contains(r#"<shape id="g1""#));

    // a cached entry is used as it is: change its output (with a matching digest) and the document follows it
    let other = br##"<shape id="g2" shape="rect" width="4" height="4" fill="#00FF00FF"/>"##;
    let digest: String = {
        use sha2::Digest;
        sha2::Sha256::digest(other).iter().map(|x| format!("{x:02x}")).collect()
    };
    std::fs::write(&entry, [digest.as_bytes(), b"\n", other].concat()).unwrap();
    let d = load_str(&xml, &opts).unwrap();
    assert!(d.node("g2").is_some() && d.node("g1").is_none());

    // a damaged entry (digest and output disagree) is ignored and recomputed
    std::fs::write(&entry, [digest.as_bytes(), b"\n", b"<shape id=\"g3\"/>"].concat()).unwrap();
    let d = load_str(&xml, &opts).unwrap();
    assert!(d.node("g1").is_some());
    let _ = std::fs::remove_dir_all(&dir);
}
