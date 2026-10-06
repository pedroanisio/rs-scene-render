//! SREP 26 on the rendered frame: copies of a repeat placed on generated points, measured as red blobs
//! (centroids and counts within the kit's 2 px). Run with SR_REQUIRE_GPU=1 to fail instead of skipping.

mod common;
use common::*;

fn scene(body: &str) -> sr_model::Document {
    let xml = format!(
        r##"<scene version="1.2"><project width="640" height="360" fps="24" duration="4" background="#000000FF"/><composition>{body}</composition></scene>"##
    );
    let opts = sr_model::LoadOptions { verify_assets: false, base_dir: None };
    sr_model::load_str(&xml, &opts).unwrap_or_else(|e| panic!("{e:?}\n{xml}"))
}

/// Centroids of the 4-connected red blobs (red above ½, green below ½), in scan order of their first pixel.
fn blobs(r: &Rendered) -> Vec<[f64; 2]> {
    let [w, h] = r.size;
    let red = |x: u32, y: u32| {
        let p = r.at(x, y);
        p[0] > 0.5 && p[1] < 0.5
    };
    let mut seen = vec![false; (w * h) as usize];
    let mut out = Vec::new();
    for y0 in 0..h {
        for x0 in 0..w {
            if seen[(y0 * w + x0) as usize] || !red(x0, y0) {
                continue;
            }
            let (mut sx, mut sy, mut n) = (0.0, 0.0, 0.0);
            let mut stack = vec![(x0, y0)];
            seen[(y0 * w + x0) as usize] = true;
            while let Some((x, y)) = stack.pop() {
                sx += x as f64 + 0.5;
                sy += y as f64 + 0.5;
                n += 1.0;
                let next = [(x.wrapping_sub(1), y), (x + 1, y), (x, y.wrapping_sub(1)), (x, y + 1)];
                for (nx, ny) in next {
                    if nx < w && ny < h && !seen[(ny * w + nx) as usize] && red(nx, ny) {
                        seen[(ny * w + nx) as usize] = true;
                        stack.push((nx, ny));
                    }
                }
            }
            out.push([sx / n, sy / n]);
        }
    }
    out
}

#[track_caller]
fn same_places(got: &[[f64; 2]], want: &[[f64; 2]]) {
    assert_eq!(got.len(), want.len(), "blobs {got:?}, want {want:?}");
    for w in want {
        assert!(
            got.iter().any(|g| (g[0] - w[0]).abs() <= 2.0 && (g[1] - w[1]).abs() <= 2.0),
            "no blob within 2 px of {w:?}: {got:?}"
        );
    }
}

const SQUARE: &str =
    r##"<shape id="s" shape="rect" width="20" height="20" anchorX="10" anchorY="10" fill="#FF0000FF"/>"##;

#[test]
fn grid() {
    let d = scene(
        r##"<repeat id="r" x="320" y="180"><points type="grid" columns="3" rows="2"/><shape id="s" shape="rect" width="40" height="40" anchorX="20" anchorY="20" fill="#FF0000FF"/></repeat>"##,
    );
    let Some(r) = render(&d) else { return };
    same_places(
        &blobs(&r),
        &[[220.0, 130.0], [320.0, 130.0], [420.0, 130.0], [220.0, 230.0], [320.0, 230.0], [420.0, 230.0]],
    );
}

#[test]
fn order() {
    let d = scene(
        r##"<repeat id="r" x="320" y="180" offsetX="20"><points type="along-path" path="M100,-50 L100,0" count="2" orient="true"/><shape id="s" shape="rect" width="40" height="10" anchorX="20" anchorY="5" fill="#FF0000FF"/></repeat>"##,
    );
    let Some(r) = render(&d) else { return };
    // copy 0 at (420, 130); copy 1 at (420, 200), not (440, 180)
    same_places(&blobs(&r), &[[420.0, 130.0], [420.0, 200.0]]);
}

#[test]
fn paths_open_and_closed() {
    let Some(r) = render(&scene(&format!(
        r#"<repeat id="r" x="320" y="180"><points type="along-path" path="M-200,0 L200,0" count="5"/>{SQUARE}</repeat>"#
    ))) else {
        return;
    };
    same_places(&blobs(&r), &[[120.0, 180.0], [220.0, 180.0], [320.0, 180.0], [420.0, 180.0], [520.0, 180.0]]);
    let Some(r) = render(&scene(&format!(
        r#"<repeat id="r" x="320" y="180"><points type="along-path" path="M-50,-50 H50 V50 H-50 Z" count="4"/>{SQUARE}</repeat>"#
    ))) else {
        return;
    };
    same_places(&blobs(&r), &[[270.0, 130.0], [370.0, 130.0], [370.0, 230.0], [270.0, 230.0]]);
}

#[test]
fn degenerate_paths_draw_nothing() {
    for p in [r#"path="" count="3""#, r#"path="M50,50" count="3""#, r#"path="M0,0 L50,0" count="0""#] {
        let Some(r) = render(&scene(&format!(r#"<repeat id="r"><points type="along-path" {p}/>{SQUARE}</repeat>"#)))
        else {
            return;
        };
        assert!(blobs(&r).is_empty(), "{p}");
    }
}

#[test]
fn scatter_sliver_counts() {
    // the SREP's sliver region, drawn three times larger so each copy is a separate 6 x 6 px blob
    let region = "M0,0 H100 V0.5 H0.5 V100 H0 Z";
    let dot = r##"<shape id="s" shape="rect" width="2" height="2" anchorX="1" anchorY="1" fill="#FF0000FF"/>"##;
    for (seed, n) in [(7, 7), (61, 6)] {
        let pts = sr_eval::points::scatter_path(&sr_eval::points::FlatPath::parse(region).unwrap(), seed, 10, false).0;
        assert_eq!(pts.len(), n);
        let apart =
            pts.iter().enumerate().all(|(i, a)| pts[i + 1..].iter().all(|b| (a.x - b.x).hypot(a.y - b.y) > 2.5));
        assert!(apart, "seed {seed}: copies closer than 2.5 units would merge into one blob");
        let Some(r) = render(&scene(&format!(
            r#"<group id="g" x="20" y="20" scaleX="3" scaleY="3"><repeat id="r"><points type="scatter" path="{region}" seed="{seed}" count="10"/>{dot}</repeat></group>"#
        ))) else {
            return;
        };
        let want: Vec<[f64; 2]> = pts.iter().map(|p| [20.0 + 3.0 * p.x, 20.0 + 3.0 * p.y]).collect();
        same_places(&blobs(&r), &want);
    }
}

#[test]
fn scatter_edge_and_fill_rule() {
    let Some(r) = render(&scene(&format!(
        r#"<repeat id="r" x="100" y="100"><points type="scatter" path="M0,0 H100 V100 H0 Z" seed="14325487974692486532" count="1"/>{SQUARE}</repeat>"#
    ))) else {
        return;
    };
    same_places(&blobs(&r), &[[100.0, 160.38]]);
    // nested squares wound the same way: nonzero puts a copy inside the inner square, evenodd does not
    let region = "M0,0 H100 V100 H0 Z M25,25 H75 V75 H25 Z";
    let dot = r##"<shape id="s" shape="rect" width="4" height="4" anchorX="2" anchorY="2" fill="#FF0000FF"/>"##;
    let inner = |c: &[f64; 2]| c[0] > 125.0 && c[0] < 175.0 && c[1] > 125.0 && c[1] < 175.0;
    let Some(r) = render(&scene(&format!(
        r#"<repeat id="r" x="100" y="100"><points type="scatter" path="{region}" seed="7" count="5"/>{dot}</repeat>"#
    ))) else {
        return;
    };
    let b = blobs(&r);
    assert!(b.iter().any(|c| (c[0] - 130.35).abs() <= 2.0 && (c[1] - 166.18).abs() <= 2.0), "{b:?}");
    let Some(r) = render(&scene(&format!(
        r#"<repeat id="r" x="100" y="100"><points type="scatter" path="{region}" fillRule="evenodd" seed="7" count="5"/>{dot}</repeat>"#
    ))) else {
        return;
    };
    let b = blobs(&r);
    assert!(!b.iter().any(inner), "{b:?}");
    assert!(b.iter().any(|c| (c[0] - 188.77).abs() <= 2.0 && (c[1] - 113.93).abs() <= 2.0), "{b:?}");
}

#[test]
fn vertices_and_list() {
    let Some(r) = render(&scene(&format!(
        r#"<repeat id="r" x="320" y="180"><points type="vertices" path="M-100,-50 L100,-50 Q50,0 0,50 Z"/>{SQUARE}</repeat>"#
    ))) else {
        return;
    };
    same_places(&blobs(&r), &[[220.0, 130.0], [420.0, 130.0], [320.0, 230.0]]);
    let Some(r) = render(&scene(&format!(
        r#"<repeat id="r" x="320" y="180"><points type="list" at="-100,-50 0,0 100,50"/>{SQUARE}</repeat>"#
    ))) else {
        return;
    };
    same_places(&blobs(&r), &[[220.0, 130.0], [320.0, 180.0], [420.0, 230.0]]);
}

#[test]
fn animated_spacing_moves_the_copies_without_the_delay() {
    let d = scene(&format!(
        r#"<repeat id="r" x="320" y="180" timeStep="0.5"><points type="grid" columns="3"><animate property="spacingX"><key time="0" value="50"/><key time="2" value="150"/></animate></points>{SQUARE}</repeat>"#
    ));
    let Some(r) = render_times(&d, &[0.0, 1.0]) else { return };
    same_places(&blobs(&r), &[[220.0, 180.0], [320.0, 180.0], [420.0, 180.0]]);
    // the cache must not keep the copies of an earlier frame
    let Some(r) = render_times(&d, &[1.0, 1.5]) else { return };
    same_places(&blobs(&r), &[[195.0, 180.0], [320.0, 180.0], [445.0, 180.0]]);
}
