use sr_vector::geom::{p, Xf};
use sr_vector::measure::{trim, TrimMode};
use sr_vector::scene::{Cmd, FillRule, Paint, Scene};
use sr_vector::stroke::{stroke, Cap, Join, Style};
use sr_vector::{shapes, tile};
use std::time::Instant;

fn main() {
    let n = 2000;
    let t0 = Instant::now();
    let mut scene = Scene::default();
    let mut outline_pts = 0;
    for k in 0..n {
        let (c, r) = (k % 50, k / 50);
        let path = match k % 4 {
            0 => shapes::ellipse(11.0, 11.0, 11.0, 11.0),
            1 => shapes::star(p(11.0, 11.0), 5, 11.0, 5.5, 0.0, 0.0, 0.0),
            2 => shapes::rect(0.0, 0.0, 22.0, 22.0, [0.0; 4]),
            _ => shapes::polygon(p(11.0, 11.0), 5, 11.0, 0.0, 0.0),
        };
        let polys = path.flatten(0.05);
        let tr = trim(&polys, 0.0, 0.7, 0.0, TrimMode::Simultaneous);
        let out = stroke(&tr, &Style { width: 3.0, cap: Cap::Round, join: Join::Round, miter_limit: 4.0 }, 0.05);
        outline_pts += out.iter().map(|q| q.pts.len()).sum::<usize>();
        let x = Xf::translate(20.0 + c as f64 * 38.0, 20.0 + r as f64 * 26.0).mul(&Xf::rotate(k as f64));
        scene.cmds.push(Cmd::Fill {
            polys: out,
            rule: FillRule::NonZero,
            paint: Paint::Solid { rgba: [1.0; 4], srgb: false },
            opacity: 1.0,
        });
        let last = scene.cmds.pop().unwrap();
        let mut one = Scene::default();
        one.cmds.push(last);
        scene.extend(one.transformed(&x));
    }
    let t1 = Instant::now();
    let e = tile::encode(&scene, [1920, 1080]);
    let t2 = Instant::now();
    println!(
        "outline {:.1} ms ({outline_pts} points), encode {:.1} ms ({} cmds, {} pieces)",
        (t1 - t0).as_secs_f64() * 1e3,
        (t2 - t1).as_secs_f64() * 1e3,
        e.cmds.len(),
        e.pieces.len()
    );
}
