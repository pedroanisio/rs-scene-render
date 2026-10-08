//! A simulation that is authored but cannot run (a resource limit, a solver error) is an error of
//! the frame, with its cause, in every command that renders: never a note that lets the frame
//! succeed without what was asked for.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

const HEAD: &str = r##"<scene version="1.3"><project width="64" height="64" fps="10" duration="2" background="#202030"/><materials><material id="m" baseColor="#FF8030" unlit="true"/></materials><composition><camera id="cam" x="0" y="-6" z="-30" fov="50"/>"##;

/// (name, body, the cause the message must name, a derived message that must not stand in for it)
fn cases() -> Vec<(&'static str, String, &'static str, Option<&'static str>)> {
    let scene = |body: &str, tail: &str| format!("{HEAD}{body}</composition>{tail}</scene>");
    vec![
        (
            "smoke",
            scene(
                r##"<object3D id="cloud" primitive="volume" y="-3"><pyro width="64" height="64" depth="64" voxelSize="1" dt="0.1" maxMemoryMiB="1"><pyroSource shape="sphere" radius="6" densityRate="5"/></pyro><medium extinction="0.2" albedo="#FFFFFF"/></object3D>"##,
                "",
            ),
            "pyro resource limit",
            Some("requires @volume"),
        ),
        (
            "ocean solver",
            scene(
                r##"<ocean id="sea" width="64" depth="64" cellSize="0.1" bottomDepth="2" dt="0.1" material="m" maxMemoryMiB="1"/>"##,
                "",
            ),
            "ocean grid exceeds memory",
            Some("produced no surface"),
        ),
        (
            "ocean surface",
            scene(
                r##"<ocean id="sea" width="64" depth="64" cellSize="0.5" bottomDepth="2" dt="0.1" material="m" surfaceMemoryMiB="1"/>"##,
                "",
            ),
            "ocean surface exceeds memory",
            Some("produced no surface"),
        ),
        (
            "particles",
            scene(
                r##"<particles3D id="p" material="m" rate="0" size="0.2" lifetime="2" dt="0.1" maxParticles="100000" maxMemoryMiB="1"><burst time="0" count="50000"/></particles3D>"##,
                "",
            ),
            "3D particle resource limit",
            None,
        ),
        (
            "crater at render time",
            scene(
                r##"<object3D id="ground" primitive="plane" width="40" height="40" segments="256" y="2" rotationX="-90" material="m"><crater radius="10" depth="3" rimHeight="1" rimWidth="2" start="0.1" end="0.5" maxMemoryMiB="1"/></object3D>"##,
                "",
            ),
            "crater draw vertices exceed memory budget",
            None,
        ),
        (
            "crater collider of a rigid body",
            scene(
                r##"<object3D id="ground" primitive="plane" segments="64" material="m"><crater maxMemoryMiB="1"/><rigidBody type="static"/></object3D>"##,
                "",
            ),
            "crater collider exceeds memory budget",
            None,
        ),
        (
            "fracture",
            scene(
                r##"<object3D id="solid" primitive="sphere" radius="4" segments="64" material="m"><rigidBody mass="4" collidesWith="none"/><fracture at="0.5" pieces="16" seed="1" interiorMaterial="m" maxMemoryMiB="1"/></object3D>"##,
                r##"<physics gravityY="0" pixelsPerMeter="1" fixedStep="0.01"/>"##,
            ),
            "fracture primitive exceeds budget",
            None,
        ),
    ]
}

fn run(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_scene-render")).args(args).env("NO_COLOR", "1").output().expect("binary runs")
}

fn scratch(name: &str) -> PathBuf {
    let dir =
        std::env::temp_dir().join(format!("sr-solver-failures-{}-{}", std::process::id(), name.replace(' ', "-")));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn written(dir: &Path) -> Vec<PathBuf> {
    std::fs::read_dir(dir)
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| p.extension().is_some_and(|e| e == "png"))
        .collect()
}

#[test]
fn a_solver_that_cannot_run_fails_every_render_command_and_names_its_cause() {
    for (name, xml, cause, derived) in cases() {
        let dir = scratch(name);
        let scene = dir.join("scene.xml");
        std::fs::write(&scene, xml).unwrap();
        let scene = scene.to_str().unwrap();
        let png = dir.join("out.png");
        let pattern = dir.join("seq_%04d.png");
        let commands: Vec<(&str, Vec<&str>)> = vec![
            ("render", vec!["render", scene, "--time", "1.0", "-o", png.to_str().unwrap()]),
            ("render --strict", vec!["render", scene, "--time", "1.0", "-o", png.to_str().unwrap(), "--strict"]),
            ("render --bench", vec!["render", scene, "--bench", "--frames", "10..11"]),
            ("encode", vec!["encode", scene, "-o", pattern.to_str().unwrap(), "--start", "1.0", "--end", "1.1"]),
        ];
        for (label, args) in commands {
            let o = run(&args);
            let stderr = String::from_utf8_lossy(&o.stderr);
            let out = String::from_utf8_lossy(&o.stdout);
            let all = format!("{stderr}\n{out}");
            assert_eq!(o.status.code(), Some(1), "{name}: {label} must fail:\n{all}");
            assert!(all.contains(cause), "{name}: {label} must name the cause {cause:?}:\n{all}");
            if let Some(derived) = derived {
                assert!(!all.contains(derived), "{name}: {label} reports the derived {derived:?}:\n{all}");
            }
            assert!(written(&dir).is_empty(), "{name}: {label} left a partial image: {:?}", written(&dir));
        }
        std::fs::remove_dir_all(&dir).ok();
    }
}

#[test]
fn a_failure_found_when_the_world_is_built_is_reported_at_every_time() {
    // the fracture is released at 0.5 s, but its budget is checked when the world is built
    let (_, xml, _, _) = cases().into_iter().find(|c| c.0 == "fracture").unwrap();
    let dir = scratch("early");
    let scene = dir.join("scene.xml");
    std::fs::write(&scene, xml).unwrap();
    let png = dir.join("out.png");
    let o = run(&["render", scene.to_str().unwrap(), "--time", "0.0", "-o", png.to_str().unwrap()]);
    let all = format!("{}\n{}", String::from_utf8_lossy(&o.stderr), String::from_utf8_lossy(&o.stdout));
    assert_eq!(o.status.code(), Some(1), "{all}");
    assert!(all.contains("fracture primitive exceeds budget"), "{all}");
    std::fs::remove_dir_all(&dir).ok();
}
