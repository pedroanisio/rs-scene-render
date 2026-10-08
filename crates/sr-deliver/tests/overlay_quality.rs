//! Quality overrides reach output layers, including independent parallel worker renderers.
//! Keep worker devices isolated from the other delivery test binaries.

mod common;

use sr_model::model::ProjectQuality;

#[test]
fn output_layers_match_the_authored_quality_in_serial_and_parallel_delivery() {
    let Some(gpu) = common::gpu() else { return };
    for captions in [false, true] {
        let dir = std::env::temp_dir().join(format!("sr-overlay-quality-{}-{captions}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let (overlay, track) = if captions {
            (
                "",
                r#"<captionTrack id="cc" language="en" preset="boxed-line" style="words" y="40%"><cue start="0" end="0.2" text="Hello"/></captionTrack>"#,
            )
        } else {
            (r#"overlay="tag""#, "")
        };
        let render = |quality: ProjectQuality, override_quality: Option<ProjectQuality>, workers: u32| {
            let xml = format!(
                r##"<scene version="1.3"><project width="96" height="64" fps="10" duration="0.2" background="#000000" quality="{quality}"/>
                <styles><textStyle id="words" font="DejaVu Sans" size="16" color="#FFFFFF"/></styles>
                <output path="v.mkv" codec="ffv1" audio="false" {overlay}>{track}</output>
                <symbols><symbol id="tag"><shape id="mark" shape="ellipse" x="7" y="5" width="41" height="31" fill="#FFFFFF"/></symbol></symbols>
                <composition/></scene>"##
            );
            let path = dir.join("scene.xml");
            std::fs::write(&path, xml).unwrap();
            let doc = sr_model::load_file(&path, &Default::default()).unwrap();
            let opts = sr_deliver::Options {
                quality: override_quality,
                parallel: sr_deliver::Parallel::Count(workers),
                hardware: sr_media::encode::Hardware::Software,
                upload: false,
                ..Default::default()
            };
            let report = sr_deliver::deliver(&doc, &doc.scene.outputs[0], Some(&gpu), &opts, &mut |_, _| {}).unwrap();
            assert_eq!(report.segments, workers);
            assert_eq!(report.quality, override_quality.unwrap_or(quality).as_str());
            assert!(report.unsupported.is_empty(), "{:?}", report.unsupported);
            let decoded = std::process::Command::new(sr_media::ffmpeg())
                .args(["-v", "error", "-i"])
                .arg(&report.path)
                .args(["-f", "rawvideo", "-pix_fmt", "rgb24", "-"])
                .output()
                .unwrap();
            assert!(decoded.status.success(), "{}", String::from_utf8_lossy(&decoded.stderr));
            assert_eq!(decoded.stdout.len(), 2 * 96 * 64 * 3);
            decoded.stdout
        };
        let final_pixels = render(ProjectQuality::Final, None, 1);
        let draft_pixels = render(ProjectQuality::Draft, None, 1);
        assert!(final_pixels != draft_pixels, "the fixture must distinguish the quality tiers");
        for workers in [1, 2] {
            assert!(
                render(ProjectQuality::Draft, Some(ProjectQuality::Final), workers) == final_pixels,
                "final override left a draft output layer: captions={captions}, workers={workers}"
            );
            assert!(
                render(ProjectQuality::Final, Some(ProjectQuality::Draft), workers) == draft_pixels,
                "draft override left a final output layer: captions={captions}, workers={workers}"
            );
        }
        std::fs::remove_dir_all(dir).unwrap();
    }
}
