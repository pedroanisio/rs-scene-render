//! Inert attribute rule I8 (SREP 18, Specification 5): a node whose window lies wholly outside [0, project
//! `duration`) is never drawn. Rules I1 to I7 depend on the document alone and are found by validation
//! (`sr_model::inert`); I8 needs the windows the evaluator computes, with the clocks of the node's ancestors applied.

use std::collections::BTreeMap;

use sr_model::element::Element as _;
use sr_model::values::Fps;
use sr_model::Diagnostic;

use crate::program::{Clock, InstNode, Kind};

/// The composition-time interval of node `i`'s own window, when every ancestor's clock is the identity or an affine
/// map with a positive scale; `None` when a clock (a symbol, include or media clock) cannot be inverted in closed form.
fn composition_window(nodes: &[InstNode], i: usize) -> Option<(f64, f64)> {
    let n = &nodes[i];
    let (mut s, mut e) = (n.vis_start, n.vis_end.unwrap_or(f64::INFINITY));
    let mut up = n.parent;
    while let Some(p) = up {
        let parent = &nodes[p as usize];
        match &parent.clock {
            Clock::Same => {}
            // child = origin + (t − origin − offset) · scale, so t = origin + offset + (child − origin) / scale
            Clock::Affine { origin, offset, scale } if *scale > 0.0 => {
                let inv = |c: f64| origin + offset + (c - origin) / scale;
                (s, e) = (inv(s), inv(e));
            }
            _ => return None,
        }
        up = parent.parent;
    }
    Some((s, e))
}

/// Time on node `i`'s timeline at composition time `t`: every ancestor's clock applied, root first.
fn timeline_at(nodes: &[InstNode], i: usize, t: f64) -> f64 {
    match nodes[i].parent {
        None => t,
        Some(p) => crate::eval::clock_map(&nodes[p as usize].clock, timeline_at(nodes, p as usize, t)),
    }
}

/// Whether node `i`'s own window contains the time of any frame of [0, `duration`).
fn sampled_inside(nodes: &[InstNode], i: usize, duration: f64, fps: Fps) -> bool {
    let n = &nodes[i];
    let (s, e) = (n.vis_start, n.vis_end.unwrap_or(f64::INFINITY));
    (0..fps.frame_count(duration).max(1)).map(|k| fps.frame_time(k)).take_while(|&t| t < duration).any(|t| {
        let tl = timeline_at(nodes, i, t);
        tl >= s && tl < e
    })
}

/// `INERT-I8` for every node of the main document whose window, on the composition timeline, lies wholly outside
/// [0, `duration`). A node instantiated several times (in a symbol, a repeat) is reported once, when no instance is
/// ever inside. Transitions and repeat copies carry no window of their own.
pub(crate) fn never_drawn(nodes: &[InstNode], duration: f64, fps: Fps) -> Vec<Diagnostic> {
    // per authored element (by its source offset): whether some instance is drawn, and the first instance
    let mut seen: BTreeMap<u32, (bool, usize)> = BTreeMap::new();
    for (i, n) in nodes.iter().enumerate() {
        if n.doc != 0 || n.kind != Kind::Plain || n.name == "transition" {
            continue;
        }
        let inside = match composition_window(nodes, i) {
            Some((s, e)) => e > 0.0 && s < duration,
            None => sampled_inside(nodes, i, duration, fps),
        };
        let entry = seen.entry(n.elem.loc().offset).or_insert((false, i));
        entry.0 |= inside;
    }
    seen.into_values()
        .filter(|(inside, _)| !inside)
        .map(|(_, i)| {
            let n = &nodes[i];
            let end = n.vis_end.map_or("on".to_string(), |e| format!("{e}"));
            Diagnostic::info(
                sr_model::inert::I8,
                format!(
                    "<{}> {:?} is shown from {} to {end} s on its timeline, wholly outside the composition's [0, {duration}) s, so it is never drawn",
                    n.name, n.id, n.vis_start
                ),
                n.elem.loc(),
                &*n.id,
            )
        })
        .collect()
}
