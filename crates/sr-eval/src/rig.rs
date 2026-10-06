//! Transform constraints, skeleton poses, tracking data and stabilisation.
//!
//! Constraints run after keyframes, expressions and layout, as a post-pass
//! over the frame's nodes in paint order: each constrained node's new world
//! transform replaces the old one, and the change carries to its
//! descendants. A constraint reads its target as already constrained when
//! the target comes earlier in paint order. Skeleton bones pose after all
//! node constraints, applying the constraints inside the skeleton to the
//! bone that precedes each of them.

use std::collections::HashMap;
use std::path::Path as FsPath;
use std::sync::Arc;

use sr_model::element::{children, AttrValue, Element};
use sr_model::model as m;
use sr_model::Diagnostic;
use sr_vector::geom::{p, Xf, P};
use sr_vector::rig::{self, Bone};
use sr_vector::track::{self, TrackData};

use crate::eval::{Affine, BonePose, FrameNode, Props, SkinWeights};
use crate::program::{InstNode, Program};
use crate::value::Value;

/// Loaded tracking data.
#[derive(Debug, Clone)]
pub struct TrackEntry {
    pub data: TrackData,
    pub kind: String,
    pub footage: Option<String>,
    pub time_offset: f64,
}

/// Whether `e` is the element `name` (some elements report their XSD type name).
pub fn is(e: &dyn Element, name: &str) -> bool {
    let n = e.element_name();
    n == name || n.strip_suffix("Type") == Some(name)
}

fn read(src: &str, base: &FsPath) -> Result<Vec<u8>, String> {
    match sr_model::assets::resolve(src, base) {
        sr_model::assets::Resolved::Local(path) => std::fs::read(&path).map_err(|e| format!("{}: {e}", path.display())),
        _ => Err(format!("{src}: only local files are supported")),
    }
}

/// Reads every `<tracking>/<trackData>` of the scene (E17 on failure).
pub fn load_tracks(scene: &m::Scene, base: &FsPath, diags: &mut Vec<Diagnostic>) -> HashMap<String, Arc<TrackEntry>> {
    let mut out = HashMap::new();
    let Some(tr) = &scene.tracking else { return out };
    for td in &tr.track_data {
        let parsed = read(&td.src, base).and_then(|b| {
            let text = String::from_utf8_lossy(&b).to_string();
            TrackData::parse(&text, td.format.as_str()).map_err(|e| e.to_string())
        });
        match parsed {
            Ok(data) => {
                out.insert(
                    td.id.clone(),
                    Arc::new(TrackEntry {
                        data,
                        kind: td.kind.as_str().to_string(),
                        footage: td.footage.clone(),
                        time_offset: td.time_offset,
                    }),
                );
            }
            Err(e) => diags.push(Diagnostic::error(
                "E17",
                format!("tracking data {:?}: {e}", td.id),
                td.loc,
                format!("/scene/tracking/trackData[@id='{}']", td.id),
            )),
        }
    }
    out
}

fn bone_ids(e: &dyn Element) -> Vec<String> {
    children(e).into_iter().filter(|c| is(*c, "bone")).filter_map(|c| c.element_id().map(str::to_string)).collect()
}

/// Reads skin weights of skeletons with `@weights` (E17 on failure):
/// JSON `{"points": [[x, y], …], "weights": [{"boneId": w, …}, …]}` in skeleton space.
pub fn load_skins(
    nodes: &[InstNode],
    base_dirs: &[std::path::PathBuf],
    diags: &mut Vec<Diagnostic>,
) -> HashMap<Arc<str>, Arc<SkinWeights>> {
    let mut out = HashMap::new();
    for n in nodes.iter().filter(|n| n.name == "skeleton") {
        let e: &dyn Element = &*n.elem;
        let Some(AttrValue::Str(src)) = e.get_attr("weights") else { continue };
        let base = base_dirs.get(n.doc as usize).cloned().unwrap_or_default();
        let ids = bone_ids(e);
        let parsed = read(&src, &base)
            .and_then(|b| serde_json::from_slice::<serde_json::Value>(&b).map_err(|e| e.to_string()))
            .and_then(|v| {
                let pts = v["points"].as_array().ok_or("missing \"points\"")?;
                let ws = v["weights"].as_array().ok_or("missing \"weights\"")?;
                if pts.len() != ws.len() {
                    return Err("\"points\" and \"weights\" differ in length".to_string());
                }
                let mut samples = Vec::with_capacity(pts.len());
                for (pt, w) in pts.iter().zip(ws) {
                    let xy: Vec<f64> =
                        pt.as_array().map(|a| a.iter().filter_map(|x| x.as_f64()).collect()).unwrap_or_default();
                    if xy.len() < 2 {
                        return Err("each point needs [x, y]".into());
                    }
                    let mut bw = Vec::new();
                    for (bone, x) in w.as_object().ok_or("each weight entry must be an object")? {
                        let k = ids.iter().position(|i| i == bone).ok_or_else(|| format!("unknown bone {bone:?}"))?;
                        bw.push((k, x.as_f64().unwrap_or(0.0)));
                    }
                    samples.push(([xy[0], xy[1]], bw));
                }
                Ok(SkinWeights(samples))
            });
        match parsed {
            Ok(sw) => {
                out.insert(n.id.clone(), Arc::new(sw));
            }
            Err(msg) => diags.push(Diagnostic::error(
                "E17",
                format!("skin weights of skeleton {:?}: {msg}", n.id),
                e.loc(),
                n.id.to_string(),
            )),
        }
    }
    out
}

// ------------------------------------------------------------------ attributes

struct A<'a> {
    e: &'a dyn Element,
    props: Option<&'a Props>,
}

impl A<'_> {
    fn num(&self, name: &str, d: f64) -> f64 {
        if let Some(v) = self.props.and_then(|p| p.get(name)).and_then(Value::as_num) {
            return v;
        }
        match self.e.get_attr(name) {
            Some(AttrValue::Num(v)) => v,
            Some(AttrValue::Length(l)) => l.value,
            _ => d,
        }
    }
    fn opt(&self, name: &str) -> Option<f64> {
        self.props.and_then(|p| p.get(name)).and_then(Value::as_num).or(match self.e.get_attr(name) {
            Some(AttrValue::Num(v)) => Some(v),
            _ => None,
        })
    }
    fn str(&self, name: &str) -> Option<String> {
        match self.e.get_attr(name) {
            Some(AttrValue::Str(s)) => Some(s),
            Some(AttrValue::Bool(b)) => Some(b.to_string()),
            Some(other) => Some(other.to_string()),
            None => None,
        }
    }
    fn bool(&self, name: &str, d: bool) -> bool {
        match self.e.get_attr(name) {
            Some(AttrValue::Bool(b)) => b,
            _ => d,
        }
    }
}

fn part<'g>(n: &'g FrameNode, key: &str) -> Option<&'g Props> {
    n.parts.iter().find(|s| &*s.key == key).map(|s| &s.props)
}

fn xf(a: &Affine) -> Xf {
    Xf(a.0)
}

fn rot(x: &Xf) -> f64 {
    libm::atan2(x.0[1], x.0[0]).to_degrees()
}

fn scales(x: &Xf) -> (f64, f64) {
    let [a, b, c, d, _, _] = x.0;
    let sx = libm::hypot(a, b);
    (sx, if sx > 0.0 { (a * d - b * c) / sx } else { 0.0 })
}

fn about(pivot: P, deg: f64) -> Xf {
    Xf::translate(pivot.x, pivot.y).mul(&Xf::rotate(deg)).mul(&Xf::translate(-pivot.x, -pivot.y))
}

fn wrap(d: f64) -> f64 {
    let mut d = d.rem_euclid(360.0);
    if d > 180.0 {
        d -= 360.0;
    }
    d
}

/// Blends two transforms by translation, rotation (shortest way) and scale.
fn blend(a: &Xf, b: &Xf, k: f64) -> Xf {
    if k >= 1.0 {
        return *b;
    }
    if k <= 0.0 {
        return *a;
    }
    let (ra, rb) = (rot(a), rot(b));
    let r = ra + wrap(rb - ra) * k;
    let (sa, sb) = (scales(a), scales(b));
    let (sx, sy) = (sa.0 + (sb.0 - sa.0) * k, sa.1 + (sb.1 - sa.1) * k);
    let t = a.origin().lerp(b.origin(), k);
    Xf::translate(t.x, t.y).mul(&Xf::rotate(r)).mul(&Xf::scale(sx, sy))
}

// ------------------------------------------------------------------ targets

enum Target {
    Node(usize),
    Bone(usize, usize),
    Track(Arc<TrackEntry>),
}

struct Scene<'a> {
    p: &'a Program,
    t: f64,
}

impl Scene<'_> {
    fn find(&self, nodes: &[FrameNode], id: &str) -> Option<Target> {
        if let Some(i) = nodes.iter().position(|n| &*n.id == id) {
            return Some(Target::Node(i));
        }
        for (i, n) in nodes.iter().enumerate().filter(|(_, n)| n.kind == "skeleton") {
            if let Some(k) = bone_ids(&*n.elem).iter().position(|b| b == id) {
                return Some(Target::Bone(i, k));
            }
        }
        self.p.tracks.get(id).map(|t| Target::Track(t.clone()))
    }

    /// World transform of a target (tracks: translation to the tracked point).
    fn world(&self, nodes: &[FrameNode], tg: &Target, point: Option<&str>) -> Option<Xf> {
        match tg {
            Target::Node(i) => Some(xf(&nodes[*i].world)),
            Target::Bone(s, k) => self.pose(nodes, *s).get(*k).map(|b| b.1),
            Target::Track(te) => self.track_point(nodes, te, point).map(|q| Xf::translate(q.x, q.y)),
        }
    }

    /// Pivot of a target in frame space.
    fn pivot(&self, nodes: &[FrameNode], tg: &Target, point: Option<&str>) -> Option<P> {
        match tg {
            Target::Node(i) => Some(xf(&nodes[*i].world).apply(p(nodes[*i].anchor[0], nodes[*i].anchor[1]))),
            other => self.world(nodes, other, point).map(|w| w.origin()),
        }
    }

    /// Local transform of a target (relative to its parent).
    fn local(&self, nodes: &[FrameNode], tg: &Target) -> Option<Xf> {
        match tg {
            Target::Node(i) => Some(xf(&nodes[*i].local)),
            Target::Bone(s, k) => self.bones(nodes, *s).0.get(*k).map(Bone::local),
            Target::Track(_) => None,
        }
    }

    /// Footage layer of a track and its source time.
    fn footage<'n>(&self, nodes: &'n [FrameNode], te: &TrackEntry) -> Option<&'n FrameNode> {
        let f = te.footage.as_deref()?;
        nodes.iter().find(|n| n.asset.as_deref() == Some(f))
    }

    /// Maps footage pixels into frame space through the footage layer's placement.
    fn footage_map(&self, n: &FrameNode) -> Xf {
        let asset = n.asset.as_deref().and_then(|k| self.p.assets.get(k)).map(|(_, id)| id.clone());
        let size = asset
            .and_then(|id| {
                self.p.scene.assets.as_ref().and_then(|a| a.children.iter().find(|c| c.id() == Some(id.as_str())))
            })
            .map(|a| {
                let num = |k: &str| match a.get_attr(k) {
                    Some(AttrValue::Num(v)) => v,
                    _ => 1.0,
                };
                [num("width"), num("height")]
            })
            .unwrap_or([1.0, 1.0]);
        let local = match n.content {
            Some(c) => {
                let [x0, y0, x1, y1] = c.dest;
                let [u0, v0, u1, v1] = c.uv;
                let (sx, sy) = ((x1 - x0) / ((u1 - u0) * size[0]), (y1 - y0) / ((v1 - v0) * size[1]));
                Xf([sx, 0.0, 0.0, sy, x0 - u0 * size[0] * sx, y0 - v0 * size[1] * sy])
            }
            None => Xf::IDENTITY,
        };
        xf(&n.world).mul(&local)
    }

    fn track_time(&self, nodes: &[FrameNode], te: &TrackEntry) -> f64 {
        let base = self.footage(nodes, te).and_then(|n| n.source_time).unwrap_or(self.t);
        base + te.time_offset
    }

    fn track_point(&self, nodes: &[FrameNode], te: &TrackEntry, point: Option<&str>) -> Option<P> {
        let name = point.or_else(|| te.data.first_name())?;
        let q = te.data.point(name, self.track_time(nodes, te))?;
        Some(match self.footage(nodes, te) {
            Some(f) => self.footage_map(f).apply(q),
            None => q,
        })
    }

    /// Bones of skeleton node `s`: (current, rest, document order → topological order).
    fn bones(&self, nodes: &[FrameNode], s: usize) -> (Vec<Bone>, Vec<Bone>) {
        let n = &nodes[s];
        let e: &dyn Element = &*n.elem;
        let mut now = Vec::new();
        let mut rest = Vec::new();
        let mut ids = Vec::new();
        let mut parents = Vec::new();
        for (k, c) in children(e).into_iter().filter(|c| is(*c, "bone")).enumerate() {
            let key = format!("{}/{}[{k}]", n.id, c.element_name());
            let a = A { e: c, props: part(n, &key) };
            let r = A { e: c, props: None };
            let mk = |a: &A| Bone {
                id: c.element_id().unwrap_or("").to_string(),
                parent: None,
                x: a.num("x", 0.0),
                y: a.num("y", 0.0),
                rotation: a.num("rotation", 0.0),
                length: a.num("length", 0.0),
                scale_x: a.num("scaleX", 1.0),
                scale_y: a.num("scaleY", 1.0),
            };
            now.push(mk(&a));
            rest.push(mk(&r));
            ids.push(c.element_id().unwrap_or("").to_string());
            parents.push(a.str("parent"));
        }
        for (i, par) in parents.iter().enumerate() {
            let pi = par.as_ref().and_then(|pid| ids.iter().position(|x| x == pid)).filter(|&j| j != i);
            now[i].parent = pi;
            rest[i].parent = pi;
        }
        (now, rest)
    }

    /// World poses (bone space → frame) of a skeleton with its constraints: (id, world, rest, length).
    fn pose(&self, nodes: &[FrameNode], s: usize) -> Vec<(String, Xf, Xf, f64)> {
        let n = &nodes[s];
        let root = xf(&n.world);
        let (mut now, rest) = self.bones(nodes, s);
        // work in topological order so parents precede children
        let order = topo(&now);
        let inv: Vec<usize> = {
            let mut v = vec![0; order.len()];
            for (k, &i) in order.iter().enumerate() {
                v[i] = k;
            }
            v
        };
        let reorder = |bs: &[Bone]| -> Vec<Bone> {
            order.iter().map(|&i| Bone { parent: bs[i].parent.map(|p| inv[p]), ..bs[i].clone() }).collect()
        };
        let mut tb = reorder(&now);
        let rb = reorder(&rest);
        // constraints after each bone apply to it
        let e: &dyn Element = &*n.elem;
        let (mut bone_k, mut ck) = (None, 0);
        for c in children(e) {
            if is(c, "bone") {
                bone_k = Some(bone_k.map_or(0, |b: usize| b + 1));
                continue;
            }
            if !is(c, "transformConstraint") {
                continue;
            }
            let key = format!("{}/{}[{ck}]", n.id, c.element_name());
            ck += 1;
            let Some(b) = bone_k.map(|b| inv[b]) else { continue };
            let a = A { e: c, props: part(n, &key) };
            self.constrain_bone(nodes, &mut tb, &root, b, &a);
        }
        let wn = rig::world_poses(&tb, &root);
        let wr = rig::world_poses(&rb, &root);
        now = tb;
        (0..order.len()).map(|i| (now[inv[i]].id.clone(), wn[inv[i]], wr[inv[i]], now[inv[i]].length)).collect()
    }

    fn constrain_bone(&self, nodes: &[FrameNode], bones: &mut [Bone], root: &Xf, b: usize, a: &A) {
        let kind = a.str("type").unwrap_or_default();
        let influence = a.num("influence", 1.0);
        let target = a.str("target").and_then(|id| self.find(nodes, &id));
        let point = a.str("point");
        if kind == "ik" {
            if let Some(tp) = target.as_ref().and_then(|t| self.pivot(nodes, t, point.as_deref())) {
                let off = p(a.num("offsetX", 0.0), a.num("offsetY", 0.0));
                let pole = a.str("pole").and_then(|id| self.find(nodes, &id)).and_then(|t| self.pivot(nodes, &t, None));
                let extras = rig::IkExtras { pole, softness: a.num("softness", 0.0) };
                rig::solve_ik_with(bones, root, b, tp + off, a.bool("bendPositive", true), influence, extras);
            }
            return;
        }
        let worlds = rig::world_poses(bones, root);
        let parent = bones[b].parent.map(|k| worlds[k]).unwrap_or(*root);
        let w = worlds[b];
        let local = bones[b].local();
        let Some(w2) = self.solve(nodes, &kind, a, &w, p(0.0, 0.0), &parent, &local, target.as_ref(), point.as_deref())
        else {
            return;
        };
        let w3 = blend(&w, &w2, influence);
        // back to local values
        let l = parent.inverse().unwrap_or(Xf::IDENTITY).mul(&w3);
        let (sx, sy) = scales(&l);
        let bone = &mut bones[b];
        bone.x = l.0[4];
        bone.y = l.0[5];
        bone.rotation = rot(&l);
        bone.scale_x = sx;
        bone.scale_y = sy;
    }

    /// New world transform of a constrained item (node or bone) for the non-IK constraint types.
    #[allow(clippy::too_many_arguments)]
    fn solve(
        &self,
        nodes: &[FrameNode],
        kind: &str,
        a: &A,
        w: &Xf,
        pivot_local: P,
        parent: &Xf,
        local: &Xf,
        target: Option<&Target>,
        point: Option<&str>,
    ) -> Option<Xf> {
        let pv = w.apply(pivot_local);
        let off = p(a.num("offsetX", 0.0), a.num("offsetY", 0.0));
        let off_rot = a.num("offsetRotation", 0.0);
        let local_space = a.str("space").as_deref() == Some("local");
        let move_to = |q: P| Xf::translate(q.x - pv.x, q.y - pv.y).mul(w);
        Some(match kind {
            "parent" => self.world(nodes, target?, point)?.mul(local),
            "look-at" => {
                let tp = self.pivot(nodes, target?, point)? + off;
                let want = (tp - pv).angle().to_degrees();
                about(pv, wrap(want - rot(w) + off_rot)).mul(w)
            }
            "follow-path" => {
                let d = a.str("path")?;
                let path = sr_vector::Path::parse(&d).ok()?;
                let polys = path.flatten(0.05);
                // one continuous track through every subpath
                let pts: Vec<P> = polys
                    .iter()
                    .flat_map(|q| {
                        let mut v = q.pts.clone();
                        if q.closed && !v.is_empty() {
                            v.push(v[0]);
                        }
                        v
                    })
                    .collect();
                let len: f64 = pts.windows(2).map(|x| x[0].dist(x[1])).sum();
                let (pt, tg) = sr_vector::measure::at_length(&pts, a.num("progress", 0.0).clamp(0.0, 1.0) * len)?;
                let q = parent.apply(pt + off);
                let w1 = move_to(q);
                if a.bool("autoOrient", false) {
                    let ang = parent.apply_vec(tg).angle().to_degrees();
                    about(q, wrap(ang - rot(&w1) + off_rot)).mul(&w1)
                } else {
                    w1
                }
            }
            "copy-position" => {
                let q = if local_space {
                    let tl = self.local(nodes, target?)?;
                    parent.apply(tl.origin() + off)
                } else {
                    self.pivot(nodes, target?, point)? + off
                };
                move_to(q)
            }
            "copy-rotation" => {
                let tr = if local_space {
                    rot(&self.local(nodes, target?)?) + rot(parent)
                } else {
                    rot(&self.world(nodes, target?, point)?)
                };
                about(pv, wrap(tr + off_rot - rot(w))).mul(w)
            }
            "copy-scale" => {
                let (tx, ty) = if local_space {
                    scales(&self.local(nodes, target?)?)
                } else {
                    scales(&self.world(nodes, target?, point)?)
                };
                let (sx, sy) = scales(w);
                let (fx, fy) =
                    (if sx.abs() > 1e-12 { tx / sx } else { 1.0 }, if sy.abs() > 1e-12 { ty / sy } else { 1.0 });
                w.mul(&Xf::translate(pivot_local.x, pivot_local.y))
                    .mul(&Xf::scale(fx, fy))
                    .mul(&Xf::translate(-pivot_local.x, -pivot_local.y))
            }
            "copy-transform" => {
                let t = if local_space {
                    parent.mul(&self.local(nodes, target?)?)
                } else {
                    self.world(nodes, target?, point)?
                };
                Xf::translate(off.x, off.y).mul(&about(t.apply(pivot_local), off_rot)).mul(&t)
            }
            "distance" => {
                let tp = self.pivot(nodes, target?, point)?;
                let d = pv.dist(tp);
                let lo = a.opt("minDistance").unwrap_or(0.0);
                let hi = a.opt("maxDistance").unwrap_or(f64::INFINITY);
                let dc = d.clamp(lo, hi.max(lo));
                if (dc - d).abs() < 1e-12 {
                    *w
                } else {
                    let dir = if d > 1e-12 { (pv - tp).norm() } else { p(1.0, 0.0) };
                    move_to(tp + dir * dc)
                }
            }
            "track" => {
                let Target::Track(te) = target? else { return None };
                let name = point.map(str::to_string).or_else(|| te.data.first_name().map(str::to_string))?;
                let q = self.track_point(nodes, te, Some(&name))? + off;
                let w1 = move_to(q);
                // planar tracks also carry rotation and scale relative to their first frame
                match (
                    te.data.plane(&name, self.track_time(nodes, te)),
                    te.data.planes.get(&name).and_then(|v| v.first()),
                ) {
                    (Some(now), Some(first)) => {
                        let [s, r, _, _] = track::similarity(&first.1, &now);
                        let (fx, fy) = (s, s);
                        Xf::translate(q.x, q.y)
                            .mul(&Xf::rotate(r + off_rot))
                            .mul(&Xf::scale(fx, fy))
                            .mul(&Xf::translate(-q.x, -q.y))
                            .mul(&w1)
                    }
                    _ => w1,
                }
            }
            _ => return None,
        })
    }
}

fn topo(bones: &[Bone]) -> Vec<usize> {
    let n = bones.len();
    let mut out = Vec::with_capacity(n);
    let mut state = vec![0u8; n];
    fn visit(i: usize, bones: &[Bone], state: &mut [u8], out: &mut Vec<usize>) {
        if state[i] != 0 {
            return;
        }
        state[i] = 1;
        if let Some(pi) = bones[i].parent {
            if state[pi] == 0 {
                visit(pi, bones, state, out);
            }
        }
        state[i] = 2;
        out.push(i);
    }
    for i in 0..n {
        visit(i, bones, &mut state, &mut out);
    }
    out
}

/// Applies `d` to node `i` and its descendants.
fn apply_delta(nodes: &mut [FrameNode], i: usize, d: &Xf) {
    let depth = nodes[i].depth;
    let mut j = i;
    loop {
        nodes[j].world = Affine(d.mul(&xf(&nodes[j].world)).0);
        j += 1;
        if j >= nodes.len() || nodes[j].depth <= depth {
            break;
        }
    }
}

/// Constraints, skeleton poses and stabilisation for one frame.
pub fn post_pass(prog: &Program, nodes: &mut [FrameNode], t: f64) {
    let sc = Scene { p: prog, t };
    // node constraints in paint order
    for i in 0..nodes.len() {
        // constraints inside a skeleton belong to its bones
        if nodes[i].kind == "skeleton" {
            continue;
        }
        let e: Arc<sr_model::model::Node> = nodes[i].elem.clone();
        let mut ck = 0;
        for c in children(&*e) {
            if !is(c, "transformConstraint") {
                continue;
            }
            let key = format!("{}/{}[{ck}]", nodes[i].id, c.element_name());
            ck += 1;
            let props = part(&nodes[i], &key).cloned();
            let a = A { e: c, props: props.as_ref() };
            let kind = a.str("type").unwrap_or_default();
            let influence = a.num("influence", 1.0);
            let point = a.str("point");
            let Some(target) = a.str("target").and_then(|id| sc.find(nodes, &id)) else {
                if kind != "follow-path" {
                    continue;
                }
                let n = &nodes[i];
                let parent = n.parent.map(|pi| xf(&nodes[pi as usize].world)).unwrap_or(Xf::IDENTITY);
                if let Some(w2) = sc.solve(
                    nodes,
                    &kind,
                    &a,
                    &xf(&n.world),
                    p(n.anchor[0], n.anchor[1]),
                    &parent,
                    &xf(&n.local),
                    None,
                    None,
                ) {
                    let w = xf(&n.world);
                    let d = blend(&w, &w2, influence).mul(&w.inverse().unwrap_or(Xf::IDENTITY));
                    apply_delta(nodes, i, &d);
                }
                continue;
            };
            if kind == "ik" {
                node_ik(&sc, nodes, i, &a, &target, point.as_deref(), influence);
                continue;
            }
            let n = &nodes[i];
            let parent = n.parent.map(|pi| xf(&nodes[pi as usize].world)).unwrap_or(Xf::IDENTITY);
            let w = xf(&n.world);
            if let Some(w2) = sc.solve(
                nodes,
                &kind,
                &a,
                &w,
                p(n.anchor[0], n.anchor[1]),
                &parent,
                &xf(&n.local),
                Some(&target),
                point.as_deref(),
            ) {
                let d = blend(&w, &w2, influence).mul(&w.inverse().unwrap_or(Xf::IDENTITY));
                apply_delta(nodes, i, &d);
            }
        }
    }
    // stabilisation of footage layers
    for i in 0..nodes.len() {
        let n = &nodes[i];
        if n.kind != "layer" || !matches!(n.elem.get_attr("stabilize"), Some(AttrValue::Bool(true))) {
            continue;
        }
        let smooth = match n.elem.get_attr("stabilizeSmoothness") {
            Some(AttrValue::Num(v)) => v,
            _ => 0.5,
        };
        let Some(te) =
            n.asset.as_deref().and_then(|a| prog.tracks.values().find(|te| te.footage.as_deref() == Some(a)))
        else {
            continue;
        };
        if let Some(d) = stabilize(&sc, nodes, i, te, smooth) {
            apply_delta(nodes, i, &d);
        }
    }
    // skeletons
    for i in 0..nodes.len() {
        if nodes[i].kind != "skeleton" {
            continue;
        }
        let poses = sc.pose(nodes, i);
        nodes[i].bones = poses
            .into_iter()
            .map(|(id, w, r, l)| BonePose { id: id.into(), world: Affine(w.0), rest: Affine(r.0), length: l })
            .collect();
        nodes[i].skin = prog.skins.get(&nodes[i].id).cloned();
    }
}

/// Two-joint IK on nodes: the parent pivots at its anchor, the node at its
/// own, and the effector (the node's anchor plus offset) reaches the target.
fn node_ik(sc: &Scene, nodes: &mut [FrameNode], i: usize, a: &A, target: &Target, point: Option<&str>, influence: f64) {
    let Some(tp) = sc.pivot(nodes, target, point) else { return };
    let n = &nodes[i];
    let w = xf(&n.world);
    let j1 = w.apply(p(n.anchor[0], n.anchor[1]));
    let eff = w.apply(p(n.anchor[0] + a.num("offsetX", 0.0), n.anchor[1] + a.num("offsetY", 0.0)));
    let Some(pi) = n.parent.map(|x| x as usize) else {
        let r = wrap((tp - j1).angle().to_degrees() - (eff - j1).angle().to_degrees()) * influence;
        apply_delta(nodes, i, &about(j1, r));
        return;
    };
    let pn = &nodes[pi];
    let j0 = xf(&pn.world).apply(p(pn.anchor[0], pn.anchor[1]));
    let (l1, l2) = (j0.dist(j1), j1.dist(eff));
    if l1 < 1e-9 || l2 < 1e-9 {
        return;
    }
    let d = rig::soft_reach((tp - j0).len(), l1, l2, a.num("softness", 0.0)).clamp((l1 - l2).abs() + 1e-9, l1 + l2 - 1e-9);
    let cos_a = ((l1 * l1 + d * d - l2 * l2) / (2.0 * l1 * d)).clamp(-1.0, 1.0);
    let pole = a.str("pole").and_then(|id| sc.find(nodes, &id)).and_then(|t| sc.pivot(nodes, &t, None));
    let sgn = rig::bend_sign(j0, tp, pole, a.bool("bendPositive", true));
    let upper = (tp - j0).angle() - sgn * libm::acos(cos_a);
    let r0 = wrap(upper.to_degrees() - (j1 - j0).angle().to_degrees()) * influence;
    apply_delta(nodes, pi, &about(j0, r0));
    let n = &nodes[i];
    let w = xf(&n.world);
    let j1b = w.apply(p(n.anchor[0], n.anchor[1]));
    let effb = w.apply(p(n.anchor[0] + a.num("offsetX", 0.0), n.anchor[1] + a.num("offsetY", 0.0)));
    let r1 = wrap((tp - j1b).angle().to_degrees() - (effb - j1b).angle().to_degrees()) * influence;
    apply_delta(nodes, i, &about(j1b, r1));
}

/// Correction for a footage layer: the tracked motion minus its smoothed
/// version (Gaussian, σ = 2 × smoothness seconds; smoothness 1 locks to the first sample).
fn stabilize(sc: &Scene, nodes: &[FrameNode], i: usize, te: &TrackEntry, smooth: f64) -> Option<Xf> {
    let n = &nodes[i];
    let name = te.data.first_name()?.to_string();
    let ts = n.source_time.unwrap_or(n.local_time) + te.time_offset;
    let pts_at = |s: f64| -> Option<Vec<P>> {
        match te.data.plane(&name, s) {
            Some(c) => Some(c.to_vec()),
            None => te.data.point(&name, s).map(|q| vec![q]),
        }
    };
    let raw = pts_at(ts)?;
    let target: Vec<P> = if smooth >= 0.999 {
        let f0 = te
            .data
            .points
            .get(&name)
            .and_then(|v| v.first())
            .map(|x| x.0)
            .or(te.data.planes.get(&name).and_then(|v| v.first()).map(|x| x.0))
            .unwrap_or(0.0);
        pts_at(f0 / te.data.fps)?
    } else {
        let sigma = (smooth * 2.0).max(1e-3);
        let dt = 1.0 / te.data.fps.max(1.0);
        let k = ((3.0 * sigma) / dt).ceil() as i64;
        let mut acc = vec![p(0.0, 0.0); raw.len()];
        let mut wsum = 0.0;
        for j in -k..=k {
            let s = ts + j as f64 * dt;
            let w = libm::exp(-0.5 * (j as f64 * dt / sigma).powi(2));
            if let Some(v) = pts_at(s) {
                for (a, q) in acc.iter_mut().zip(&v) {
                    *a = *a + *q * w;
                }
                wsum += w;
            }
        }
        acc.iter().map(|q| *q * (1.0 / wsum.max(1e-12))).collect()
    };
    // correction in footage pixels, mapped to frame space
    let c = if raw.len() >= 2 {
        let [s, r, tx, ty] = track::similarity(&raw, &target);
        Xf::translate(tx, ty).mul(&Xf::rotate(r)).mul(&Xf::scale(s, s))
    } else {
        Xf::translate(target[0].x - raw[0].x, target[0].y - raw[0].y)
    };
    let m = sc.footage_map(n);
    Some(m.mul(&c).mul(&m.inverse()?))
}
