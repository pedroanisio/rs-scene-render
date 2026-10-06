//! With the GPU debug layers on, a delivery renders with one worker whatever was asked for, and says why.
//! One test per binary: it sets a process-wide environment variable.

use sr_gpu::gpu::GpuOptions;

fn programme() -> (sr_model::Document, std::path::PathBuf) {
    let d = std::env::temp_dir().join(format!("sr-debug-serial-{}", std::process::id()));
    std::fs::create_dir_all(&d).unwrap();
    let xml = format!(
        r##"<scene version="1.2"><project width="32" height="18" fps="1" duration="40" background="#000000"/>
          <output path="{}/v.mkv" codec="ffv1" audio="false"/>
          <composition><shape id="s" shape="rect" width="8" height="8" fill="#FFFFFF">
            <animate property="x"><key time="0" value="0"/><key time="40" value="24"/></animate></shape></composition></scene>"##,
        d.display()
    );
    let path = d.join("scene.xml");
    std::fs::write(&path, xml).unwrap();
    (sr_model::load_file(&path, &sr_model::LoadOptions::default()).unwrap_or_else(|e| panic!("{e:?}")), d)
}

#[test]
fn debug_layers_force_one_worker_and_the_report_says_why() {
    let Ok(gpu) = sr_gpu::Gpu::with_options(&GpuOptions { adapter: Some("llvmpipe".into()), ..GpuOptions::default() })
    else {
        eprintln!("skipping: no llvmpipe adapter");
        return;
    };
    let (doc, _dir) = programme();
    let opts = sr_deliver::Options { parallel: sr_deliver::Parallel::Count(2), ..Default::default() };
    let run =
        |gpu: &sr_gpu::Gpu| sr_deliver::deliver(&doc, &doc.scene.outputs[0], Some(gpu), &opts, &mut |_, _| {}).unwrap();
    std::env::remove_var("SR_GPU_DEBUG");
    let plain = run(&gpu);
    assert_eq!(plain.segments, 2, "two workers were asked for");
    assert!(plain.serial_because.is_none());
    std::env::set_var("SR_GPU_DEBUG", "1");
    let debug = run(&gpu);
    assert_eq!(debug.segments, 1, "debug layers: one worker");
    let why = debug.serial_because.expect("the report says why");
    assert!(why.contains("SR_GPU_DEBUG") && why.contains("1 worker instead of 2"), "{why}");
}
