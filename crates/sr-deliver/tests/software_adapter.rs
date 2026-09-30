//! Delivery on a software adapter: the report says so, and the automatic worker count is one
//! (each more worker would be one more llvmpipe device competing for the same cores), while an
//! explicit count is honoured. It runs in its own test binary because workers open devices.

use std::path::PathBuf;

use sr_gpu::gpu::GpuOptions;

fn software() -> Option<sr_gpu::Gpu> {
    sr_gpu::Gpu::with_options(&GpuOptions { adapter: Some("llvmpipe".into()), ..GpuOptions::default() })
        .map_err(|e| eprintln!("skipping: no llvmpipe adapter ({e})"))
        .ok()
}

fn dir(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("sr-software-adapter-{}-{name}", std::process::id()));
    std::fs::create_dir_all(&d).unwrap();
    d
}

/// A 40 s programme at 1 fps: long enough for two automatic segments of at least 20 s.
fn programme(name: &str) -> sr_model::Document {
    let d = dir(name);
    let xml = format!(
        r##"<scene version="1.2"><project width="32" height="18" fps="1" duration="40" background="#000000"/>
          <output path="{}/v.mkv" codec="ffv1" audio="false"/>
          <composition><shape id="s" shape="rect" width="8" height="8" fill="#FFFFFF">
            <animate property="x"><key time="0" value="0"/><key time="40" value="24"/></animate></shape></composition></scene>"##,
        d.display()
    );
    let path = d.join("scene.xml");
    std::fs::write(&path, xml).unwrap();
    sr_model::load_file(&path, &sr_model::LoadOptions::default()).unwrap_or_else(|e| panic!("{e:?}"))
}

#[test]
fn software_rendering_is_reported_and_runs_one_automatic_worker() {
    let Some(gpu) = software() else { return };
    let doc = programme("auto");
    let opts = sr_deliver::Options { parallel: sr_deliver::Parallel::Auto, ..Default::default() };
    let r = sr_deliver::deliver(&doc, &doc.scene.outputs[0], Some(&gpu), &opts, &mut |_, _| {}).unwrap();
    assert_eq!(r.segments, 1, "a software adapter renders with one automatic worker");
    // the adapter is a fact about the machine, not a finding about the document
    let a = r.render_adapter.as_ref().expect("a video output names its adapter");
    assert!(a.software && a.name == gpu.info.name, "{a:?}");
    assert!(r.warnings.is_empty(), "document warnings only: {:?}", r.warnings);
}

#[test]
fn workers_open_the_adapter_the_caller_chose() {
    let Some(gpu) = software() else { return };
    let twin = gpu.open_like().expect("the same adapter opens again");
    assert_eq!((twin.info.name.as_str(), twin.info.backend), (gpu.info.name.as_str(), gpu.info.backend));
}

#[test]
fn an_explicit_worker_count_is_honoured_on_a_software_adapter() {
    let Some(gpu) = software() else { return };
    let doc = programme("explicit");
    let opts = sr_deliver::Options { parallel: sr_deliver::Parallel::Count(2), ..Default::default() };
    let r = sr_deliver::deliver(&doc, &doc.scene.outputs[0], Some(&gpu), &opts, &mut |_, _| {}).unwrap();
    assert_eq!(r.segments, 2);
}
