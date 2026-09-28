//! Tracking data: point tracks, planar tracks (four corners) and camera
//! tracks, read from JSON, CSV, Nuke, After Effects keyframe data, Mocha
//! (which exports the After Effects format) and ASCII FBX.

use std::collections::BTreeMap;

use crate::geom::{p, P};

/// Tracking data for one file, keyed by track name.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct TrackData {
    /// Frames per second of the frame numbers.
    pub fps: f64,
    /// Point tracks: (frame, position).
    pub points: BTreeMap<String, Vec<(f64, P)>>,
    /// Planar tracks: (frame, corners TL, TR, BR, BL).
    pub planes: BTreeMap<String, Vec<(f64, [P; 4])>>,
    /// Camera tracks: (frame, [tx, ty, tz, rx, ry, rz, vertical fov degrees]).
    pub cameras: BTreeMap<String, Vec<(f64, [f64; 7])>>,
}

/// Parse errors.
#[derive(Debug, Clone, PartialEq, thiserror::Error)]
#[error("{format} tracking data, line {line}: {message}")]
pub struct TrackError {
    pub format: &'static str,
    pub line: usize,
    pub message: String,
}

fn lerp_track<T: Copy>(v: &[(f64, T)], frame: f64, mix: impl Fn(T, T, f64) -> T) -> Option<T> {
    let first = v.first()?;
    if frame <= first.0 {
        return Some(first.1);
    }
    let last = v.last()?;
    if frame >= last.0 {
        return Some(last.1);
    }
    let k = v.partition_point(|x| x.0 <= frame);
    let (a, b) = (v[k - 1], v[k]);
    let t = if b.0 > a.0 { (frame - a.0) / (b.0 - a.0) } else { 0.0 };
    Some(mix(a.1, b.1, t))
}

impl TrackData {
    /// Point `name` at `seconds` (planar tracks answer with their centre).
    pub fn point(&self, name: &str, seconds: f64) -> Option<P> {
        let f = seconds * self.fps;
        if let Some(v) = self.points.get(name) {
            return lerp_track(v, f, |a, b, t| a.lerp(b, t));
        }
        self.plane(name, seconds).map(|c| (c[0] + c[1] + c[2] + c[3]) * 0.25)
    }
    /// Corners of planar track `name` at `seconds`.
    pub fn plane(&self, name: &str, seconds: f64) -> Option<[P; 4]> {
        lerp_track(self.planes.get(name)?, seconds * self.fps, |a, b, t| [0, 1, 2, 3].map(|k| a[k].lerp(b[k], t)))
    }
    /// Camera `name` at `seconds`.
    pub fn camera(&self, name: &str, seconds: f64) -> Option<[f64; 7]> {
        lerp_track(self.cameras.get(name)?, seconds * self.fps, |a, b, t| {
            let mut o = a;
            for k in 0..7 {
                o[k] = a[k] + (b[k] - a[k]) * t;
            }
            o
        })
    }
    /// Default track name: the first point, plane or camera track.
    pub fn first_name(&self) -> Option<&str> {
        self.points.keys().chain(self.planes.keys()).chain(self.cameras.keys()).next().map(String::as_str)
    }

    /// Parses `text` in `format` (`json`, `csv`, `nuke`, `after-effects`, `mocha`, `fbx`).
    pub fn parse(text: &str, format: &str) -> Result<TrackData, TrackError> {
        let mut d = match format {
            "json" => parse_json(text),
            "csv" => parse_csv(text),
            "nuke" => parse_nuke(text),
            "after-effects" | "mocha" => parse_ae(text, if format == "mocha" { "Mocha" } else { "After Effects" }),
            "fbx" => parse_fbx(text),
            other => Err(TrackError { format: "unknown", line: 0, message: format!("unknown format {other}") }),
        }?;
        if d.fps <= 0.0 {
            d.fps = 24.0;
        }
        for v in d.points.values_mut() {
            v.sort_by(|a, b| a.0.total_cmp(&b.0));
        }
        for v in d.planes.values_mut() {
            v.sort_by(|a, b| a.0.total_cmp(&b.0));
        }
        for v in d.cameras.values_mut() {
            v.sort_by(|a, b| a.0.total_cmp(&b.0));
        }
        Ok(d)
    }
}

/// JSON: `{"fps": 24, "points": {"name": [[frame, x, y], …]}, "planes": {"name": [[frame, x0, y0, …, x3, y3], …]},
/// "cameras": {"name": [[frame, tx, ty, tz, rx, ry, rz, fov], …]}}`.
fn parse_json(text: &str) -> Result<TrackData, TrackError> {
    let e = |m: String| TrackError { format: "JSON", line: 0, message: m };
    let v: serde_json::Value = serde_json::from_str(text).map_err(|x| TrackError {
        format: "JSON",
        line: x.line(),
        message: x.to_string(),
    })?;
    let mut d = TrackData { fps: v["fps"].as_f64().unwrap_or(24.0), ..Default::default() };
    let rows = |x: &serde_json::Value, n: usize, what: &str| -> Result<Vec<Vec<f64>>, TrackError> {
        x.as_array()
            .ok_or_else(|| e(format!("{what} must be an array of rows")))?
            .iter()
            .map(|r| {
                let r: Vec<f64> =
                    r.as_array().map(|a| a.iter().filter_map(|x| x.as_f64()).collect()).unwrap_or_default();
                if r.len() < n {
                    Err(e(format!("{what}: each row needs {n} numbers")))
                } else {
                    Ok(r)
                }
            })
            .collect()
    };
    if let Some(m) = v["points"].as_object() {
        for (k, x) in m {
            d.points.insert(k.clone(), rows(x, 3, k)?.into_iter().map(|r| (r[0], p(r[1], r[2]))).collect());
        }
    }
    if let Some(m) = v["planes"].as_object() {
        for (k, x) in m {
            d.planes.insert(
                k.clone(),
                rows(x, 9, k)?
                    .into_iter()
                    .map(|r| (r[0], [p(r[1], r[2]), p(r[3], r[4]), p(r[5], r[6]), p(r[7], r[8])]))
                    .collect(),
            );
        }
    }
    if let Some(m) = v["cameras"].as_object() {
        for (k, x) in m {
            d.cameras.insert(
                k.clone(),
                rows(x, 7, k)?
                    .into_iter()
                    .map(|r| (r[0], [r[1], r[2], r[3], r[4], r[5], r[6], r.get(7).copied().unwrap_or(39.6)]))
                    .collect(),
            );
        }
    }
    Ok(d)
}

/// CSV with a header: `frame,track,x,y` for points, plus `x0,y0,…,x3,y3` for planes.
fn parse_csv(text: &str) -> Result<TrackData, TrackError> {
    let mut lines = text.lines().enumerate().filter(|(_, l)| !l.trim().is_empty() && !l.trim_start().starts_with('#'));
    let (_, header) = lines.next().ok_or(TrackError { format: "CSV", line: 1, message: "empty file".into() })?;
    let cols: Vec<String> = header.split(',').map(|c| c.trim().to_ascii_lowercase()).collect();
    let col = |n: &str| cols.iter().position(|c| c == n);
    let fcol = col("frame").ok_or(TrackError { format: "CSV", line: 1, message: "missing a frame column".into() })?;
    let tcol = col("track").or_else(|| col("point")).or_else(|| col("name"));
    let (xc, yc) = (col("x"), col("y"));
    let corners: Option<Vec<usize>> = ["x0", "y0", "x1", "y1", "x2", "y2", "x3", "y3"].iter().map(|n| col(n)).collect();
    let mut d = TrackData { fps: 24.0, ..Default::default() };
    for (ln, l) in lines {
        let f: Vec<&str> = l.split(',').map(str::trim).collect();
        let num = |i: usize| -> Result<f64, TrackError> {
            f.get(i).and_then(|s| s.parse().ok()).ok_or(TrackError {
                format: "CSV",
                line: ln + 1,
                message: format!("column {} is not a number", i + 1),
            })
        };
        let name = tcol.and_then(|i| f.get(i)).map(|s| s.to_string()).unwrap_or_else(|| "track".into());
        let frame = num(fcol)?;
        if let Some(cs) = &corners {
            let v: Vec<f64> = cs.iter().map(|&i| num(i)).collect::<Result<_, _>>()?;
            d.planes
                .entry(name)
                .or_default()
                .push((frame, [p(v[0], v[1]), p(v[2], v[3]), p(v[4], v[5]), p(v[6], v[7])]));
        } else if let (Some(x), Some(y)) = (xc, yc) {
            d.points.entry(name).or_default().push((frame, p(num(x)?, num(y)?)));
        } else {
            return Err(TrackError { format: "CSV", line: 1, message: "needs x,y or x0..y3 columns".into() });
        }
    }
    Ok(d)
}

/// Values of an animated Nuke knob: `{curve x1 v v v x10 v}` or a constant.
fn nuke_curve(s: &str) -> Vec<(f64, f64)> {
    let s = s.trim().trim_start_matches('{').trim_end_matches('}').trim();
    let body = s.strip_prefix("curve").unwrap_or(s);
    let mut out = Vec::new();
    let mut frame = 1.0;
    for tok in body.split_whitespace() {
        if let Some(f) = tok.strip_prefix('x') {
            if let Ok(f) = f.parse() {
                frame = f;
            }
        } else if let Ok(v) = tok.parse::<f64>() {
            out.push((frame, v));
            frame += 1.0;
        }
    }
    out
}

/// Splits `{a} {b}` groups at the top level of a brace expression.
fn brace_groups(s: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let (mut depth, mut start) = (0i32, None);
    for (i, c) in s.char_indices() {
        match c {
            '{' => {
                if depth == 0 {
                    start = Some(i);
                }
                depth += 1;
            }
            '}' => {
                depth -= 1;
                if depth == 0 {
                    if let Some(st) = start.take() {
                        out.push(&s[st..=i]);
                    }
                }
            }
            _ => {}
        }
    }
    out
}

/// Nuke: knobs `name {{curve x1 …} {curve x1 …}}` holding an x/y pair
/// (Tracker `track1`…, Transform `translate`, CornerPin `to1`…`to4`), or a
/// `.chan` camera file (`frame tx ty tz rx ry rz [vfov]` per line).
fn parse_nuke(text: &str) -> Result<TrackData, TrackError> {
    let mut d = TrackData { fps: 24.0, ..Default::default() };
    let chan =
        text.lines().filter(|l| !l.trim().is_empty()).all(|l| l.split_whitespace().all(|t| t.parse::<f64>().is_ok()));
    if chan {
        for (ln, l) in text.lines().enumerate().filter(|(_, l)| !l.trim().is_empty()) {
            let v: Vec<f64> = l.split_whitespace().filter_map(|t| t.parse().ok()).collect();
            if v.len() < 7 {
                return Err(TrackError {
                    format: "Nuke .chan",
                    line: ln + 1,
                    message: "expected frame tx ty tz rx ry rz [fov]".into(),
                });
            }
            d.cameras
                .entry("camera".into())
                .or_default()
                .push((v[0], [v[1], v[2], v[3], v[4], v[5], v[6], v.get(7).copied().unwrap_or(39.6)]));
        }
        return Ok(d);
    }
    let mut corners: BTreeMap<usize, Vec<(f64, P)>> = BTreeMap::new();
    for l in text.lines() {
        let t = l.trim();
        let Some((name, rest)) = t.split_once(char::is_whitespace) else { continue };
        let rest = rest.trim();
        if !rest.starts_with("{{") && !rest.starts_with("{ {") {
            continue;
        }
        let inner = &rest[1..rest.len().saturating_sub(1)];
        let groups = brace_groups(inner);
        if groups.len() < 2 {
            continue;
        }
        let (xs, ys) = (nuke_curve(groups[0]), nuke_curve(groups[1]));
        let pts: Vec<(f64, P)> = xs.iter().zip(&ys).map(|(a, b)| (a.0, p(a.1, b.1))).collect();
        if let Some(k) = name.strip_prefix("to").and_then(|k| k.parse::<usize>().ok()).filter(|k| (1..=4).contains(k)) {
            corners.insert(k, pts);
        } else if !pts.is_empty() {
            d.points.insert(name.to_string(), pts);
        }
    }
    if corners.len() == 4 {
        let n = corners.values().map(Vec::len).min().unwrap_or(0);
        let v: Vec<(f64, [P; 4])> = (0..n)
            .map(|i| (corners[&1][i].0, [corners[&1][i].1, corners[&2][i].1, corners[&3][i].1, corners[&4][i].1]))
            .collect();
        d.planes.insert("cornerpin".into(), v);
    }
    Ok(d)
}

/// After Effects keyframe clipboard text (also Mocha's AE export): tab-separated
/// sections such as `Motion Trackers\tTracker #1\tTrack Point #1\tFeature Center`,
/// `Transform\tPosition` or corner pin `Effects\tCorner Pin #1\tUpper Left`.
fn parse_ae(text: &str, format: &'static str) -> Result<TrackData, TrackError> {
    let fmt = if format == "Mocha" { "Mocha" } else { "After Effects" };
    let mut d = TrackData { fps: 24.0, ..Default::default() };
    if !text.contains("Keyframe Data") && !text.contains("Frame") {
        return Err(TrackError { format: fmt, line: 1, message: "not After Effects keyframe data".into() });
    }
    let mut section: Option<String> = None;
    let mut corner_tracks: BTreeMap<usize, Vec<(f64, P)>> = BTreeMap::new();
    for (ln, raw) in text.lines().enumerate() {
        let l = raw.trim_end();
        let cells: Vec<&str> = l.split('\t').map(str::trim).collect();
        if l.trim().is_empty() {
            continue;
        }
        if cells.iter().any(|c| c.starts_with("Units Per Second")) {
            if let Some(v) = cells.iter().skip(1).find_map(|c| c.parse::<f64>().ok()) {
                d.fps = v;
            }
            continue;
        }
        if !l.starts_with('\t') {
            // a section header: its last cells name the property
            let named: Vec<&str> = cells.iter().copied().filter(|c| !c.is_empty()).collect();
            section = (named.len() >= 2).then(|| named.join("/"));
            continue;
        }
        let Some(sec) = &section else { continue };
        let nums: Vec<f64> = cells.iter().filter_map(|c| c.parse::<f64>().ok()).collect();
        if nums.len() < 3 {
            continue;
        }
        let (frame, pt) = (nums[0], p(nums[1], nums[2]));
        let lower = sec.to_ascii_lowercase();
        let corner =
            ["upper left", "upper right", "lower right", "lower left"].iter().position(|c| lower.ends_with(c)).or_else(
                || ["top left", "top right", "bottom right", "bottom left"].iter().position(|c| lower.ends_with(c)),
            );
        match corner {
            Some(k) => corner_tracks.entry(k + 1).or_default().push((frame, pt)),
            None => {
                if lower.contains("scale") || lower.contains("rotation") {
                    continue;
                }
                d.points.entry(sec.clone()).or_default().push((frame, pt));
            }
        }
        let _ = ln;
    }
    if corner_tracks.len() == 4 {
        let n = corner_tracks.values().map(Vec::len).min().unwrap_or(0);
        let c = |k: usize, i: usize| corner_tracks[&k][i];
        d.planes.insert(
            "cornerpin".into(),
            (0..n).map(|i| (c(1, i).0, [c(1, i).1, c(2, i).1, c(3, i).1, c(4, i).1])).collect(),
        );
    }
    if d.points.is_empty() && d.planes.is_empty() {
        return Err(TrackError { format: fmt, line: 1, message: "no position or corner-pin keyframes found".into() });
    }
    // short names: the last path component of each track
    let renamed: BTreeMap<String, Vec<(f64, P)>> = std::mem::take(&mut d.points)
        .into_iter()
        .map(|(k, v)| {
            let parts: Vec<&str> = k.split('/').collect();
            let short = if parts.len() >= 2 { parts[parts.len() - 2].to_string() } else { k.clone() };
            (if short.is_empty() { k } else { short }, v)
        })
        .collect();
    d.points = renamed;
    Ok(d)
}

/// ASCII FBX: model translation and rotation curves (`Lcl Translation`,
/// `Lcl Rotation`) keyed by model name; times in FBX ticks (46186158000 per second).
fn parse_fbx(text: &str) -> Result<TrackData, TrackError> {
    const TICKS: f64 = 46_186_158_000.0;
    if text.starts_with("Kaydara FBX Binary") {
        return Err(TrackError {
            format: "FBX",
            line: 1,
            message: "binary FBX is not supported; export ASCII FBX".into(),
        });
    }
    let numbers = |s: &str| -> Vec<f64> {
        s.split(|c: char| c == ',' || c.is_whitespace()).filter_map(|t| t.trim().parse::<f64>().ok()).collect()
    };
    let mut models: BTreeMap<i64, String> = BTreeMap::new();
    let mut curves: BTreeMap<i64, (Vec<f64>, Vec<f64>)> = BTreeMap::new();
    let mut conns: Vec<(i64, i64, String)> = Vec::new();
    let lines: Vec<&str> = text.lines().collect();
    let mut i = 0;
    let id_of = |l: &str| -> Option<i64> { l.split(':').nth(1)?.split(',').next()?.trim().parse().ok() };
    while i < lines.len() {
        let l = lines[i].trim();
        if l.starts_with("Model:") {
            if let Some(id) = id_of(l) {
                let name = l.split('"').nth(1).unwrap_or("model").trim_start_matches("Model::").to_string();
                models.insert(id, name);
            }
        } else if l.starts_with("AnimationCurve:") {
            if let Some(id) = id_of(l) {
                let (mut times, mut vals) = (Vec::new(), Vec::new());
                let mut j = i + 1;
                while j < lines.len()
                    && !lines[j].trim().starts_with("AnimationCurve:")
                    && !lines[j].trim().starts_with("AnimationCurveNode:")
                {
                    let t = lines[j].trim();
                    if t.starts_with("KeyTime:") || t.starts_with("KeyValueFloat:") {
                        let mut buf = String::new();
                        let mut k = j + 1;
                        while k < lines.len() && !lines[k].contains('}') {
                            buf.push_str(lines[k].trim().trim_start_matches("a:"));
                            buf.push(' ');
                            k += 1;
                        }
                        if k < lines.len() {
                            buf.push_str(lines[k].split('}').next().unwrap_or("").trim().trim_start_matches("a:"));
                        }
                        let v = numbers(&buf);
                        if t.starts_with("KeyTime:") {
                            times = v;
                        } else {
                            vals = v;
                        }
                        j = k;
                    }
                    j += 1;
                    if lines.get(j).is_some_and(|x| x.trim() == "}") && !times.is_empty() && !vals.is_empty() {
                        break;
                    }
                }
                curves.insert(id, (times, vals));
            }
        } else if let Some(rest) = l.strip_prefix("C:") {
            let parts: Vec<&str> = rest.split(',').map(|s| s.trim().trim_matches('"')).collect();
            if parts.len() >= 3 {
                if let (Ok(a), Ok(b)) = (parts[1].parse::<i64>(), parts[2].parse::<i64>()) {
                    conns.push((a, b, parts.get(3).map(|s| s.to_string()).unwrap_or_default()));
                }
            }
        }
        i += 1;
    }
    // curve → curve node (channel d|X/Y/Z); curve node → model (Lcl Translation / Lcl Rotation)
    let mut d = TrackData { fps: 24.0, ..Default::default() };
    let mut tracks: BTreeMap<String, BTreeMap<i64, [f64; 7]>> = BTreeMap::new();
    for (node, model, prop) in conns.iter().filter(|c| models.contains_key(&c.1) && c.2.starts_with("Lcl ")) {
        let base = if prop.contains("Translation") {
            0
        } else if prop.contains("Rotation") {
            3
        } else {
            continue;
        };
        for (curve, _, ch) in conns.iter().filter(|c| c.1 == *node && curves.contains_key(&c.0)) {
            let axis = match ch.as_str() {
                "d|X" => 0,
                "d|Y" => 1,
                "d|Z" => 2,
                _ => continue,
            };
            let (times, vals) = &curves[curve];
            let entry = tracks.entry(models[model].clone()).or_default();
            for (t, v) in times.iter().zip(vals) {
                let key = (t / TICKS * 1000.0).round() as i64;
                entry.entry(key).or_insert([0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 39.6])[base + axis] = *v;
            }
        }
    }
    if tracks.is_empty() {
        return Err(TrackError {
            format: "FBX",
            line: 1,
            message: "no Lcl Translation or Lcl Rotation curves on any model".into(),
        });
    }
    for (name, keys) in tracks {
        let v: Vec<(f64, [f64; 7])> = keys.into_iter().map(|(ms, x)| (ms as f64 / 1000.0 * d.fps, x)).collect();
        d.points.insert(name.clone(), v.iter().map(|(f, x)| (*f, p(x[0], x[1]))).collect());
        d.cameras.insert(name, v);
    }
    Ok(d)
}

/// Similarity transform (scale, rotation, translation) best mapping `a` to `b` (least squares).
pub fn similarity(a: &[P], b: &[P]) -> [f64; 4] {
    let n = a.len().min(b.len()).max(1) as f64;
    let ca = a.iter().fold(p(0.0, 0.0), |s, &q| s + q) * (1.0 / n);
    let cb = b.iter().fold(p(0.0, 0.0), |s, &q| s + q) * (1.0 / n);
    let (mut sdot, mut scross, mut saa) = (0.0, 0.0, 0.0);
    for (&x, &y) in a.iter().zip(b) {
        let (u, v) = (x - ca, y - cb);
        sdot += u.dot(v);
        scross += u.cross(v);
        saa += u.dot(u);
    }
    let ang = libm::atan2(scross, sdot);
    let scale = if saa > 0.0 { libm::hypot(sdot, scross) / saa } else { 1.0 };
    // returns (scale, rotation degrees, tx, ty) mapping a's centroid to b's
    let r = (ca * scale).rot(ang);
    [scale, ang.to_degrees(), cb.x - r.x, cb.y - r.y]
}
