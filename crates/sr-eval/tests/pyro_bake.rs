use sr_volume::{
    bake::{BakeLimits, BakedSequence},
    sequence::Interpolation,
};
struct Temp(std::path::PathBuf);
impl Drop for Temp {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
fn evaluator() -> sr_eval::Evaluator {
    let xml = r#"<scene version="1.3"><project width="16" height="16" fps="10" duration="2"/>
      <composition><group id="g" start="0.2" timeScale="2"><object3D id="cloud" primitive="volume" start="0.4" animationSpeed="2">
      <pyro width="4" height="4" depth="4" voxelSize="1" dt="0.05"><pyroSource radius="1.5" densityRate="1" temperatureRate="100"/></pyro>
      </object3D></group></composition></scene>"#;
    let doc = sr_model::load_str(xml, &sr_model::LoadOptions::without_assets()).unwrap();
    sr_eval::Evaluator::new(&doc, &Default::default()).unwrap()
}
#[test]
fn native_bake_preserves_absolute_times_and_all_fields_after_a_backward_seek() {
    let dir = Temp(std::env::temp_dir().join(format!("sr-eval-bake-{}", std::process::id())));
    std::fs::create_dir(&dir.0).unwrap();
    let ev = evaluator();
    ev.evaluate(1.5);
    let receipt = ev.bake_pyro("cloud", &dir.0.join("take"), 0, 12, BakeLimits::default()).unwrap();
    let baked = BakedSequence::open(&receipt.manifest, Some(&receipt.sha256), BakeLimits::default()).unwrap();
    for frame in [11, 4, 0, 7] {
        let live = ev.evaluate_frame(frame);
        assert!(live.problems.is_empty());
        let cached = baked.load(live.time, Interpolation::Hold, 1 << 20).unwrap();
        let native = live.nodes.iter().find_map(|n| n.sim_volume.as_ref());
        match native {
            None => assert!(cached.first.is_none()),
            Some(n) => {
                let mut a = Vec::new();
                let mut b = Vec::new();
                n.data.write(&mut a).unwrap();
                cached.first.unwrap().write(&mut b).unwrap();
                assert_eq!(a, b);
            }
        }
    }
    assert!(ev.bake_pyro("missing", &dir.0.join("bad"), 0, 2, BakeLimits::default()).is_err());
    assert!(!dir.0.join("bad").exists());
    assert!(ev.bake_pyro("cloud", &dir.0.join("huge"), 0, 100001, BakeLimits::default()).is_err());
    assert!(!dir.0.join("huge").exists());
}

#[test]
fn bake_identity_distinguishes_the_pressure_solvers() {
    let receipt = |solver: &str| {
        let xml = format!(
            r#"<scene version="1.3"><project width="16" height="16" fps="10" duration="2"/>
          <composition><object3D id="cloud" primitive="volume">
          <pyro width="12" height="12" depth="12" voxelSize="1" dt="0.05" pressureTolerance="1e-9" pressureIterations="400" buoyancy="0.02" {solver}>
            <pyroSource radius="2" densityRate="1" temperatureRate="100" velocityRateY="-2"/></pyro>
          </object3D></composition></scene>"#
        );
        let doc = sr_model::load_str(&xml, &sr_model::LoadOptions::without_assets()).unwrap();
        let ev = sr_eval::Evaluator::new(&doc, &Default::default()).unwrap();
        let dir = Temp(std::env::temp_dir().join(format!(
            "sr-eval-bake-solver-{}-{}",
            std::process::id(),
            solver.replace(['"', '=', ' '], "")
        )));
        let _ = std::fs::remove_dir_all(&dir.0);
        std::fs::create_dir(&dir.0).unwrap();
        ev.bake_pyro("cloud", &dir.0.join("take"), 0, 8, BakeLimits::default()).unwrap().sha256
    };
    let (absent, jacobi, multigrid) = (receipt(""), receipt(r#"solver="jacobi""#), receipt(r#"solver="multigrid""#));
    assert_eq!(absent, jacobi);
    assert_ne!(jacobi, multigrid);
}

#[test]
fn bake_identity_distinguishes_the_advection_schemes() {
    let receipt = |extra: &str, name: &str| {
        let xml = format!(
            r#"<scene version="1.3"><project width="16" height="16" fps="10" duration="2"/>
          <composition><object3D id="cloud" primitive="volume">
          <pyro width="12" height="12" depth="12" voxelSize="1" dt="0.05" pressureTolerance="1e-9" pressureIterations="400" buoyancy="0.02" vorticity="1" {extra}>
            <pyroSource radius="2" densityRate="1" temperatureRate="100" velocityRateY="-2"/></pyro>
          </object3D></composition></scene>"#
        );
        let doc = sr_model::load_str(&xml, &sr_model::LoadOptions::without_assets()).unwrap();
        let ev = sr_eval::Evaluator::new(&doc, &Default::default()).unwrap();
        let dir = Temp(std::env::temp_dir().join(format!("sr-eval-bake-adv-{}-{name}", std::process::id())));
        let _ = std::fs::remove_dir_all(&dir.0);
        std::fs::create_dir(&dir.0).unwrap();
        ev.bake_pyro("cloud", &dir.0.join("take"), 0, 8, BakeLimits::default()).unwrap().sha256
    };
    let absent = receipt("", "absent");
    let semi = receipt(r#"advection="semilagrangian""#, "semi");
    let mac = receipt(r#"advection="maccormack""#, "mac");
    let both = receipt(r#"solver="multigrid" advection="maccormack""#, "both");
    assert_eq!(absent, semi);
    assert_ne!(semi, mac);
    assert_ne!(mac, both);
}
