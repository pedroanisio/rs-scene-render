//! Corpus documents valid at 1.5 render the same bytes at 1.6, using small frames on software adapters.

use super::common;
use common::*;

fn with_version(xml: &str, v: &str) -> Option<String> {
    let at = xml.find("<scene")?;
    let head = &xml[at..at + xml[at..].find('>')?];
    let i = head.find("version=\"")? + 9;
    let j = i + head[i..].find('"')?;
    Some(format!("{}{v}{}", &xml[..at + i], &xml[at + j..]))
}

fn render(xml: &str, base: &std::path::Path, gpu: sr_gpu::Gpu) -> Option<Vec<[f32; 4]>> {
    let opts = sr_model::LoadOptions { verify_assets: false, base_dir: Some(base.to_path_buf()) };
    let mut d = sr_model::load_str(xml, &opts).ok()?;
    d.scene.project.width = d.scene.project.width.min(64);
    d.scene.project.height = d.scene.project.height.min(64);
    let ev = sr_eval::Evaluator::new(&d, &Default::default()).ok()?;
    let mut r = sr_gpu::Renderer::new(gpu, ev.program());
    let t = ev.program().duration * 0.5;
    let g = ev.evaluate(t);
    let mut sub = |t: f64| ev.evaluate(t);
    let f = r.render_with(&g, ev.program(), Some(&mut sub));
    (f.stats.errors.is_empty() && f.stats.unsupported.is_empty()).then(|| r.read(&f.texture))
}

#[test]
fn small_corpus_documents_render_alike_at_1_5_and_1_6() {
    if gpu().is_none() {
        return;
    }
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/corpus/valid");
    let mut files: Vec<_> = std::fs::read_dir(&root)
        .unwrap()
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.to_string_lossy().ends_with(".scene.xml"))
        .collect();
    files.sort();
    let mut compared = 0;
    for f in files {
        let xml = std::fs::read_to_string(&f).unwrap();
        // Bound frame dimensions after loading instead of selecting dimensions by XML spelling.
        // Most of the originally small documents contain volumes restricted to version 1.3.
        let (Some(a), Some(b)) = (with_version(&xml, "1.5"), with_version(&xml, "1.6")) else { continue };
        let Some(pa) = render(&a, &root, gpu().unwrap()) else { continue };
        let pb = render(&b, &root, gpu().unwrap()).expect("renders as 1.6 when it renders as 1.5");
        assert!(pa == pb, "{} differs at 1.6", f.display());
        eprintln!("compared {}", f.display());
        compared += 1;
    }
    assert!(compared >= 20, "{compared} documents compared");
}
