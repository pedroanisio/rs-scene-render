//! Caller inputs reach output overlays in serial and parallel delivery.
//! Kept in a separate binary so its worker devices do not race other GPU tests.

mod common;
#[test]
fn overlays_use_caller_parameters_and_data_rows() {
    let Some(gpu) = common::gpu() else { return };
    for workers in [1, 2] {
        for cli in [false, true] {
            let d = std::env::temp_dir().join(format!("sr-overlay-{}-{workers}-{cli}", std::process::id()));
            std::fs::create_dir_all(&d).unwrap();
            let xml = r##"<scene version="1.2"><project width="64" height="36" fps="10" duration="0.2" background="#000000"/>
              <parameters><param id="pos" type="number" required="true"/><data id="rows" format="json">[{"pos":40}]</data></parameters>
              <output path="v.mkv" codec="ffv1" overlay="tag" audio="false"/>
              <symbols><symbol id="tag"><shape id="w" shape="rect" width="8" height="8" y="0" fill="#FFFFFF"><expression property="x">param('pos')</expression></shape></symbol></symbols>
              <composition/></scene>"##;
            let path = d.join("scene.xml");
            std::fs::write(&path, xml).unwrap();
            let doc = sr_model::load_file(path, &Default::default()).unwrap();
            let opts = sr_deliver::Options {
                parallel: sr_deliver::Parallel::Count(workers),
                params: if cli { vec![("pos".into(), "24".into())] } else { Vec::new() },
                row: Some((Some("rows".into()), 0)),
                hardware: sr_media::encode::Hardware::Software,
                ..Default::default()
            };
            let r = sr_deliver::deliver(&doc, &doc.scene.outputs[0], Some(&gpu), &opts, &mut |_, _| {}).unwrap();
            assert_eq!(r.segments, workers);
            let decoded = std::process::Command::new(sr_media::ffmpeg())
                .args(["-v", "error", "-i"])
                .arg(&r.path)
                .args(["-f", "rawvideo", "-pix_fmt", "rgb24", "-"])
                .output()
                .unwrap();
            assert!(decoded.status.success());
            assert_eq!(decoded.stdout.len(), 2 * 64 * 36 * 3);
            for frame in decoded.stdout.as_chunks::<{ 64 * 36 * 3 }>().0 {
                let x = if cli { 26 } else { 42 };
                assert!(frame[(2 * 64 + x) * 3] > 250, "overlay uses the selected input");
                assert!(frame[(2 * 64 + 2) * 3] < 3, "overlay is not drawn at the default position");
            }
        }
    }
}
