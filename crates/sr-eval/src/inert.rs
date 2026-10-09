//! Inert attribute rule I8 (SREP 18, Specification 5): a node whose window lies wholly outside [0, project
//! `duration`) is never drawn. Rules I1 to I7 depend on the document alone and are found by validation
//! (`sr_model::inert`); I8 needs the windows the evaluator computes, with the clocks of the node's ancestors applied.
//!
//! Also the attributes that SREP 54 asks engines to report as inert without giving them a rule of the SREP 18
//! table: an IK `pole` or `softness` the solver does not read. They are reported as [`UNREAD`], this engine's own
//! information finding, until an SREP names them.

use std::collections::BTreeMap;

use sr_model::element::{children, AttrValue, Element};
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

/// The code of an attribute with no effect that no inert rule of SREPs 18 and 34 names: this engine's own information
/// finding.
pub const UNREAD: &str = "E19";

/// SREP 54, Semantics 3: a skeleton's `transformConstraint type="ik"` reads `pole` and `softness` only on a chain of
/// two bones (the analytic solution); the chain of the bone it follows is that bone and its ancestors, less the
/// leading bones its next bone sits at the origin of (zero length, which the solver drops), in the rest pose. A
/// single bone aims at the target and FABRIK solves longer chains: both ignore the two attributes. On a constraint of
/// another type they are not read either.
pub(crate) fn unread_ik_extras(e: &dyn Element, out: &mut Vec<Diagnostic>) {
    if e.element_name() != "skeleton" {
        return;
    }
    let num = |e: &dyn Element, name: &str| match e.get_attr(name) {
        Some(AttrValue::Num(v)) => v,
        _ => 0.0,
    };
    let text = |e: &dyn Element, name: &str| e.get_attr(name).map(|v| v.to_string());
    // bones in document order: (id, parent id, x, y)
    let bones: Vec<(String, Option<String>, f64, f64)> = children(e)
        .into_iter()
        .filter(|c| c.element_name() == "bone")
        .map(|c| (c.element_id().unwrap_or("").to_string(), text(c, "parent"), num(c, "x"), num(c, "y")))
        .collect();
    // a constraint applies to the bone before it
    let mut seen = 0usize;
    for c in children(e) {
        match c.element_name() {
            "bone" => seen += 1,
            "transformConstraint" => {
                let Some(end) = seen.checked_sub(1) else { continue };
                let pole = c.get_attr("pole").is_some();
                let soft = num(c, "softness") != 0.0;
                if !pole && !soft {
                    continue;
                }
                let kind = text(c, "type").unwrap_or_default();
                let why = if kind != "ik" {
                    format!("a {kind} constraint, which does not read it")
                } else {
                    let n = chain_length(&bones, end);
                    if n == 2 {
                        continue;
                    }
                    format!("an IK chain of {n} bone(s), which only a chain of two reads")
                };
                for (attr, set) in [("pole", pole), ("softness", soft)] {
                    if set {
                        out.push(Diagnostic::info(
                            UNREAD,
                            format!(
                                "transformConstraint after bone {:?}: @{attr} is accepted but has no effect on {why} (SREP 54)",
                                bones[end].0
                            ),
                            c.loc(),
                            e.element_id().unwrap_or(""),
                        ));
                    }
                }
            }
            _ => {}
        }
    }
}

/// Bones in the chain that ends at bone `end`: it and its ancestors, less the leading ones of zero length (whose next
/// bone sits at their origin).
fn chain_length(bones: &[(String, Option<String>, f64, f64)], end: usize) -> usize {
    let mut chain = vec![end];
    let mut cur = end;
    while let Some(p) = bones[cur].1.as_ref().and_then(|pid| bones.iter().position(|b| &b.0 == pid)) {
        if chain.contains(&p) {
            break;
        }
        chain.push(p);
        cur = p;
    }
    chain.reverse();
    while chain.len() > 1 && bones[chain[1]].2 == 0.0 && bones[chain[1]].3 == 0.0 {
        chain.remove(0);
    }
    chain.len()
}
