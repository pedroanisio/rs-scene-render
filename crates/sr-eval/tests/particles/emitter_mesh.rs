//! A particle emitter on a mesh is born on the surface of the mesh, where the emitter's own transform puts that
//! surface in the world. The mesh is the imported one: a file in metres, y up, is 100 scene units to the metre and
//! turned half a turn about x, as for every mesh of the scene, so a file that measures its ring in scene units
//! has to be scaled by 0.01 on the emitter. The particles here have no speed and no force on them, so where one is
//! at any time is where it was born.

use glam::{DMat4, DVec3};

const FLAT: &str = include_str!("../fixtures/ring-flat.obj");
const CLOSED: &str = include_str!("../fixtures/ring-closed.obj");

struct Dir(std::path::PathBuf);
impl Dir {
    fn new(name: &str) -> Dir {
        static NEXT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let id = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!("emitter-mesh-{name}-{}-{id}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        Dir(dir)
    }
}
impl Drop for Dir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// The triangles of an OBJ file as the importer places them in scene units, in single precision as it does.
fn triangles(obj: &str) -> Vec<[DVec3; 3]> {
    let points: Vec<DVec3> = obj
        .lines()
        .filter_map(|l| l.strip_prefix("v "))
        .map(|l| {
            let c: Vec<f32> = l.split_whitespace().map(|x| x.parse().unwrap()).collect();
            sr_3d::Y_UP_METRES.transform_point3(glam::Vec3::new(c[0], c[1], c[2])).as_dvec3()
        })
        .collect();
    obj.lines()
        .filter_map(|l| l.strip_prefix("f "))
        .map(|l| {
            let i: Vec<usize> = l.split_whitespace().map(|x| x.split('/').next().unwrap().parse().unwrap()).collect();
            [points[i[0] - 1], points[i[1] - 1], points[i[2] - 1]]
        })
        .collect()
}

/// Distance from `p` to the triangle `abc`.
fn distance(p: DVec3, [a, b, c]: [DVec3; 3]) -> f64 {
    // Ericson, Real-Time Collision Detection, closest point on a triangle
    let (ab, ac, ap) = (b - a, c - a, p - a);
    let (d1, d2) = (ab.dot(ap), ac.dot(ap));
    let closest = 'found: {
        if d1 <= 0. && d2 <= 0. {
            break 'found a;
        }
        let bp = p - b;
        let (d3, d4) = (ab.dot(bp), ac.dot(bp));
        if d3 >= 0. && d4 <= d3 {
            break 'found b;
        }
        let vc = d1 * d4 - d3 * d2;
        if vc <= 0. && d1 >= 0. && d3 <= 0. {
            break 'found a + ab * (d1 / (d1 - d3));
        }
        let cp = p - c;
        let (d5, d6) = (ab.dot(cp), ac.dot(cp));
        if d6 >= 0. && d5 <= d6 {
            break 'found c;
        }
        let vb = d5 * d2 - d1 * d6;
        if vb <= 0. && d2 >= 0. && d6 <= 0. {
            break 'found a + ac * (d2 / (d2 - d6));
        }
        let va = d3 * d6 - d5 * d4;
        if va <= 0. && d4 - d3 >= 0. && d5 - d6 >= 0. {
            break 'found b + (c - b) * ((d4 - d3) / ((d4 - d3) + (d5 - d6)));
        }
        let denominator = 1. / (va + vb + vc);
        a + ab * (vb * denominator) + ac * (vc * denominator)
    };
    (p - closest).length()
}

/// The emitter's own transform, T(x, y, z) · Rz · Ry · Rx · S, as the document defines it.
fn transform(x: [f64; 3], rotation: [f64; 3], scale: [f64; 3]) -> DMat4 {
    DMat4::from_translation(DVec3::from(x))
        * DMat4::from_rotation_z(rotation[2].to_radians())
        * DMat4::from_rotation_y(rotation[1].to_radians())
        * DMat4::from_rotation_x(rotation[0].to_radians())
        * DMat4::from_scale(DVec3::from(scale))
}

/// Positions of the particles of an emitter that sits on the mesh `file` with `attributes`, at 2 s.
fn born(file: &str, obj: &str, attributes: &str) -> Vec<DVec3> {
    let dir = Dir::new(file);
    std::fs::write(dir.0.join(file), obj).unwrap();
    let xml = format!(
        r#"<scene version="1.3"><project width="64" height="64" fps="10" duration="4"/>
        <assets><mesh id="ring" src="{file}"/></assets>
        <composition><particles3D id="dust" emitterShape="mesh" emitterMesh="ring" rate="60" lifetime="5" dt="0.05" seed="7" {attributes}/></composition></scene>"#
    );
    let doc = sr_model::load_str(&xml, &sr_model::LoadOptions { verify_assets: true, base_dir: Some(dir.0.clone()) })
        .unwrap_or_else(|e| panic!("{e}"));
    let ev = sr_eval::Evaluator::new(&doc, &Default::default()).unwrap_or_else(|e| panic!("{e}"));
    let frame = ev.evaluate(2.0);
    assert!(frame.problems.is_empty() && frame.failures.is_empty(), "{:?} {:?}", frame.problems, frame.failures);
    let particles = &frame.nodes[0].particles3d.as_ref().expect("particles").frame.particles;
    assert!(particles.len() > 100, "{} particles", particles.len());
    particles.iter().map(|p| DVec3::from(p.position)).collect()
}

/// The largest distance of a particle from the surface of `obj` placed by `world`.
fn worst(obj: &str, world: DMat4, particles: &[DVec3]) -> f64 {
    let placed: Vec<[DVec3; 3]> = triangles(obj).into_iter().map(|t| t.map(|v| world.transform_point3(v))).collect();
    particles.iter().map(|&p| placed.iter().map(|&t| distance(p, t)).fold(f64::INFINITY, f64::min)).fold(0., f64::max)
}

#[test]
fn a_mesh_emitter_with_no_transform_is_born_on_the_mesh() {
    for (file, obj) in [("ring-flat.obj", FLAT), ("ring-closed.obj", CLOSED)] {
        let particles = born(file, obj, "");
        let far = worst(obj, DMat4::IDENTITY, &particles);
        assert!(far <= 1e-6, "{file}: a particle is {far} from the surface");
    }
}

#[test]
fn a_mesh_emitter_is_born_on_the_mesh_the_transform_of_the_emitter_places() {
    let cases = [
        ("moved", r#"x="3" y="-4" z="5""#, transform([3., -4., 5.], [0.; 3], [1.; 3])),
        ("turned", r#"rotationX="90""#, transform([0.; 3], [90., 0., 0.], [1.; 3])),
        (
            "turned all ways",
            r#"rotation="20" rotationY="-35" rotationX="70""#,
            transform([0.; 3], [70., -35., 20.], [1.; 3]),
        ),
        ("scaled", r#"scaleX="2" scaleY="0.5" scaleZ="3""#, transform([0.; 3], [0.; 3], [2., 0.5, 3.])),
        (
            "all of them",
            r#"x="-2" y="7" z="1" rotation="15" rotationY="40" rotationX="-60" scaleX="1.5" scaleY="1.5" scaleZ="0.2""#,
            transform([-2., 7., 1.], [-60., 40., 15.], [1.5, 1.5, 0.2]),
        ),
    ];
    for (file, obj) in [("ring-flat.obj", FLAT), ("ring-closed.obj", CLOSED)] {
        for (name, attributes, world) in cases {
            let particles = born(file, obj, attributes);
            let far = worst(obj, world, &particles);
            assert!(far <= 1e-6, "{file}, {name}: a particle is {far} from the surface");
        }
    }
}

#[test]
fn a_ring_measured_in_scene_units_lands_where_it_is_measured_with_the_emitter_scaled_by_a_hundredth() {
    for (file, obj) in [("ring-flat.obj", FLAT), ("ring-closed.obj", CLOSED)] {
        let particles = born(file, obj, r#"scaleX="0.01" scaleY="0.01" scaleZ="0.01""#);
        // the ring has 96 straight sides: its inner edge is the chords between vertices at 6
        let inner = 6. * (std::f64::consts::PI / 96.).cos();
        for p in &particles {
            let radius = p.x.hypot(p.y);
            assert!((inner - 1e-5..=11. + 1e-5).contains(&radius), "{file}: a particle at radius {radius}");
            let thickness = if file == "ring-flat.obj" { 0. } else { 0.15 + 1e-5 };
            assert!(p.z.abs() <= thickness, "{file}: a particle {} off the plane of the ring", p.z);
        }
    }
}
