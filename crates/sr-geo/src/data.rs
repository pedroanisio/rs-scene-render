//! Geographic data: GeoJSON (RFC 7946), TopoJSON, KML and GPX, read into
//! features with longitude/latitude coordinates in degrees.
//!
//! Polygons follow d3-geo's convention (exterior rings clockwise). Data wound
//! the other way, as RFC 7946 asks for, would enclose the rest of the globe,
//! so any polygon larger than a hemisphere is turned round when it is read.

use std::path::Path;

use serde_json::{Map, Value};

use crate::sphere::{self, RAD};

/// A geometry; coordinates are [longitude, latitude] in degrees.
#[derive(Clone, Debug, PartialEq)]
pub enum Geometry {
    /// Points.
    Points(Vec<[f64; 2]>),
    /// Line strings.
    Lines(Vec<Vec<[f64; 2]>>),
    /// Polygons, each exterior ring first, then holes.
    Polygons(Vec<Vec<Vec<[f64; 2]>>>),
    /// Mixed geometries.
    Collection(Vec<Geometry>),
    /// The whole sphere (d3's `{type: "Sphere"}`).
    Sphere,
}

impl Geometry {
    /// Longitude/latitude bounds ([[west, south], [east, north]]), ignoring the antimeridian.
    pub fn extent(&self) -> Option<[[f64; 2]; 2]> {
        let mut b = [[f64::INFINITY; 2], [f64::NEG_INFINITY; 2]];
        let mut add = |p: &[f64; 2]| {
            b[0][0] = b[0][0].min(p[0]);
            b[0][1] = b[0][1].min(p[1]);
            b[1][0] = b[1][0].max(p[0]);
            b[1][1] = b[1][1].max(p[1]);
        };
        fn walk(g: &Geometry, add: &mut dyn FnMut(&[f64; 2])) {
            match g {
                Geometry::Points(ps) => ps.iter().for_each(add),
                Geometry::Lines(ls) => ls.iter().flatten().for_each(add),
                Geometry::Polygons(ps) => ps.iter().flatten().flatten().for_each(add),
                Geometry::Collection(gs) => gs.iter().for_each(|g| walk(g, add)),
                Geometry::Sphere => {
                    add(&[-180.0, -90.0]);
                    add(&[180.0, 90.0]);
                }
            }
        }
        walk(self, &mut add);
        (b[0][0] <= b[1][0]).then_some(b)
    }
}

/// A feature: geometry, identifier and properties.
#[derive(Clone, Debug, PartialEq)]
pub struct Feature {
    pub id: Option<String>,
    pub properties: Map<String, Value>,
    pub geometry: Geometry,
}

impl Feature {
    /// A property as text (`id` is the feature identifier unless a property has that name).
    pub fn text(&self, name: &str) -> Option<String> {
        match self.properties.get(name) {
            Some(Value::String(s)) => Some(s.clone()),
            Some(Value::Null) | None => (name == "id").then(|| self.id.clone()).flatten(),
            Some(v) => Some(v.to_string()),
        }
    }

    /// A property as a number (numeric strings count).
    pub fn number(&self, name: &str) -> Option<f64> {
        match self.properties.get(name)? {
            Value::Number(n) => n.as_f64(),
            Value::String(s) => s.trim().parse().ok(),
            Value::Bool(b) => Some(*b as u8 as f64),
            _ => None,
        }
    }
}

/// Input formats.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Format {
    GeoJson,
    TopoJson,
    Kml,
    Gpx,
}

impl Format {
    /// The format of a file from its extension, then its content.
    pub fn detect(path: &Path, text: &str) -> Option<Format> {
        let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("").to_ascii_lowercase();
        match ext.as_str() {
            "kml" => return Some(Format::Kml),
            "gpx" => return Some(Format::Gpx),
            "topojson" => return Some(Format::TopoJson),
            "geojson" => return Some(Format::GeoJson),
            _ => {}
        }
        let head = text.trim_start();
        if head.starts_with('<') {
            if head.contains("<kml") {
                return Some(Format::Kml);
            }
            if head.contains("<gpx") {
                return Some(Format::Gpx);
            }
            return None;
        }
        if head.starts_with('{') {
            return Some(if text.contains("\"Topology\"") { Format::TopoJson } else { Format::GeoJson });
        }
        None
    }

    /// Parses a format name (`geojson`, `topojson`, `kml`, `gpx`).
    pub fn parse(s: &str) -> Option<Format> {
        Some(match s {
            "geojson" => Format::GeoJson,
            "topojson" => Format::TopoJson,
            "kml" => Format::Kml,
            "gpx" => Format::Gpx,
            _ => return None,
        })
    }
}

/// Reads a file. `object` picks one TopoJSON object (all objects when `None`).
pub fn load(path: &Path, format: Option<Format>, object: Option<&str>) -> Result<Vec<Feature>, String> {
    let text = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let format = format
        .or_else(|| Format::detect(path, &text))
        .ok_or_else(|| format!("{}: not GeoJSON, TopoJSON, KML or GPX", path.display()))?;
    parse(&text, format, object).map_err(|e| format!("{}: {e}", path.display()))
}

/// Parses text in a format.
pub fn parse(text: &str, format: Format, object: Option<&str>) -> Result<Vec<Feature>, String> {
    let mut features = match format {
        Format::GeoJson => geojson(&serde_json::from_str(text).map_err(|e| e.to_string())?)?,
        Format::TopoJson => topojson(&serde_json::from_str(text).map_err(|e| e.to_string())?, object)?,
        Format::Kml => kml(text)?,
        Format::Gpx => gpx(text)?,
    };
    for f in &mut features {
        rewind(&mut f.geometry);
    }
    Ok(features)
}

/// Turns round polygons that enclose more than a hemisphere (RFC 7946 winding).
fn rewind(g: &mut Geometry) {
    match g {
        Geometry::Polygons(polys) => {
            for rings in polys {
                let rad: Vec<Vec<[f64; 2]>> = rings
                    .iter()
                    .map(|r| {
                        let open = if r.len() > 1 && r.first() == r.last() { &r[..r.len() - 1] } else { &r[..] };
                        open.iter().map(|p| [p[0] * RAD, p[1] * RAD]).collect()
                    })
                    .collect();
                if sphere::polygon_area(&rad) > 2.0 * std::f64::consts::PI {
                    rings.iter_mut().for_each(|r| r.reverse());
                }
            }
        }
        Geometry::Collection(gs) => gs.iter_mut().for_each(rewind),
        _ => {}
    }
}

// ------------------------------------------------------------------ GeoJSON

fn position(v: &Value) -> Result<[f64; 2], String> {
    let a = v.as_array().ok_or("position is not an array")?;
    match (a.first().and_then(Value::as_f64), a.get(1).and_then(Value::as_f64)) {
        (Some(x), Some(y)) => Ok([x, y]),
        _ => Err("position needs two numbers".into()),
    }
}

fn positions(v: &Value) -> Result<Vec<[f64; 2]>, String> {
    v.as_array().ok_or("expected an array of positions")?.iter().map(position).collect()
}

fn rings(v: &Value) -> Result<Vec<Vec<[f64; 2]>>, String> {
    v.as_array().ok_or("expected an array of rings")?.iter().map(positions).collect()
}

fn geometry(v: &Value) -> Result<Option<Geometry>, String> {
    if v.is_null() {
        return Ok(None);
    }
    let c = &v["coordinates"];
    Ok(Some(match v["type"].as_str().unwrap_or("") {
        "Point" => Geometry::Points(vec![position(c)?]),
        "MultiPoint" => Geometry::Points(positions(c)?),
        "LineString" => Geometry::Lines(vec![positions(c)?]),
        "MultiLineString" => Geometry::Lines(rings(c)?),
        "Polygon" => Geometry::Polygons(vec![rings(c)?]),
        "MultiPolygon" => Geometry::Polygons(
            c.as_array().ok_or("MultiPolygon needs an array")?.iter().map(rings).collect::<Result<_, _>>()?,
        ),
        "GeometryCollection" => Geometry::Collection(
            v["geometries"]
                .as_array()
                .ok_or("GeometryCollection needs geometries")?
                .iter()
                .filter_map(|g| geometry(g).transpose())
                .collect::<Result<_, _>>()?,
        ),
        "Sphere" => Geometry::Sphere,
        other => return Err(format!("unknown geometry type {other:?}")),
    }))
}

fn id_of(v: &Value) -> Option<String> {
    match v {
        Value::String(s) => Some(s.clone()),
        Value::Number(n) => Some(n.to_string()),
        _ => None,
    }
}

fn props(v: &Value) -> Map<String, Value> {
    v.as_object().cloned().unwrap_or_default()
}

fn geojson(v: &Value) -> Result<Vec<Feature>, String> {
    match v["type"].as_str().unwrap_or("") {
        "FeatureCollection" => {
            let fs = v["features"].as_array().ok_or("FeatureCollection needs features")?;
            let mut out = Vec::new();
            for f in fs {
                out.extend(geojson(f)?);
            }
            Ok(out)
        }
        "Feature" => Ok(geometry(&v["geometry"])?
            .map(|g| Feature { id: id_of(&v["id"]), properties: props(&v["properties"]), geometry: g })
            .into_iter()
            .collect()),
        _ => Ok(geometry(v)?.map(|g| Feature { id: None, properties: Map::new(), geometry: g }).into_iter().collect()),
    }
}

// ------------------------------------------------------------------ TopoJSON

/// Decodes TopoJSON (as topojson-client's `feature`).
fn topojson(v: &Value, object: Option<&str>) -> Result<Vec<Feature>, String> {
    let objects = v["objects"].as_object().ok_or("Topology has no objects")?;
    let (scale, translate) = match v.get("transform") {
        Some(t) if t.is_object() => (position(&t["scale"])?, position(&t["translate"])?),
        _ => ([1.0, 1.0], [0.0, 0.0]),
    };
    let quantized = v.get("transform").is_some_and(Value::is_object);
    let arcs: Vec<Vec<[f64; 2]>> = v["arcs"]
        .as_array()
        .ok_or("Topology has no arcs")?
        .iter()
        .map(|a| {
            let raw = positions(a)?;
            let (mut x, mut y) = (0.0, 0.0);
            Ok(raw
                .into_iter()
                .map(|p| {
                    if quantized {
                        x += p[0];
                        y += p[1];
                        [x * scale[0] + translate[0], y * scale[1] + translate[1]]
                    } else {
                        p
                    }
                })
                .collect())
        })
        .collect::<Result<_, String>>()?;
    let point =
        |p: [f64; 2]| if quantized { [p[0] * scale[0] + translate[0], p[1] * scale[1] + translate[1]] } else { p };
    let arc_index = |v: &Value| v.as_i64().ok_or_else(|| "arc index is not an integer".to_string());
    let line = |refs: &Value| -> Result<Vec<[f64; 2]>, String> {
        let mut pts: Vec<[f64; 2]> = Vec::new();
        for r in refs.as_array().ok_or("arcs is not an array")? {
            let i = arc_index(r)?;
            let a = arcs.get(if i < 0 { !i } else { i } as usize).ok_or("arc index out of range")?;
            pts.pop();
            let start = pts.len();
            pts.extend_from_slice(a);
            if i < 0 {
                pts[start..].reverse();
            }
        }
        if pts.len() == 1 {
            pts.push(pts[0]);
        }
        Ok(pts)
    };
    let ring = |refs: &Value| -> Result<Vec<[f64; 2]>, String> {
        let mut pts = line(refs)?;
        while !pts.is_empty() && pts.len() < 4 {
            pts.push(pts[0]);
        }
        Ok(pts)
    };
    let polygon = |refs: &Value| -> Result<Vec<Vec<[f64; 2]>>, String> {
        refs.as_array().ok_or("polygon arcs is not an array")?.iter().map(ring).collect()
    };
    type Line = Result<Vec<[f64; 2]>, String>;
    type Rings = Result<Vec<Vec<[f64; 2]>>, String>;
    fn walk(
        o: &Value,
        point: &dyn Fn([f64; 2]) -> [f64; 2],
        line: &dyn Fn(&Value) -> Line,
        polygon: &dyn Fn(&Value) -> Rings,
        out: &mut Vec<Feature>,
    ) -> Result<(), String> {
        let a = &o["arcs"];
        let each = |v: &Value| v.as_array().cloned().unwrap_or_default();
        let g = match o["type"].as_str().unwrap_or("") {
            "GeometryCollection" => {
                for g in o["geometries"].as_array().ok_or("GeometryCollection needs geometries")? {
                    walk(g, point, line, polygon, out)?;
                }
                return Ok(());
            }
            "Point" => Geometry::Points(vec![point(position(&o["coordinates"])?)]),
            "MultiPoint" => Geometry::Points(positions(&o["coordinates"])?.into_iter().map(point).collect()),
            "LineString" => Geometry::Lines(vec![line(a)?]),
            "MultiLineString" => Geometry::Lines(each(a).iter().map(line).collect::<Result<_, _>>()?),
            "Polygon" => Geometry::Polygons(vec![polygon(a)?]),
            "MultiPolygon" => Geometry::Polygons(each(a).iter().map(polygon).collect::<Result<_, _>>()?),
            _ => return Ok(()),
        };
        out.push(Feature { id: id_of(&o["id"]), properties: props(&o["properties"]), geometry: g });
        Ok(())
    }
    let mut out = Vec::new();
    match object {
        Some(name) => {
            let o = objects.get(name).ok_or_else(|| {
                format!("no object {name:?} (has {})", objects.keys().cloned().collect::<Vec<_>>().join(", "))
            })?;
            walk(o, &point, &line, &polygon, &mut out)?;
        }
        None => {
            for o in objects.values() {
                walk(o, &point, &line, &polygon, &mut out)?;
            }
        }
    }
    Ok(out)
}

// ------------------------------------------------------------------ KML and GPX

fn local<'a>(n: &roxmltree::Node<'a, '_>) -> &'a str {
    n.tag_name().name()
}

fn child<'a, 'i>(n: &roxmltree::Node<'a, 'i>, name: &str) -> Option<roxmltree::Node<'a, 'i>> {
    n.children().find(|c| c.is_element() && local(c) == name)
}

fn child_text(n: &roxmltree::Node, name: &str) -> Option<String> {
    child(n, name).and_then(|c| c.text()).map(|t| t.trim().to_string())
}

fn kml_coords(n: &roxmltree::Node) -> Vec<[f64; 2]> {
    child(n, "coordinates")
        .and_then(|c| c.text())
        .map(|t| {
            t.split_whitespace()
                .filter_map(|tuple| {
                    let mut it = tuple.split(',').map(|v| v.trim().parse::<f64>());
                    match (it.next(), it.next()) {
                        (Some(Ok(x)), Some(Ok(y))) => Some([x, y]),
                        _ => None,
                    }
                })
                .collect()
        })
        .unwrap_or_default()
}

fn kml_geometry(n: &roxmltree::Node) -> Option<Geometry> {
    Some(match local(n) {
        "Point" => Geometry::Points(kml_coords(n)),
        "LineString" | "LinearRing" => Geometry::Lines(vec![kml_coords(n)]),
        "Polygon" => {
            let ring = |b: roxmltree::Node| child(&b, "LinearRing").map(|r| kml_coords(&r));
            let mut rings: Vec<Vec<[f64; 2]>> =
                n.children().filter(|c| c.is_element() && local(c) == "outerBoundaryIs").filter_map(ring).collect();
            rings.extend(n.children().filter(|c| c.is_element() && local(c) == "innerBoundaryIs").filter_map(ring));
            Geometry::Polygons(vec![rings])
        }
        "Track" => {
            let pts = n
                .children()
                .filter(|c| c.is_element() && local(c) == "coord")
                .filter_map(|c| {
                    let t = c.text()?;
                    let mut it = t.split_whitespace().map(|v| v.parse::<f64>());
                    match (it.next(), it.next()) {
                        (Some(Ok(x)), Some(Ok(y))) => Some([x, y]),
                        _ => None,
                    }
                })
                .collect();
            Geometry::Lines(vec![pts])
        }
        "MultiGeometry" | "MultiTrack" => {
            Geometry::Collection(n.children().filter(|c| c.is_element()).filter_map(|c| kml_geometry(&c)).collect())
        }
        _ => return None,
    })
}

fn kml(text: &str) -> Result<Vec<Feature>, String> {
    let doc = roxmltree::Document::parse(text).map_err(|e| e.to_string())?;
    let mut out = Vec::new();
    for pm in doc.descendants().filter(|n| n.is_element() && local(n) == "Placemark") {
        let Some(g) = pm.children().filter(|c| c.is_element()).find_map(|c| kml_geometry(&c)) else { continue };
        let mut properties = Map::new();
        for key in ["name", "description"] {
            if let Some(v) = child_text(&pm, key) {
                properties.insert(key.into(), Value::String(v));
            }
        }
        if let Some(ext) = child(&pm, "ExtendedData") {
            for d in ext.descendants().filter(|d| d.is_element()) {
                let (name, value) = match local(&d) {
                    "Data" => (d.attribute("name"), child_text(&d, "value")),
                    "SimpleData" => (d.attribute("name"), d.text().map(|t| t.trim().to_string())),
                    _ => continue,
                };
                if let (Some(n), Some(v)) = (name, value) {
                    properties.insert(n.into(), Value::String(v));
                }
            }
        }
        out.push(Feature { id: pm.attribute("id").map(str::to_string), properties, geometry: g });
    }
    Ok(out)
}

fn gpx(text: &str) -> Result<Vec<Feature>, String> {
    let doc = roxmltree::Document::parse(text).map_err(|e| e.to_string())?;
    let pt = |n: &roxmltree::Node| -> Option<[f64; 2]> {
        Some([n.attribute("lon")?.trim().parse().ok()?, n.attribute("lat")?.trim().parse().ok()?])
    };
    let named = |n: &roxmltree::Node, kind: &str| {
        let mut p = Map::new();
        p.insert("kind".into(), Value::String(kind.into()));
        for key in ["name", "desc", "type", "ele", "time"] {
            if let Some(v) = child_text(n, key) {
                p.insert(key.into(), Value::String(v));
            }
        }
        p
    };
    let root = doc.root_element();
    let mut out = Vec::new();
    for n in root.children().filter(|c| c.is_element()) {
        let g = match local(&n) {
            "wpt" => pt(&n).map(|p| Geometry::Points(vec![p])),
            "rte" => Some(Geometry::Lines(vec![n
                .children()
                .filter(|c| c.is_element() && local(c) == "rtept")
                .filter_map(|c| pt(&c))
                .collect()])),
            "trk" => Some(Geometry::Lines(
                n.children()
                    .filter(|c| c.is_element() && local(c) == "trkseg")
                    .map(|s| {
                        s.children().filter(|c| c.is_element() && local(c) == "trkpt").filter_map(|c| pt(&c)).collect()
                    })
                    .collect(),
            )),
            _ => None,
        };
        if let Some(g) = g {
            out.push(Feature { id: None, properties: named(&n, local(&n)), geometry: g });
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn geojson_features_and_winding() {
        // An RFC 7946 (counter-clockwise) square is turned round to d3's clockwise.
        let t = r#"{"type":"FeatureCollection","features":[
            {"type":"Feature","id":7,"properties":{"name":"sq","pop":"12"},
             "geometry":{"type":"Polygon","coordinates":[[[0,0],[10,0],[10,10],[0,10],[0,0]]]}},
            {"type":"Feature","properties":null,"geometry":{"type":"Point","coordinates":[1,2,3]}}]}"#;
        let f = parse(t, Format::GeoJson, None).unwrap();
        assert_eq!(f.len(), 2);
        assert_eq!(f[0].id.as_deref(), Some("7"));
        assert_eq!(f[0].number("pop"), Some(12.0));
        let Geometry::Polygons(p) = &f[0].geometry else { panic!() };
        assert_eq!(p[0][0][1], [0.0, 10.0], "reversed");
        assert_eq!(f[1].geometry, Geometry::Points(vec![[1.0, 2.0]]));
    }

    #[test]
    fn topojson_arcs_decode() {
        let t = r#"{"type":"Topology","transform":{"scale":[1,1],"translate":[0,0]},
            "objects":{"a":{"type":"GeometryCollection","geometries":[
              {"type":"LineString","arcs":[0,1],"properties":{"n":1}},
              {"type":"LineString","arcs":[-2]}]}},
            "arcs":[[[0,0],[1,0]],[[1,0],[0,1]]]}"#;
        let f = parse(t, Format::TopoJson, Some("a")).unwrap();
        assert_eq!(f[0].geometry, Geometry::Lines(vec![vec![[0.0, 0.0], [1.0, 0.0], [1.0, 1.0]]]));
        assert_eq!(f[1].geometry, Geometry::Lines(vec![vec![[1.0, 1.0], [1.0, 0.0]]]));
    }

    #[test]
    fn kml_and_gpx() {
        let k = r#"<kml xmlns="http://www.opengis.net/kml/2.2"><Document><Placemark id="p"><name>Home</name>
            <ExtendedData><Data name="rank"><value>3</value></Data></ExtendedData>
            <Point><coordinates>2.35,48.85,0</coordinates></Point></Placemark></Document></kml>"#;
        let f = parse(k, Format::Kml, None).unwrap();
        assert_eq!(f[0].text("name").as_deref(), Some("Home"));
        assert_eq!(f[0].number("rank"), Some(3.0));
        assert_eq!(f[0].geometry, Geometry::Points(vec![[2.35, 48.85]]));
        let g = r#"<gpx version="1.1"><wpt lat="1" lon="2"><name>W</name></wpt>
            <trk><name>T</name><trkseg><trkpt lat="0" lon="0"/><trkpt lat="1" lon="1"/></trkseg></trk></gpx>"#;
        let f = parse(g, Format::Gpx, None).unwrap();
        assert_eq!(f.len(), 2);
        assert_eq!(f[1].geometry, Geometry::Lines(vec![vec![[0.0, 0.0], [1.0, 1.0]]]));
    }
}
