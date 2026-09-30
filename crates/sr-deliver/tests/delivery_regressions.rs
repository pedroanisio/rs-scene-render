use std::path::PathBuf;

fn gpu() -> Option<sr_gpu::Gpu> {
    static GPU: std::sync::OnceLock<Option<sr_gpu::Gpu>> = std::sync::OnceLock::new();
    GPU.get_or_init(|| sr_gpu::Gpu::new().map_err(|e| eprintln!("skipping: {e}")).ok()).clone()
}

fn directory(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("sr-delivery-regression-{}-{name}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

#[test]
fn plain_output_posters_include_overlay_crop_and_output_clock() {
    let Some(gpu) = gpu() else { return };
    let dir = directory("poster");
    let xml = r##"<scene version="1.2"><project width="64" height="36" fps="10" duration="2" background="#000000"/>
      <output path="frame_%03d.png" codec="png-sequence" layout="square" overlay="tag" start="1" end="1.1" audio="false">
        <poster path="poster.png" format="png" time="1"/>
        <thumbnail path="thumbnail.png" format="png" time="1" width="18"/>
      </output>
      <layouts><layout id="square" width="36" height="36" reframe="crop" focusX="1"/></layouts>
      <symbols><symbol id="tag"><shape id="stamp" shape="rect" width="8" height="8" fill="#FFFFFF">
        <animate property="x"><key time="0" value="8"/><key time="1" value="24"/></animate>
      </shape></symbol></symbols>
      <composition><shape id="left" shape="rect" width="32" height="36" fill="#FF0000"/>
        <shape id="right" shape="rect" width="32" height="36" x="32" fill="#0000FF"/></composition></scene>"##;
    let path = dir.join("scene.xml");
    std::fs::write(&path, xml).unwrap();
    let doc = sr_model::load_file(path, &Default::default()).unwrap();
    sr_deliver::deliver(&doc, &doc.scene.outputs[0], Some(&gpu), &Default::default(), &mut |_, _| {}).unwrap();
    let frame = image::open(dir.join("frame_010.png")).unwrap().to_rgba8();
    let poster = image::open(dir.join("poster.png")).unwrap().to_rgba8();
    assert_eq!(poster.dimensions(), frame.dimensions());
    assert_eq!(frame.get_pixel(10, 2).0, [255; 4], "overlay starts at output time zero");
    // Each readback dithers independently; placement and colours agree within one level.
    assert!(
        poster.as_raw().iter().zip(frame.as_raw()).all(|(a, b)| a.abs_diff(*b) <= 1),
        "poster is the same delivered picture"
    );
    assert_eq!(image::open(dir.join("thumbnail.png")).unwrap().to_rgba8().dimensions(), (18, 18));
}

#[test]
fn overlay_uses_output_representation_and_cli_override_in_every_worker() {
    let Some(gpu) = gpu() else { return };
    for workers in [1, 2] {
        for cli in [false, true] {
            let dir = directory(&format!("proxy-{workers}-{cli}"));
            for (name, color) in
                [("original", [255, 0, 0, 255]), ("proxy", [0, 0, 255, 255]), ("preview", [0, 255, 0, 255])]
            {
                image::RgbaImage::from_pixel(4, 4, image::Rgba(color)).save(dir.join(format!("{name}.png"))).unwrap();
            }
            let xml = r##"<scene version="1.2"><project width="64" height="36" fps="10" duration="0.2" background="#000000"/>
              <output path="clip.mkv" codec="ffv1" overlay="tag" representation="proxy" audio="false"/>
              <assets><image id="im" src="original.png" width="4" height="4">
                <representation name="proxy" src="proxy.png" width="4" height="4"/>
                <representation name="preview" src="preview.png" width="4" height="4"/>
              </image></assets>
              <symbols><symbol id="tag"><layer id="over" asset="im"/></symbol></symbols>
              <composition><layer id="main" asset="im" x="32"/></composition></scene>"##;
            let path = dir.join("scene.xml");
            std::fs::write(&path, xml).unwrap();
            let doc = sr_model::load_file(path, &Default::default()).unwrap();
            let opts = sr_deliver::Options {
                representation: cli.then(|| "preview".into()),
                parallel: sr_deliver::Parallel::Count(workers),
                hardware: sr_media::encode::Hardware::Software,
                ..Default::default()
            };
            let report = sr_deliver::deliver(&doc, &doc.scene.outputs[0], Some(&gpu), &opts, &mut |_, _| {}).unwrap();
            assert_eq!(report.segments, workers);
            let decoded = std::process::Command::new(sr_media::ffmpeg())
                .args(["-v", "error", "-i"])
                .arg(report.path)
                .args(["-f", "rawvideo", "-pix_fmt", "rgb24", "-"])
                .output()
                .unwrap();
            assert!(decoded.status.success());
            assert_eq!(decoded.stdout.len(), 2 * 64 * 36 * 3);
            for frame in decoded.stdout.chunks_exact(64 * 36 * 3) {
                let want = if cli { [0, 255, 0] } else { [0, 0, 255] };
                for x in [2, 34] {
                    assert!(
                        frame[(2 * 64 + x) * 3..(2 * 64 + x) * 3 + 3]
                            .iter()
                            .zip(want)
                            .all(|(a, b)| (*a as i32 - b).abs() <= 4),
                        "representation at {x}, workers={workers}, cli={cli}"
                    );
                }
            }
        }
    }
}
