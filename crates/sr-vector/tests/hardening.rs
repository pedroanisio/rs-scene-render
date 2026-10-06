//! Hostile input is bounded: no unbounded recursion, allocation or work.

use sr_vector::geom::p;
use sr_vector::lottie::{self, Lottie};
use sr_vector::measure::{self, TrimMode};
use sr_vector::modifiers::{self, Ctx, Item, Modifier};
use sr_vector::path::{self, Poly};
use sr_vector::scene::{Cmd, Paint};
use sr_vector::{shapes, zip};

const PRECOMP: &str = r#"{"ty":0,"refId":"ID","ip":0,"op":60,"w":10,"h":10,"ks":{}}"#;

#[test]
fn lottie_precomp_cycle_draws_once() {
    let l = PRECOMP.replace("ID", "a");
    let doc = format!(
        r#"{{"fr":30,"ip":0,"op":60,"w":100,"h":100,"assets":[{{"id":"a","layers":[{l},{l}]}}],"layers":[{l}]}}"#
    );
    let lot = Lottie::parse(doc.as_bytes(), None, &[]).unwrap();
    assert!(lot.skipped.iter().any(|s| s.contains("recursive precomp")), "{:?}", lot.skipped);
    assert!(lot.render(0.0, 0.1).cmds.len() < 100);
    // a cycle through two assets
    let (la, lb) = (PRECOMP.replace("ID", "a"), PRECOMP.replace("ID", "b"));
    let doc = format!(
        r#"{{"fr":30,"ip":0,"op":60,"w":100,"h":100,"assets":[{{"id":"a","layers":[{lb},{lb}]}},{{"id":"b","layers":[{la},{la}]}}],"layers":[{la}]}}"#
    );
    let lot = Lottie::parse(doc.as_bytes(), None, &[]).unwrap();
    assert!(lot.render(0.0, 0.1).cmds.len() < 100);
}

#[test]
fn lottie_precomp_fanout_is_budgeted() {
    // 12 levels of three instances each: 3^12 leaves without a budget
    let assets: Vec<String> = (0..12)
        .map(|k| {
            let l = PRECOMP.replace("ID", &format!("a{}", k + 1));
            format!(r#"{{"id":"a{k}","layers":[{l},{l},{l}]}}"#)
        })
        .collect();
    let doc = format!(
        r#"{{"fr":30,"ip":0,"op":60,"w":100,"h":100,"assets":[{}],"layers":[{}]}}"#,
        assets.join(","),
        PRECOMP.replace("ID", "a0")
    );
    let lot = Lottie::parse(doc.as_bytes(), None, &[]).unwrap();
    assert!(lot.skipped.iter().all(|s| !s.contains("recursive precomp")));
    let n = lot.render(0.0, 0.1).cmds.len();
    // three commands a layer: the root's, and three layers an evaluated instance
    assert!(n <= 3 + 9 * lottie::MAX_PRECOMPS, "{n}");
}

fn shape_doc(items: &str) -> Vec<u8> {
    format!(
        r#"{{"fr":30,"ip":0,"op":60,"w":100,"h":100,"layers":[{{"ty":4,"ip":0,"op":60,"ks":{{}},"shapes":[
        {{"ty":"rc","p":{{"a":0,"k":[50,50]}},"s":{{"a":0,"k":[40,40]}},"r":{{"a":0,"k":0}}}},{items}]}}]}}"#
    )
    .into_bytes()
}

#[test]
fn lottie_gradient_stop_count_is_bounded_by_its_data() {
    for count in ["4611686018427387904", "100000000000", "2"] {
        let doc = shape_doc(&format!(
            r#"{{"ty":"gf","o":{{"a":0,"k":100}},"s":{{"a":0,"k":[0,0]}},"e":{{"a":0,"k":[100,0]}},"t":1,
            "g":{{"p":{count},"k":{{"a":0,"k":[0,1,0,0,1,0,0,1]}}}}}}"#
        ));
        let scene = Lottie::parse(&doc, None, &[]).unwrap().render(0.0, 0.1);
        let stops: Vec<usize> = scene
            .cmds
            .iter()
            .filter_map(|c| match c {
                Cmd::Fill { paint: Paint::Gradient(g), .. } => Some(g.stops.len()),
                _ => None,
            })
            .collect();
        assert_eq!(stops, [2], "p = {count}");
    }
}

fn square() -> Vec<Item> {
    vec![Item::new(shapes::rect(0.0, 0.0, 100.0, 100.0, [0.0; 4]))]
}

const CTX: Ctx = Ctx { center: p(50.0, 50.0), time: 0.0, tol: 0.1 };

fn repeater(copies: f64) -> Modifier {
    Modifier::Repeater {
        copies,
        offset: 0.0,
        offset_x: 1.0,
        offset_y: 0.0,
        rotation: 0.0,
        scale: 1.0,
        start_opacity: 1.0,
        end_opacity: 1.0,
        below: false,
    }
}

#[test]
fn repeater_copies_are_clamped() {
    for copies in [1e18, 1e12, f64::INFINITY] {
        let mut items = square();
        modifiers::apply(&mut items, &repeater(copies), &CTX);
        assert_eq!(items.len(), modifiers::MAX_REPEATER_ITEMS, "{copies}");
    }
    // repeaters stacked on each other share the bound
    let mut items = square();
    modifiers::apply(&mut items, &repeater(1000.0), &CTX);
    assert_eq!(items.len(), 1000);
    modifiers::apply(&mut items, &repeater(1000.0), &CTX);
    assert_eq!(items.len(), modifiers::MAX_REPEATER_ITEMS / 1000 * 1000);
    // NaN and negative counts repeat nothing
    for copies in [f64::NAN, -3.0] {
        let mut items = square();
        modifiers::apply(&mut items, &repeater(copies), &CTX);
        assert!(items.is_empty());
    }
    let doc = shape_doc(
        r#"{"ty":"rp","c":{"a":0,"k":1e18},"o":{"a":0,"k":0},"tr":{}},{"ty":"fl","c":{"a":0,"k":[1,0,0,1]}}"#,
    );
    Lottie::parse(&doc, None, &[]).unwrap().render(0.0, 0.1);
}

#[test]
fn star_points_and_ridges_and_wiggle_detail_are_clamped() {
    let star = shapes::star(p(0.0, 0.0), 200_000, 10.0, 5.0, 0.0, 0.0, 0.0);
    assert_eq!(star.contours()[0].v.len(), 2 * shapes::MAX_POINTS as usize);
    let poly = shapes::polygon(p(0.0, 0.0), 200_000, 10.0, 0.0, 0.0);
    assert_eq!(poly.contours()[0].v.len(), shapes::MAX_POINTS as usize);
    // legitimate counts are untouched
    assert_eq!(shapes::star(p(0.0, 0.0), 5, 10.0, 5.0, 0.0, 0.0, 0.0).contours()[0].v.len(), 10);

    let mut items = square();
    modifiers::apply(&mut items, &Modifier::ZigZag { size: 1.0, ridges: 100_000, smooth: false }, &CTX);
    assert_eq!(items[0].parts[0].0.contours()[0].v.len(), 4 * 2 * modifiers::MAX_RIDGES as usize);

    let mut items = square();
    modifiers::apply(
        &mut items,
        &Modifier::WigglePath { size: 1.0, detail: 2e5, frequency: 1.0, seed: 1, smooth: false },
        &CTX,
    );
    let n: usize = items[0].parts[0].0.flatten(0.1).iter().map(|q| q.pts.len()).sum();
    assert!(n <= modifiers::MAX_WIGGLE_POINTS, "{n}");
}

#[test]
fn subdivide_caps_its_points() {
    let long = Poly { pts: vec![p(0.0, 0.0), p(4e6, 0.0)], closed: false };
    let s = long.subdivide(0.5);
    assert!(s.pts.len() <= path::MAX_SUBDIVIDE + 2, "{}", s.pts.len());
    assert_eq!((s.pts[0], *s.pts.last().unwrap()), (p(0.0, 0.0), p(4e6, 0.0)));
    // ordinary edges split exactly as before
    let q = Poly { pts: vec![p(0.0, 0.0), p(10.0, 0.0), p(10.0, 3.0)], closed: true }.subdivide(4.0);
    assert_eq!(q.pts.len(), 3 + 1 + 3);
    assert_eq!(q.pts[1], p(0.0, 0.0).lerp(p(10.0, 0.0), 1.0 / 3.0));
}

#[test]
fn subdivide_leaves_non_finite_polylines_alone() {
    for bad in [f64::INFINITY, f64::NAN, 1e308] {
        let q = Poly { pts: vec![p(0.0, 0.0), p(bad, 0.0), p(-bad, 5.0)], closed: true };
        assert!(q.subdivide(0.5).pts.len() <= path::MAX_SUBDIVIDE + 4, "{bad}");
    }
    let q = Poly { pts: vec![p(0.0, 0.0), p(8.0, 0.0)], closed: false };
    assert_eq!(q.subdivide(f64::NAN).pts.len(), 2);
    assert!(q.subdivide(0.0).pts.len() <= path::MAX_SUBDIVIDE + 2);
}

#[test]
fn over_dense_dashes_are_solid() {
    let period = 2e-5;
    let len = (measure::MAX_DASHES + 100_000) as f64 * period;
    let line = vec![Poly { pts: vec![p(0.0, 0.0), p(len, 0.0)], closed: false }];
    assert_eq!(measure::dash(&line, &[1e-5, 1e-5], 0.0), line);
    // the densest accepted pattern still dashes
    let ok = vec![Poly { pts: vec![p(0.0, 0.0), p(100.0, 0.0)], closed: false }];
    assert_eq!(measure::dash(&ok, &[0.5, 0.5], 0.0).len(), 100);
    // degenerate patterns and offsets terminate
    for (pat, off) in [
        (vec![f64::INFINITY, 1.0], -1.0),
        (vec![f64::NAN, 1.0], 0.0),
        (vec![1.0, 1.0], f64::NAN),
        (vec![1.0, 1.0], f64::INFINITY),
        (vec![1e-9, 1e-9], 1e300),
        (vec![0.0, 1e-7], 0.0),
    ] {
        measure::dash(&ok, &pat, off);
    }
}

#[test]
fn dashes_match_trimmed_ranges() {
    // a 2000-point circle: every dash equals the same range cut on its own
    let n = 2000;
    let pts = (0..n).map(|k| p(0.0, 100.0).rot(std::f64::consts::TAU * k as f64 / n as f64)).collect();
    let ring = Poly { pts, closed: true };
    let total = ring.length();
    let dashes = measure::dash(std::slice::from_ref(&ring), &[0.7, 0.4, 0.1, 0.3], 0.25);
    let mut s = -0.25;
    let mut k = 0;
    for step in [0.7, 0.4, 0.1, 0.3].into_iter().cycle() {
        if s >= total {
            break;
        }
        let e = s + step;
        if k % 2 == 0 {
            let want = &measure::trim(
                std::slice::from_ref(&ring),
                s.max(0.0) / total,
                e.min(total) / total,
                0.0,
                TrimMode::Simultaneous,
            )[0];
            let got = &dashes[k / 2];
            assert_eq!(got.pts.len(), want.pts.len(), "dash {}", k / 2);
            assert!(got.pts.iter().zip(&want.pts).all(|(a, b)| a.dist(*b) < 1e-9), "dash {}", k / 2);
        }
        s = e;
        k += 1;
    }
    assert_eq!(k / 2 + k % 2, dashes.len());
}

/// A zip of `(name, method, declared size, raw bytes)`, each listed `listed` times in the directory.
fn zip_of(files: &[(&str, u16, u32, &[u8])], listed: usize) -> Vec<u8> {
    let mut out = Vec::new();
    let mut central = Vec::new();
    for (name, method, size, raw) in files {
        let off = out.len() as u32;
        out.extend(0x0403_4b50u32.to_le_bytes());
        out.extend([20, 0, 0, 0]);
        out.extend(method.to_le_bytes());
        out.extend([0u8; 8]);
        out.extend((raw.len() as u32).to_le_bytes());
        out.extend(size.to_le_bytes());
        out.extend((name.len() as u16).to_le_bytes());
        out.extend([0, 0]);
        out.extend(name.as_bytes());
        out.extend(*raw);
        for _ in 0..listed {
            central.extend(0x0201_4b50u32.to_le_bytes());
            central.extend([20, 0, 20, 0, 0, 0]);
            central.extend(method.to_le_bytes());
            central.extend([0u8; 8]);
            central.extend((raw.len() as u32).to_le_bytes());
            central.extend(size.to_le_bytes());
            central.extend((name.len() as u16).to_le_bytes());
            central.extend([0u8; 12]);
            central.extend(off.to_le_bytes());
            central.extend(name.as_bytes());
        }
    }
    let cd = out.len() as u32;
    let count = (files.len() * listed) as u16;
    out.extend(&central);
    out.extend(0x0605_4b50u32.to_le_bytes());
    out.extend([0, 0, 0, 0]);
    out.extend(count.to_le_bytes());
    out.extend(count.to_le_bytes());
    out.extend((central.len() as u32).to_le_bytes());
    out.extend(cd.to_le_bytes());
    out.extend([0, 0]);
    out
}

#[test]
fn zip_entry_count_is_capped() {
    let z = zip_of(&[("a.json", 0, 2, b"{}")], 60_000);
    let e = zip::entries(&z).unwrap_err();
    assert!(e.contains("entries"), "{e}");
    assert_eq!(zip::entries(&zip_of(&[("a.json", 0, 2, b"{}")], 3)).unwrap().len(), 3);
}

#[test]
fn zip_inflated_sizes_are_capped() {
    let zeros = vec![0u8; 1 << 20];
    let packed = miniz_oxide::deflate::compress_to_vec(&zeros, 6);
    let lim = zip::Limits { entries: 64, entry_bytes: 1 << 16, total_bytes: 1 << 22 };
    // the declared size (4 GiB here) does not set the limit
    let bomb = zip_of(&[("a.json", 8, u32::MAX, &packed)], 1);
    assert!(zip::entries_within(&bomb, &lim).unwrap_err().contains("a.json"));
    // an honest entry inflates
    let honest = zip_of(&[("a.json", 8, 1 << 20, &packed)], 1);
    let roomy = zip::Limits { entry_bytes: 1 << 21, ..lim };
    assert_eq!(zip::entries_within(&honest, &roomy).unwrap()[0].1.len(), 1 << 20);
    assert_eq!(zip::entries(&honest).unwrap()[0].1.len(), 1 << 20);
    // one body listed many times counts every time, deflated or stored
    let many = zip_of(&[("a.json", 8, 1 << 20, &packed)], 8);
    assert!(zip::entries_within(&many, &roomy).unwrap_err().contains("total"));
    let stored = zip_of(&[("a.json", 0, 1 << 20, &zeros)], 8);
    assert!(zip::entries_within(&stored, &roomy).unwrap_err().contains("total"));
    assert!(zip::entries_within(&stored, &lim).unwrap_err().contains("a.json"));
}

#[test]
fn lottie_easing_handle_without_an_axis() {
    // found by the dotlottie fuzz target: an `i`/`o` handle with no `x` or `y`
    let doc = br##"{"fr":30,"ip":0,"op":60,"w":100,"h":100,"layers":[{"ty":1,"ip":0,"op":60,"sw":5,"sh":5,"sc":"#102030",
    "ks":{"p":{"a":1,"k":[{"t":0,"s":[0,0],"o":{"x":0.3},"i":{"h":0.7,"y":1}},{"t":30,"s":[60,0],"o":{},"i":{}}]}}}]}"##;
    assert_eq!(Lottie::parse(doc, None, &[]).unwrap().render(15.0, 0.1).cmds.len(), 1);
}
