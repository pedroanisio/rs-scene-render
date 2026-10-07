//! Output shaders read the delivered programme in output time, also in workers.
mod common;

#[test]
fn overlay_shaders_read_audio_in_serial_and_parallel_delivery() {
    let Some(gpu) = common::gpu() else { return };
    let d = std::env::temp_dir().join(format!("sr-overlay-audio-{}", std::process::id()));
    std::fs::create_dir_all(&d).unwrap();
    let samples: Vec<f32> = (0..19200).map(|i| if i < 9600 { 0.0 } else { 0.5 }).collect();
    sr_audio::wav::write(&d.join("tone.wav"), &vec![samples], 48000, 16, sr_audio::Layout::Mono, false).unwrap();
    std::fs::write(
        d.join("audio.fs"),
        r#"/*{"INPUTS":[{"NAME":"inputImage","TYPE":"image"},{"NAME":"signal","TYPE":"audio","MAX":1}]}*/
void main() { float level = IMG_NORM_PIXEL(signal, vec2(0.5)).r; gl_FragColor = vec4(level, 0.0, 0.0, 1.0); }"#,
    )
    .unwrap();
    // Skip the silent first half. The same shader must see the tone both in composition
    // time and in the overlay's clock, which starts at zero for this trimmed output.
    {
        let render = |overlay: bool, workers: u32, mode: &str| {
            let shape = r##"<shape id="visual" shape="rect" width="32" height="32" fill="#FFFFFF" effects="sound"/>"##;
            let xml = format!(
                r##"<scene version="1.3"><project width="32" height="32" fps="10" duration="0.4" background="#000000"/>
            <output path="v.mkv" codec="ffv1" audio="false" {range} {attr}>{children}</output>
            <assets><audio id="tone" src="tone.wav"/></assets>
            <symbols><symbol id="tag">{over}</symbol></symbols><composition>{body}</composition>
            <effects><effect id="sound" type="shader" src="audio.fs" space="raw"/></effects>
            {mix}</scene>"##,
                range = match mode {
                    "trim" => "start=\"0.2\"",
                    "own" => "end=\"0.2\"",
                    _ => "",
                },
                children = match mode {
                    "segments" => "<segment from=\"0.2\" to=\"0.4\"/>",
                    "own" => "<audioTrack id=\"own\" asset=\"tone\" clipIn=\"0.2\"/>",
                    _ => "",
                },
                mix = if mode == "own" {
                    ""
                } else {
                    "<audioMix sampleRate=\"48000\"><audioTrack id=\"music\" asset=\"tone\"/></audioMix>"
                },
                attr = if overlay { "overlay=\"tag\"" } else { "" },
                over = if overlay { shape } else { "" },
                body = if overlay { "" } else { shape }
            );
            let path = d.join("scene.xml");
            std::fs::write(&path, xml).unwrap();
            let doc = sr_model::load_file(path, &Default::default()).unwrap();
            let opts = sr_deliver::Options {
                parallel: sr_deliver::Parallel::Count(workers),
                upload: false,
                hardware: sr_media::encode::Hardware::Software,
                ..Default::default()
            };
            let report = sr_deliver::deliver(&doc, &doc.scene.outputs[0], Some(&gpu), &opts, &mut |_, _| {}).unwrap();
            assert_eq!(report.segments, workers);
            assert!(report.unsupported.is_empty(), "{:?}", report.unsupported);
            let decoded = std::process::Command::new(sr_media::ffmpeg())
                .args(["-v", "error", "-i"])
                .arg(&report.path)
                .args(["-f", "rawvideo", "-pix_fmt", "rgb24", "-"])
                .output()
                .unwrap();
            assert!(decoded.status.success());
            assert_eq!(decoded.stdout.len(), 2 * 32 * 32 * 3);
            decoded.stdout
        };
        let expected = render(false, 1, "trim");
        assert!(expected[32 * 32 * 3 + (16 * 32 + 16) * 3] > 210, "reference reads the tone");
        for mode in ["trim", "segments", "own"] {
            let serial = render(true, 1, mode);
            // Segments and output-owned audio start numbering at zero, unlike the trim.
            // Output dithering therefore differs; compare average red per frame.
            for (actual, reference) in serial.chunks_exact(32 * 32 * 3).zip(expected.chunks_exact(32 * 32 * 3)) {
                let mean = |frame: &[u8]| frame.iter().step_by(3).map(|v| *v as f64).sum::<f64>() / 1024.0;
                assert!(
                    (mean(actual) - mean(reference)).abs() < 1.0,
                    "{mode}: {} vs {}",
                    mean(actual),
                    mean(reference)
                );
            }
            assert!(render(true, 2, mode) == serial, "serial and parallel {mode} differ");
        }
    }
    std::fs::remove_dir_all(d).unwrap();
}
