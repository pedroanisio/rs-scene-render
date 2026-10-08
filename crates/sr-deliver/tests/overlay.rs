//! Caller inputs reach output overlays in serial and parallel delivery.
//! Kept in a separate binary so its worker devices do not race other GPU tests.

mod common;

#[test]
fn output_overlays_enforce_safe_areas_on_their_own_clock() {
    let Some(gpu) = common::gpu() else { return };
    for (name, level, force, rejects) in
        [("error", "error", "", true), ("warn", "warn", "", false), ("force", "error", "safeAreaForce=\"true\"", false)]
    {
        let d = std::env::temp_dir().join(format!("sr-overlay-safe-{}-{name}", std::process::id()));
        std::fs::create_dir_all(&d).unwrap();
        let xml = format!(
            r##"<scene version="1.3"><project width="64" height="36" fps="10" duration="2" safeArea="sa"/>
        <output path="{}/f_%03d.png" codec="png-sequence" start="1" end="1.2" overlay="tag"/>
        <safeAreas><safeArea id="sa" top="0" right="0.25" bottom="0" left="0" enforce="{level}"/></safeAreas>
        <symbols><symbol id="tag" {force}><shape id="logo" shape="rect" x="50" y="10" width="10" height="10" end="0.2" tags="logo" fill="#FFFFFF"/></symbol></symbols>
        <composition/></scene>"##,
            d.display()
        );
        let doc = sr_model::load_str(&xml, &sr_model::LoadOptions::without_assets()).unwrap();
        let result = sr_deliver::deliver(&doc, &doc.scene.outputs[0], Some(&gpu), &Default::default(), &mut |_, _| {});
        if rejects {
            match result {
                Err(sr_deliver::DeliverError::Document(r)) => {
                    assert!(r.diagnostics.iter().any(|d| d.code == "SA01" && d.is_error()))
                }
                other => panic!("expected overlay safe-area rejection, got {other:?}"),
            }
            assert!(std::fs::read_dir(&d).unwrap().next().is_none(), "reject before writing frames");
        } else {
            let report = result.unwrap();
            assert_eq!(report.warnings.iter().any(|w| w.contains("SA01")), level == "warn");
        }
        std::fs::remove_dir_all(d).unwrap();
    }
}

#[test]
fn safe_area_audit_checks_only_selected_output_captions() {
    let Some(gpu) = common::gpu() else { return };
    for (selected, mode, rejects) in [("good", "burn", false), ("bad", "sidecar", true), ("bad", "burn", true)] {
        let d = std::env::temp_dir().join(format!("sr-overlay-captions-{}-{selected}-{mode}", std::process::id()));
        std::fs::create_dir_all(&d).unwrap();
        let xml = format!(
            r##"<scene version="1.3"><project width="96" height="64" fps="10" duration="0.2" safeArea="sa"/>
        <styles><textStyle id="words" font="DejaVu Sans" size="12" color="#FFFFFF"/></styles>
        <output path="{}/f_%03d.png" codec="png-sequence" burnCaptions="{selected}">
          <captionTrack id="good" language="en" style="words" y="20%"><cue start="0" end="0.2" text="Good"/></captionTrack>
          <captionTrack id="bad" language="en" style="words" y="80%" mode="{mode}"><cue start="0" end="0.2" text="Bad"/></captionTrack>
        </output><safeAreas><safeArea id="sa" top="0" right="0" bottom="0.5" left="0" enforce="error"/></safeAreas><composition/></scene>"##,
            d.display()
        );
        let doc = sr_model::load_str(&xml, &sr_model::LoadOptions::without_assets()).unwrap();
        let result = sr_deliver::deliver(&doc, &doc.scene.outputs[0], Some(&gpu), &Default::default(), &mut |_, _| {});
        if rejects {
            match result {
                Err(sr_deliver::DeliverError::Document(r)) => {
                    assert!(r.diagnostics.iter().any(|d| d.code == "SA01" && d.is_error()))
                }
                other => panic!("expected caption safe-area rejection, got {other:?}"),
            }
        } else {
            assert!(result.is_ok(), "unselected captions aren't visible: {result:?}");
        }
        std::fs::remove_dir_all(d).unwrap();
    }
}

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
