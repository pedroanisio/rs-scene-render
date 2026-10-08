//! A smoke whose window follows its plume: the plume that an open domain cuts at its top is kept, nothing changes
//! where the window does not move, and the same frames come at any time requested, from a fresh evaluator and from
//! a baked sequence.

use sr_volume::{
    bake::{BakeLimits, BakedSequence},
    sequence::Interpolation,
};

/// A blob of smoke 4 units above the bottom face of a domain 60 units tall, in air that a field of force
/// accelerates upward (toward negative y; the force field is written with the sign of the document) at 6 units a second squared: in 5 s it rises 75 units, past the top of
/// the domain.
fn scene(follow: &str) -> String {
    format!(
        r#"<scene version="1.3"><project width="32" height="32" fps="10" duration="6"/>
        <composition><object3D id="cloud" primitive="volume">
          <pyro width="8" height="60" depth="8" voxelSize="0.5" dt="0.05" boundary="open" forceFields="lift" pressureIterations="400" pressureTolerance="1e-7" {follow}>
            <pyroImpulse shape="sphere" radius="1.5" y="26" time="0" density="1"/>
          </pyro><medium blackbody="true"/></object3D></composition>
        <physics pixelsPerMeter="1"><forceField id="lift" type="directional" forceY="6" affects="particles"/></physics></scene>"#
    )
}

fn evaluator(follow: &str) -> sr_eval::Evaluator {
    let doc = sr_model::load_str(&scene(follow), &sr_model::LoadOptions::without_assets()).unwrap();
    sr_eval::Evaluator::new(&doc, &Default::default()).unwrap_or_else(|e| panic!("{e}"))
}

fn at(ev: &sr_eval::Evaluator, time: f64) -> sr_eval::FrameGraph {
    let frame = ev.evaluate(time);
    assert!(frame.problems.is_empty() && frame.failures.is_empty(), "{:?} {:?}", frame.problems, frame.failures);
    frame
}

fn key(frame: &sr_eval::FrameGraph) -> u64 {
    frame.nodes.iter().find(|n| &*n.id == "cloud").unwrap().sim_volume.as_ref().expect("a native pyro volume").key
}

/// The highest smoke (the lowest y) on the axis of the cloud above the top face of the first window (y = -30), or none.
fn above_the_old_top(frame: &sr_eval::FrameGraph) -> Option<f64> {
    let volume = frame.nodes.iter().find(|n| &*n.id == "cloud").unwrap().sim_volume.as_ref().unwrap();
    let density = volume.data.grid("density").unwrap();
    (0..240).map(|i| -120.0 + 0.5 * i as f64).find(|&y| y < -30.0 && density.sample_world([0.0, y, 0.0]) > 1e-4)
}

const FOLLOW: &str = r#"follow="true" followMargin="4" followLoss="0.000001""#;

#[test]
fn the_plume_that_an_open_domain_cuts_at_its_top_goes_on_with_a_window_that_follows_it() {
    let (plain, following) = (evaluator(""), evaluator(FOLLOW));
    let (cut, kept) = (at(&plain, 5.0), at(&following, 5.0));
    assert_eq!(above_the_old_top(&cut), None, "the fixed domain holds nothing above its top");
    let head = above_the_old_top(&kept).expect("smoke above the old top of the domain");
    assert!(head < -35.0, "{head}");
}

#[test]
fn a_follow_that_is_false_or_absent_is_the_domain_it_always_was() {
    let (absent, falsy) = (evaluator(""), evaluator(r#"follow="false""#));
    for time in [1.0, 3.0] {
        assert_eq!(key(&at(&absent, time)), key(&at(&falsy, time)), "t = {time}");
    }
}

#[test]
fn any_order_of_times_and_a_fresh_evaluator_give_the_same_frames() {
    let first = evaluator(FOLLOW);
    let times = [5.0, 2.0, 4.0, 0.5, 5.0];
    let keys: Vec<u64> = times.iter().map(|&t| key(&at(&first, t))).collect();
    let fresh = evaluator(FOLLOW);
    for k in [3, 1, 4, 0, 2] {
        assert_eq!(keys[k], key(&at(&fresh, times[k])), "t = {}", times[k]);
    }
    assert_eq!(keys[0], keys[4]);
}

struct Temp(std::path::PathBuf);
impl Drop for Temp {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[test]
fn a_baked_sequence_of_a_following_plume_is_the_frames_it_was_baked_from() {
    let dir = Temp(std::env::temp_dir().join(format!("sr-eval-follow-bake-{}", std::process::id())));
    std::fs::create_dir(&dir.0).unwrap();
    let ev = evaluator(FOLLOW);
    let receipt = ev.bake_pyro("cloud", &dir.0.join("take"), 0, 51, BakeLimits::default()).unwrap();
    let baked = BakedSequence::open(&receipt.manifest, Some(&receipt.sha256), BakeLimits::default()).unwrap();
    for frame in [50, 30, 0, 44] {
        let live = ev.evaluate_frame(frame);
        assert!(live.problems.is_empty());
        let cached = baked.load(live.time, Interpolation::Hold, 1 << 22).unwrap();
        let native = live.nodes.iter().find_map(|n| n.sim_volume.as_ref()).expect("native volume");
        let (mut a, mut b) = (Vec::new(), Vec::new());
        native.data.write(&mut a).unwrap();
        cached.first.unwrap().write(&mut b).unwrap();
        assert_eq!(a, b, "frame {frame}");
    }
}
