use sr_vector::arap::{Pin, PinKind, Puppet};
use sr_vector::deform::{self, Axis, Deformer};
use sr_vector::geom::{p, Xf};
use sr_vector::measure::{self, TrimMode};
use sr_vector::modifiers::{self, Item, Modifier};
use sr_vector::path::{poly_bounds, Path};
use sr_vector::rig::{self, Bone};
use sr_vector::scene::{Cmd, FillRule, MaskOp, MatteMode, Paint, Scene};
use sr_vector::stroke::{self, Cap, Join, Style};
use sr_vector::tile;
use sr_vector::{shapes, Poly};

fn area(ps: &[Poly]) -> f64 {
    ps.iter().map(|q| q.area()).sum::<f64>().abs()
}

/// Renders a scene with solid white paint and returns total coverage (sum of alpha).
fn coverage_sum(scene: &Scene, w: u32, h: u32) -> f64 {
    let e = tile::encode(scene, [w, h]);
    let px = tile::render_cpu(&e, &|_, _, _| [1.0, 1.0, 1.0, 1.0]);
    px.iter().map(|c| c[3] as f64).sum()
}

fn white() -> Paint {
    Paint::Solid { rgba: [1.0; 4], srgb: false }
}

#[test]
fn parses_every_path_command() {
    let d = "M10 10 h20 v20 H10 Z m5 5 l5 0 c1 1 2 2 3 3 s 1 1 2 2 q1 1 2 2 t 3 3 a 5 5 0 0 1 10 0 A5 5 0 1 0 50 50 z";
    let path = Path::parse(d).unwrap();
    assert!(path.segs.len() > 10);
    assert!(Path::parse("L 1 2").is_err());
    assert!(Path::parse("M 1").is_err());
    let sq = Path::parse("M0 0 L10 0 L10 10 L0 10 Z").unwrap().flatten(0.1);
    assert_eq!(sq.len(), 1);
    assert!((sq[0].area() - 100.0).abs() < 1e-9);
}

#[test]
fn primitive_areas_match_formulas() {
    let e = shapes::ellipse(50.0, 50.0, 40.0, 20.0).flatten(0.01);
    assert!((area(&e) - std::f64::consts::PI * 800.0).abs() / (std::f64::consts::PI * 800.0) < 1e-3);
    let r = shapes::rect(0.0, 0.0, 100.0, 50.0, [10.0; 4]).flatten(0.01);
    let expect = 5000.0 - 4.0 * 100.0 + std::f64::consts::PI * 100.0;
    assert!((area(&r) - expect).abs() < 1.0, "{}", area(&r));
    // radii larger than the box shrink to fit: a pill
    let pill = shapes::rect(0.0, 0.0, 100.0, 20.0, [100.0; 4]).flatten(0.01);
    assert!((area(&pill) - (80.0 * 20.0 + std::f64::consts::PI * 100.0)).abs() < 2.0);
    let hex = shapes::polygon(p(0.0, 0.0), 6, 10.0, 0.0, 0.0).flatten(0.01);
    assert!((area(&hex) - 1.5 * 3f64.sqrt() * 100.0).abs() < 1e-6);
    let star = shapes::star(p(0.0, 0.0), 5, 10.0, 5.0, 0.0, 0.0, 0.0).flatten(0.01);
    assert_eq!(star[0].pts.len(), 10);
    let top = star[0].pts[0];
    assert!((top.x).abs() < 1e-9 && (top.y + 10.0).abs() < 1e-9, "first tip points up");
}

#[test]
fn closed_outlines_start_where_svg_2_starts_them() {
    // an ellipse starts at 3 o'clock, a rect at its top-left corner (x + rx, y),
    // both clockwise on screen (+y down), so a quarter trim of an ellipse is its bottom-right arc
    let e = shapes::ellipse(0.0, 0.0, 40.0, 20.0).flatten(0.01);
    assert!((e[0].pts[0].x - 40.0).abs() < 1e-9 && e[0].pts[0].y.abs() < 1e-9);
    assert!(e[0].pts[1].y > 0.0, "clockwise on screen: {:?}", e[0].pts[1]);
    let q = measure::trim(&e, 0.0, 0.25, 0.0, TrimMode::Simultaneous);
    assert!(q[0].pts.iter().all(|v| v.x >= -1e-9 && v.y >= -1e-9), "bottom-right quadrant");
    let r = shapes::rect(0.0, 0.0, 100.0, 50.0, [10.0; 4]).flatten(0.01);
    assert!((r[0].pts[0].x - 10.0).abs() < 1e-9 && r[0].pts[0].y.abs() < 1e-9);
    assert!(r[0].pts[1].x > r[0].pts[0].x, "then right along the top");
    // Lottie's ellipse keeps its own start at the top
    let l = shapes::ellipse_top(0.0, 0.0, 40.0, 20.0).flatten(0.01);
    assert!(l[0].pts[0].x.abs() < 1e-9 && (l[0].pts[0].y + 20.0).abs() < 1e-9);
}

#[test]
fn polygon_and_star_on_an_ellipse() {
    // vertices on the ellipse of radii (rx, ry), first straight up, clockwise
    let d = shapes::polygon_on(p(0.0, 0.0), 4, p(20.0, 10.0), 0.0, 0.0).flatten(0.01);
    let v: Vec<(f64, f64)> = d[0].pts.iter().map(|q| (q.x.round(), q.y.round())).collect();
    assert_eq!(v, vec![(0.0, -10.0), (20.0, 0.0), (0.0, 10.0), (-20.0, 0.0)]);
    let s = shapes::star_on(p(0.0, 0.0), 4, p(20.0, 10.0), p(5.0, 5.0), 0.0, 0.0, 0.0).flatten(0.01);
    assert_eq!(s[0].pts.len(), 8);
    assert!((s[0].pts[2].x - 20.0).abs() < 1e-9 && (s[0].pts[1].x.hypot(s[0].pts[1].y) - 5.0).abs() < 1e-9);
}

#[test]
fn flattening_stays_within_tolerance() {
    let c = shapes::ellipse(0.0, 0.0, 100.0, 100.0);
    for tol in [1.0, 0.25, 0.05] {
        let ps = c.flatten(tol);
        for w in ps[0].pts.windows(2) {
            let m = w[0].lerp(w[1], 0.5);
            // the Bézier circle deviates from a true circle by ≤ 0.03 %
            assert!(100.0 - m.len() <= tol + 0.03, "tol {tol}: sagitta {}", 100.0 - m.len());
        }
    }
}

#[test]
fn trim_simultaneous_and_sequential() {
    let line = |y: f64, len: f64| Poly { pts: vec![p(0.0, y), p(len, y)], closed: false };
    let ps = vec![line(0.0, 100.0), line(10.0, 300.0)];
    let sim = measure::trim(&ps, 0.25, 0.75, 0.0, TrimMode::Simultaneous);
    assert_eq!(sim.len(), 2);
    assert!((sim[0].length() - 50.0).abs() < 1e-9 && (sim[1].length() - 150.0).abs() < 1e-9);
    let seq = measure::trim(&ps, 0.0, 0.5, 0.0, TrimMode::Sequential);
    let total: f64 = seq.iter().map(Poly::length).sum();
    assert!((total - 200.0).abs() < 1e-9);
    // offset wraps around a closed path and joins across the seam
    let sq = Path::parse("M0 0 H10 V10 H0 Z").unwrap().flatten(0.1);
    let w = measure::trim(&sq, 0.0, 0.5, 0.75, TrimMode::Simultaneous);
    assert_eq!(w.len(), 1);
    assert!((w[0].length() - 20.0).abs() < 1e-9);
    assert!(measure::trim(&ps, 0.4, 0.4, 0.0, TrimMode::Simultaneous).is_empty());
}

#[test]
fn dashes_follow_the_pattern() {
    let l = vec![Poly { pts: vec![p(0.0, 0.0), p(100.0, 0.0)], closed: false }];
    let d = measure::dash(&l, &[10.0, 5.0], 0.0);
    assert_eq!(d.len(), 7);
    assert!((d[0].length() - 10.0).abs() < 1e-9);
    assert!((d[1].pts[0].x - 15.0).abs() < 1e-9);
    let shifted = measure::dash(&l, &[10.0, 5.0], 5.0);
    assert!((shifted[0].length() - 5.0).abs() < 1e-9);
    // odd patterns repeat twice
    let odd = measure::dash(&l, &[10.0], 0.0);
    assert_eq!(odd.len(), 5);
}

#[test]
fn stroke_areas() {
    let st = |cap, join| Style { width: 10.0, cap, join, miter_limit: 4.0 };
    let line = vec![Poly { pts: vec![p(0.0, 0.0), p(100.0, 0.0)], closed: false }];
    let scene_area = |ps: Vec<Poly>| {
        let mut s = Scene::default();
        s.cmds.push(Cmd::Fill {
            polys: ps
                .iter()
                .map(|q| Poly { pts: q.pts.iter().map(|&x| x + p(20.0, 20.0)).collect(), closed: true })
                .collect(),
            rule: FillRule::NonZero,
            paint: white(),
            opacity: 1.0,
        });
        coverage_sum(&s, 160, 60)
    };
    assert!((scene_area(stroke::stroke(&line, &st(Cap::Butt, Join::Miter), 0.05)) - 1000.0).abs() < 0.5);
    assert!((scene_area(stroke::stroke(&line, &st(Cap::Square, Join::Miter), 0.05)) - 1100.0).abs() < 0.5);
    let round = scene_area(stroke::stroke(&line, &st(Cap::Round, Join::Miter), 0.01));
    assert!((round - (1000.0 + std::f64::consts::PI * 25.0)).abs() < 0.5, "{round}");
    // closed square ring: outer 110² minus inner 90² with miter joins
    let sq = Path::parse("M0 0 H100 V100 H0 Z").unwrap().flatten(0.1);
    let ring = stroke::stroke(&sq, &st(Cap::Butt, Join::Miter), 0.05);
    let mut s = Scene::default();
    s.cmds.push(Cmd::Fill {
        polys: ring
            .iter()
            .map(|q| Poly { pts: q.pts.iter().map(|&x| x + p(10.0, 10.0)).collect(), closed: true })
            .collect(),
        rule: FillRule::NonZero,
        paint: white(),
        opacity: 1.0,
    });
    let a = coverage_sum(&s, 130, 130);
    assert!((a - (110.0 * 110.0 - 90.0 * 90.0)).abs() < 1.0, "{a}");
    // a sharp zig-zag keeps a solid inner join (no holes): covered area ≥ its segments' rectangles minus overlap
    let zz = vec![Poly { pts: vec![p(0.0, 30.0), p(50.0, 0.0), p(100.0, 30.0)], closed: false }];
    let bevel = scene_area(stroke::stroke(&zz, &st(Cap::Butt, Join::Bevel), 0.05));
    let rects = 2.0 * (50f64.hypot(30.0)) * 10.0;
    assert!(bevel > rects - 60.0 && bevel < rects + 30.0, "{bevel} vs {rects}");
}

#[test]
fn rasteriser_matches_analytic_areas() {
    let mut s = Scene::default();
    s.fill(&shapes::ellipse(100.3, 70.7, 50.0, 50.0), FillRule::NonZero, white(), 1.0, 0.01);
    let a = coverage_sum(&s, 200, 150);
    assert!((a - std::f64::consts::PI * 2500.0).abs() < 1.0, "circle {a}");
    // half-pixel-aligned rectangle: exact fractional coverage on every edge pixel
    let mut r = Scene::default();
    r.fill(&shapes::rect(10.5, 20.25, 33.0, 17.5, [0.0; 4]), FillRule::NonZero, white(), 1.0, 0.1);
    let e = tile::encode(&r, [64, 48]);
    let px = tile::render_cpu(&e, &|_, _, _| [1.0; 4]);
    assert!((px[25 * 64 + 10][3] - 0.5).abs() < 1e-5);
    assert!((px[20 * 64 + 20][3] - 0.75).abs() < 1e-5);
    assert!((px[30 * 64 + 30][3] - 1.0).abs() < 1e-6);
    // even-odd: a square inside a square leaves a hole; nonzero with the same winding fills it
    let d = "M0 0 H60 V60 H0 Z M20 20 H40 V40 H20 Z";
    for (rule, expect) in [(FillRule::EvenOdd, 3200.0), (FillRule::NonZero, 3600.0)] {
        let mut s = Scene::default();
        s.fill(&Path::parse(d).unwrap().transform(&Xf::translate(2.0, 2.0)), rule, white(), 1.0, 0.1);
        assert!((coverage_sum(&s, 64, 64) - expect).abs() < 1e-3);
    }
    // shapes partly off-target and crossing many tiles keep exact coverage inside
    let mut big = Scene::default();
    big.fill(&shapes::rect(-50.0, -30.0, 100.0, 80.0, [0.0; 4]), FillRule::NonZero, white(), 1.0, 0.1);
    assert!((coverage_sum(&big, 40, 40) - 1600.0).abs() < 1e-3);
}

#[test]
fn layers_masks_and_mattes() {
    let rect = |x: f64, y: f64, w: f64, h: f64| shapes::rect(x, y, w, h, [0.0; 4]).flatten(0.1);
    // clip: a 32×32 fill masked to its left half
    let mut s = Scene::default();
    s.cmds.push(Cmd::Push { mask_init: 0.0 });
    s.cmds.push(Cmd::Fill { polys: rect(0.0, 0.0, 32.0, 32.0), rule: FillRule::NonZero, paint: white(), opacity: 1.0 });
    s.cmds.push(Cmd::Mask {
        polys: rect(0.0, 0.0, 16.0, 32.0),
        rule: FillRule::NonZero,
        op: MaskOp::Add,
        opacity: 1.0,
        invert: false,
    });
    s.cmds.push(Cmd::Pop { opacity: 0.5 });
    assert!((coverage_sum(&s, 32, 32) - 16.0 * 32.0 * 0.5).abs() < 1e-3);
    // subtract with an inverted matte
    let mut m = Scene::default();
    m.cmds.push(Cmd::Push { mask_init: 1.0 });
    m.cmds.push(Cmd::Fill { polys: rect(0.0, 0.0, 32.0, 32.0), rule: FillRule::NonZero, paint: white(), opacity: 1.0 });
    m.cmds.push(Cmd::PushMatte);
    m.cmds.push(Cmd::Fill { polys: rect(0.0, 0.0, 32.0, 8.0), rule: FillRule::NonZero, paint: white(), opacity: 1.0 });
    m.cmds.push(Cmd::PopMatte { mode: MatteMode::AlphaInverted, opacity: 1.0 });
    assert!((coverage_sum(&m, 32, 32) - 32.0 * 24.0).abs() < 1e-3);
}

#[test]
fn shape_modifiers() {
    let ctx = modifiers::Ctx { center: p(50.0, 50.0), time: 0.0, tol: 0.05 };
    let sq = || vec![Item::new(shapes::rect(40.0, 40.0, 20.0, 20.0, [0.0; 4]))];
    let mut r = sq();
    modifiers::apply(
        &mut r,
        &Modifier::Repeater {
            copies: 4.0,
            offset: 0.0,
            offset_x: 30.0,
            offset_y: 0.0,
            rotation: 0.0,
            scale: 1.0,
            start_opacity: 1.0,
            end_opacity: 0.25,
            below: false,
        },
        &ctx,
    );
    assert_eq!(r.len(), 4);
    assert!((r[3].opacity - 0.25).abs() < 1e-12);
    assert!((poly_bounds(&r[3].parts[0].0.flatten(0.1)).0[0] - 130.0).abs() < 1e-9);
    let mut o = sq();
    modifiers::apply(&mut o, &Modifier::OffsetPath { amount: 5.0, join: Join::Miter, miter_limit: 4.0 }, &ctx);
    let a = area(&o[0].parts[0].0.flatten(0.05));
    assert!((a - 900.0).abs() < 1e-6, "offset square {a}");
    let mut rc = sq();
    modifiers::apply(&mut rc, &Modifier::RoundCorners { radius: 5.0 }, &ctx);
    let a = area(&rc[0].parts[0].0.flatten(0.01));
    assert!((a - (400.0 - 100.0 + std::f64::consts::PI * 25.0)).abs() < 1.0, "rounded {a}");
    let mut pb = sq();
    modifiers::apply(&mut pb, &Modifier::PuckerBloat { amount: -50.0 }, &ctx);
    let v = pb[0].parts[0].0.contours()[0].v.clone();
    assert!(v.iter().all(|q| q.dist(p(50.0, 50.0)) > 14.2), "pucker pushes vertices out");
    assert!(area(&pb[0].parts[0].0.flatten(0.05)) < 900.0);
    let mut zz = sq();
    modifiers::apply(&mut zz, &Modifier::ZigZag { size: 3.0, ridges: 4, smooth: false }, &ctx);
    assert_eq!(zz[0].parts[0].0.contours()[0].v.len(), 4 * 8);
    let mut tw = sq();
    modifiers::apply(&mut tw, &Modifier::Twist { amount: 90.0 }, &ctx);
    assert!((area(&tw[0].parts[0].0.flatten(0.05)) - 400.0).abs() < 40.0);
    let mut wg = sq();
    modifiers::apply(
        &mut wg,
        &Modifier::WigglePath { size: 2.0, detail: 50.0, frequency: 2.0, seed: 7, smooth: false },
        &ctx,
    );
    let mut wg2 = sq();
    modifiers::apply(
        &mut wg2,
        &Modifier::WigglePath { size: 2.0, detail: 50.0, frequency: 2.0, seed: 7, smooth: false },
        &ctx,
    );
    assert_eq!(wg, wg2, "deterministic");
    let mut mg = sq();
    mg.push(Item::new(shapes::rect(50.0, 50.0, 20.0, 20.0, [0.0; 4])));
    modifiers::apply(&mut mg, &Modifier::Merge { op: MaskOp::Subtract }, &ctx);
    assert_eq!(mg.len(), 1);
    assert!(mg[0].is_compound());
    let mut tr = sq();
    tr.push(Item::new(shapes::rect(0.0, 0.0, 20.0, 20.0, [0.0; 4])));
    modifiers::apply(&mut tr, &Modifier::Trim { amount: 75.0, offset: 0.0, mode: TrimMode::Sequential }, &ctx);
    let lens: Vec<f64> = tr.iter().map(|i| i.parts[0].0.flatten(0.1).iter().map(Poly::length).sum()).collect();
    assert!((lens[0] - 40.0).abs() < 1e-6 && lens[1] < 1e-9, "{lens:?}");
}

#[test]
fn deformers() {
    let c = p(50.0, 50.0);
    let id = |d: &Deformer, q| d.apply(q);
    assert_eq!(
        id(&Deformer::Bend { amount: 0.0, axis: Axis::X, center: c, extent: 100.0 }, p(10.0, 20.0)),
        p(10.0, 20.0)
    );
    let bent = Deformer::Bend { amount: 90.0, axis: Axis::X, center: c, extent: 100.0 }.apply(p(100.0, 50.0));
    let r = 100.0 / std::f64::consts::FRAC_PI_2;
    assert!((bent.x - (50.0 + r * (0.5f64 * std::f64::consts::FRAC_PI_2).sin())).abs() < 1e-9);
    let tw = Deformer::Twist { amount: 90.0, center: c, radius: 50.0 };
    assert_eq!(tw.apply(p(0.0, 0.0)), p(0.0, 0.0));
    let wave = Deformer::Wave { amount: 5.0, frequency: 1.0, phase: 90.0, axis: Axis::Y, size: p(100.0, 100.0) };
    assert!((wave.apply(p(0.0, 10.0)).y - 15.0).abs() < 1e-9);
    let sq = Deformer::Squash { amount: 0.5, axis: Axis::Y, center: c }.apply(p(60.0, 60.0));
    assert!((sq.x - 70.0).abs() < 1e-9 && (sq.y - 55.0).abs() < 1e-9);
    let mut offs = vec![p(0.0, 0.0); 16];
    offs[5] = p(10.0, 0.0);
    let mw = Deformer::MeshWarp { rows: 4, cols: 4, size: p(90.0, 90.0), offsets: offs };
    assert!((mw.apply(p(30.0, 30.0)).x - 40.0).abs() < 1e-9, "control point moves by its offset");
    assert_eq!(mw.apply(p(0.0, 0.0)), p(0.0, 0.0));
    let b = Deformer::Bulge { amount: 0.5, center: c, radius: 40.0 }.apply(p(60.0, 50.0));
    assert!(b.x > 60.0);
    let pinch = Deformer::Bulge { amount: -0.5, center: c, radius: 40.0 }.apply(p(60.0, 50.0));
    assert!(pinch.x < 60.0);
    let sph = Deformer::Spherize { amount: 1.0, center: c, radius: 40.0 }.apply(p(70.0, 50.0));
    assert!(sph.x > 70.0 && sph.x < 90.0);
    let rip = Deformer::Ripple { amount: 3.0, frequency: 5.0, phase: 0.0, center: c, radius: 0.0 }.apply(p(55.0, 50.0));
    assert!((rip.y - 50.0).abs() < 1e-12 && (rip.x - 55.0).abs() <= 3.0 + 1e-12);
    let tb = Deformer::Turbulence { amount: 4.0, frequency: 3.0, phase: 0.0, seed: 1 };
    let q = tb.apply(p(12.0, 34.0));
    assert!(q.dist(p(12.0, 34.0)) <= 4.0 * 2f64.sqrt() + 1e-9);
    assert_eq!(q, tb.apply(p(12.0, 34.0)));
    let h = deform::corner_pin(100.0, 50.0, [p(10.0, 10.0), p(120.0, 0.0), p(110.0, 70.0), p(0.0, 60.0)]).unwrap();
    let cp = Deformer::CornerPin { h };
    assert!(cp.apply(p(100.0, 50.0)).dist(p(110.0, 70.0)) < 1e-6);
    assert!(cp.apply(p(0.0, 50.0)).dist(p(0.0, 60.0)) < 1e-6);
    let (pos, uv, idx) = deform::grid([0.0, 0.0, 10.0, 10.0], 4, 2);
    assert_eq!((pos.len(), uv.len(), idx.len()), (15, 15, 48));
}

#[test]
fn puppet_pins_are_satisfied_and_rigid() {
    let rect = [0.0, 0.0, 200.0, 100.0];
    let pins = [
        Pin { kind: PinKind::Position, rest: p(20.0, 50.0), offset: p(0.0, 0.0), rotation: 0.0, amount: 1.0 },
        Pin { kind: PinKind::Position, rest: p(180.0, 50.0), offset: p(0.0, -40.0), rotation: 0.0, amount: 1.0 },
    ];
    let pp = Puppet::solve(rect, 16, 8, &pins);
    assert!(pp.map(p(20.0, 50.0)).dist(p(20.0, 50.0)) < 0.5);
    assert!(pp.map(p(180.0, 50.0)).dist(p(180.0, 10.0)) < 0.5);
    // lengths along the middle stay close to rest (rigidity)
    let (a, b) = (pp.map(p(90.0, 50.0)), pp.map(p(110.0, 50.0)));
    assert!((a.dist(b) - 20.0).abs() < 2.0, "{}", a.dist(b));
    // no pins moved: identity
    let still = Puppet::solve(
        rect,
        8,
        4,
        &[Pin { kind: PinKind::Position, rest: p(50.0, 50.0), offset: p(0.0, 0.0), rotation: 0.0, amount: 1.0 }],
    );
    assert!(still.map(p(120.0, 30.0)).dist(p(120.0, 30.0)) < 1e-6);
}

#[test]
fn inverse_kinematics() {
    let bone = |id: &str, parent: Option<usize>, x: f64, len: f64| Bone {
        id: id.into(),
        parent,
        x,
        y: 0.0,
        rotation: 0.0,
        length: len,
        scale_x: 1.0,
        scale_y: 1.0,
    };
    let root = Xf::translate(100.0, 100.0);
    let mut two = vec![bone("a", None, 0.0, 50.0), bone("b", Some(0), 50.0, 50.0)];
    let target = p(160.0, 150.0);
    rig::solve_ik(&mut two, &root, 1, target, true, 1.0);
    let w = rig::world_poses(&two, &root);
    assert!(rig::tip(&w[1], 50.0).dist(target) < 1e-6);
    let mut flip = vec![bone("a", None, 0.0, 50.0), bone("b", Some(0), 50.0, 50.0)];
    rig::solve_ik(&mut flip, &root, 1, target, false, 1.0);
    let wf = rig::world_poses(&flip, &root);
    assert!(wf[1].origin().dist(w[1].origin()) > 1.0, "bend side flips");
    // FABRIK on five bones
    let mut chain: Vec<Bone> =
        (0usize..5).map(|k| bone(&format!("b{k}"), k.checked_sub(1), if k == 0 { 0.0 } else { 20.0 }, 20.0)).collect();
    let t = p(150.0, 160.0);
    rig::solve_ik(&mut chain, &root, 4, t, true, 1.0);
    let wc = rig::world_poses(&chain, &root);
    assert!(rig::tip(&wc[4], 20.0).dist(t) < 1e-3);
    // out of reach: the chain points at the target
    let mut far = vec![bone("a", None, 0.0, 50.0), bone("b", Some(0), 50.0, 50.0)];
    rig::solve_ik(&mut far, &root, 1, p(400.0, 100.0), true, 1.0);
    assert!((rig::tip(&rig::world_poses(&far, &root)[1], 50.0).y - 100.0).abs() < 1e-3);
    // skinning with rest = now is the identity
    let poses = rig::world_poses(&two, &root);
    let skin = rig::Skin::new(&poses, &poses, &[50.0, 50.0], Xf::IDENTITY, Vec::new());
    assert!(skin.map(p(130.0, 110.0)).dist(p(130.0, 110.0)) < 1e-9);
}

#[test]
fn wiggle_path_smooth_joins_the_same_points_with_curves() {
    // a straight line of 200 units wiggled with 20 points: corner mode is a polyline, smooth a Catmull-Rom spline through them
    let line = || vec![Item::new(shapes::line(200.0, 0.0))];
    let ctx = modifiers::Ctx { center: p(100.0, 0.0), time: 0.0, tol: 0.01 };
    let wiggle = |smooth: bool| {
        let mut it = line();
        modifiers::apply(
            &mut it,
            &Modifier::WigglePath { size: 6.0, detail: 10.0, frequency: 0.0, seed: 3, smooth },
            &ctx,
        );
        it.remove(0).parts.remove(0).0
    };
    let (corner, smooth) = (wiggle(false), wiggle(true));
    let (vc, vs) = (corner.contours()[0].v.clone(), smooth.contours()[0].v.clone());
    assert_eq!(vc.len(), 20, "20 points at detail 10 over 200 units");
    assert_eq!(vc, vs, "the same points, so the same wiggle");
    // the largest turn between successive flattened segments: sharp in the polyline, gentle in the spline
    let turn = |path: &Path| {
        let q = &path.flatten(0.01)[0].pts;
        q.windows(3)
            .map(|w| {
                let (a, b) = (w[1] - w[0], w[2] - w[1]);
                (a.x * b.y - a.y * b.x).atan2(a.x * b.x + a.y * b.y).abs()
            })
            .fold(0.0, f64::max)
    };
    let (tc, ts) = (turn(&corner), turn(&smooth));
    assert!(tc > 0.5 && ts < tc / 3.0, "corners turn {tc} rad, the spline {ts}");
    // deterministic, and the curve passes through the points
    assert_eq!(wiggle(true), wiggle(true));
}

#[test]
fn ik_pole_and_soft_reach() {
    use sr_vector::rig::{self, IkExtras};
    let root = Xf::IDENTITY;
    let arm = || {
        vec![
            rig::Bone {
                id: "a".into(),
                parent: None,
                x: 0.0,
                y: 0.0,
                rotation: 0.0,
                length: 10.0,
                scale_x: 1.0,
                scale_y: 1.0,
            },
            rig::Bone {
                id: "b".into(),
                parent: Some(0),
                x: 10.0,
                y: 0.0,
                rotation: 0.0,
                length: 10.0,
                scale_x: 1.0,
                scale_y: 1.0,
            },
        ]
    };
    let elbow = |b: &[rig::Bone]| rig::world_poses(b, &root)[1].origin();
    let target = p(10.0, 10.0);
    // a pole on one side of the line root-target puts the elbow on that side; the other side flips it
    for (pole, side) in [(p(0.0, 30.0), 1.0), (p(30.0, 0.0), -1.0)] {
        for bend_positive in [true, false] {
            let mut b = arm();
            rig::solve_ik_with(
                &mut b,
                &root,
                1,
                target,
                bend_positive,
                1.0,
                IkExtras { pole: Some(pole), softness: 0.0 },
            );
            let e = elbow(&b);
            let cross = (target.x) * (e.y) - (target.y) * (e.x);
            assert!(cross * side > 0.0, "pole {pole:?} bend_positive {bend_positive}: elbow {e:?}");
            let tipp = rig::tip(&rig::world_poses(&b, &root)[1], 10.0);
            assert!(tipp.dist(target) < 1e-6, "the tip still reaches {tipp:?}");
        }
    }
    // a pole on the line leaves the choice to bend_positive
    let mut on = arm();
    rig::solve_ik_with(&mut on, &root, 1, target, true, 1.0, IkExtras { pole: Some(p(20.0, 20.0)), softness: 0.0 });
    let mut plain = arm();
    rig::solve_ik(&mut plain, &root, 1, target, true, 1.0);
    assert_eq!(on, plain);
    // soft reach: continuous, never beyond the reach, identical to the hard solution when the target is near
    assert_eq!(rig::soft_reach(5.0, 10.0, 10.0, 0.3), 5.0);
    assert!(
        (rig::soft_reach(14.0, 10.0, 10.0, 0.3) - 14.0).abs() < 1e-12,
        "up to (1 - softness) of the reach nothing changes"
    );
    let eased = rig::soft_reach(19.0, 10.0, 10.0, 0.3);
    assert!(eased < 19.0 && eased > 14.0, "{eased}");
    assert!(rig::soft_reach(1e6, 10.0, 10.0, 0.3) < 20.0);
    let mut last = 0.0;
    for k in 0..400 {
        let r = rig::soft_reach(k as f64 * 0.1, 10.0, 10.0, 0.3);
        assert!(r >= last, "monotone");
        last = r;
    }
    // a target past the reach: the soft chain stays slightly bent where the hard one is straight
    let far = p(25.0, 0.0);
    let mut hard = arm();
    rig::solve_ik(&mut hard, &root, 1, far, true, 1.0);
    let mut soft = arm();
    rig::solve_ik_with(&mut soft, &root, 1, far, true, 1.0, IkExtras { pole: None, softness: 0.3 });
    assert!(hard[1].rotation.abs() < 1e-3, "{}", hard[1].rotation);
    assert!(soft[1].rotation.abs() > 1.0, "{}", soft[1].rotation);
}
