//! Delivery at a quality tier: a draft renders at half size and the output stage scales it to
//! the output's size, so a draft delivery has the final's dimensions, and the report says so.

mod common;
use std::path::PathBuf;

fn gpu() -> Option<sr_gpu::Gpu> {
    common::gpu()
}

fn dir(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("sr-deliver-quality-{}-{name}", std::process::id()));
    std::fs::create_dir_all(&d).unwrap();
    d
}

#[test]
fn a_draft_delivery_keeps_the_output_size_and_reports_its_quality() {
    let Some(gpu) = gpu() else { return };
    let d = dir("draft");
    let xml = format!(
        r##"<scene version="1.2"><project width="96" height="64" fps="10" duration="0.3" background="#203040"/>
          <output path="{}/f_%03d.png" codec="png-sequence"/>
          <composition><shape id="s" shape="ellipse" x="20" y="10" width="50" height="40" fill="#FF8000"/></composition></scene>"##,
        d.display()
    );
    let path = d.join("scene.xml");
    std::fs::write(&path, xml).unwrap();
    let doc = sr_model::load_file(&path, &sr_model::LoadOptions::default()).unwrap_or_else(|e| panic!("{e:?}"));
    for (q, name) in [(None, "final"), (Some(sr_model::model::ProjectQuality::Draft), "draft")] {
        let opts = sr_deliver::Options { quality: q, ..Default::default() };
        let r = sr_deliver::deliver(&doc, &doc.scene.outputs[0], Some(&gpu), &opts, &mut |_, _| {}).unwrap();
        assert_eq!(r.quality, name);
        assert_eq!(r.size, [96, 64]);
        let img = image::open(d.join("f_000.png")).unwrap();
        assert_eq!((img.width(), img.height()), (96, 64), "{name}");
    }
}
