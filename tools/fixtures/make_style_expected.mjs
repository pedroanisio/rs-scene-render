// Writes MapLibre's own evaluation of a basemap style for the sr-geo style tests.
//
//   mkdir /tmp/ms && cd /tmp/ms && npm install @maplibre/maplibre-gl-style-spec@23 @protomaps/basemaps@5 @mapbox/vector-tile pbf pmtiles
//   STYLE_DIR=/tmp/ms node tools/fixtures/make_style_expected.mjs
//
// It writes the built-in styles crates/sr-geo/styles/protomaps-{light,dark}.json
// (the Protomaps basemap layers, BSD-3-Clause, see the NOTICE there) and
// crates/sr-geo/tests/fixtures/style.json: for a sample of the
// features of baixa.pmtiles at zooms 12 to 17, which layers' filters accept each
// feature and the values of their paint and layout properties.

import fs from "node:fs";
import path from "node:path";
import zlib from "node:zlib";
import {fileURLToPath} from "node:url";

const dir = process.env.STYLE_DIR;
if (!dir) throw new Error("set STYLE_DIR");
const mod = (p) => import(path.join(dir, "node_modules", p));
const spec = await mod("@maplibre/maplibre-gl-style-spec/dist/index.mjs");
const {layers, namedFlavor} = await mod("@protomaps/basemaps/dist/esm/index.js");
const {VectorTile} = await mod("@mapbox/vector-tile/index.js");
const Pbf = (await mod("pbf/index.js")).default;
const {PMTiles, FileSource} = await mod("pmtiles/dist/esm/index.js");

const here = path.dirname(fileURLToPath(import.meta.url));
const out = path.join(here, "../../crates/sr-geo/tests/fixtures");
const styles = path.join(here, "../../crates/sr-geo/styles");
fs.mkdirSync(styles, {recursive: true});
for (const flavor of ["light", "dark"]) {
  fs.writeFileSync(path.join(styles, `protomaps-${flavor}.json`),
    JSON.stringify({version: 8, name: `protomaps-${flavor}`, layers: layers("protomaps", namedFlavor(flavor), {lang: "en"})}));
}
fs.writeFileSync(path.join(styles, "NOTICE"),
  "protomaps-light.json and protomaps-dark.json: the Protomaps basemap style layers (@protomaps/basemaps), " +
  "BSD-3-Clause, Copyright (c) Protomaps LLC. They style the Protomaps basemap tile schema.\n");
const style = JSON.parse(fs.readFileSync(path.join(styles, "protomaps-light.json"), "utf8"));

// features: a spread from every layer of every tile
class NodeSource { constructor(p) { this.p = p; }
  getKey() { return this.p; }
  async getBytes(offset, length) { const fd = fs.openSync(this.p, "r"); const b = Buffer.alloc(length); fs.readSync(fd, b, 0, length, offset); fs.closeSync(fd);
    return {data: b.buffer.slice(b.byteOffset, b.byteOffset + length)}; } }
const pm = new PMTiles(new NodeSource(path.join(out, "baixa.pmtiles")));
const header = await pm.getHeader();
const samples = [];
for (let z = header.minZoom; z <= header.maxZoom; z++) {
  // the tiles around the extract's centre
  const n = 2 ** z, lon = -9.1375, lat = 38.711;
  const x = Math.floor((lon + 180) / 360 * n);
  const y = Math.floor((1 - Math.log(Math.tan(lat * Math.PI / 180) + 1 / Math.cos(lat * Math.PI / 180)) / Math.PI) / 2 * n);
  const t = await pm.getZxy(z, x, y);
  if (!t) continue;
  let data = Buffer.from(t.data);
  if (data[0] === 0x1f) data = zlib.gunzipSync(data);
  const vt = new VectorTile(new Pbf(data));
  for (const name of Object.keys(vt.layers)) {
    const l = vt.layers[name];
    const step = Math.max(1, Math.floor(l.length / 25));
    for (let i = 0; i < l.length; i += step) {
      const f = l.feature(i);
      samples.push({tile: `${z}/${x}/${y}`, layer: name, index: i, id: f.id ?? null,
        type: ["Unknown", "Point", "LineString", "Polygon"][f.type], properties: f.properties});
    }
  }
}

// straight RGBA in 0..1 (the style-spec stores premultiplied colours)
const unpremultiply = (c) => c.a > 0 ? [c.r / c.a, c.g / c.a, c.b / c.a, c.a] : [0, 0, 0, 0];
const props = (l, kind) => Object.keys((kind === "paint" ? l.paint : l.layout) || {});
const results = [];
for (const s of samples) {
  const feature = {type: {Point: 1, LineString: 2, Polygon: 3}[s.type], properties: s.properties, id: s.id};
  for (const zoom of [12, 13.5, 15, 17]) {
    const hits = [];
    for (const l of style.layers) {
      if (l.type === "background" || l["source-layer"] !== s.layer) continue;
      const ok = l.filter === undefined ? true : spec.featureFilter(l.filter).filter({zoom}, feature);
      if (!ok) continue;
      const values = {};
      for (const kind of ["paint", "layout"]) {
        for (const p of props(l, kind)) {
          if (p.startsWith("icon-") || p === "text-font" || p === "symbol-sort-key" && !l.layout[p]) continue;
          const ref = spec.latest[`${kind}_${l.type}`][p];
          const ex = spec.expression.createPropertyExpression(l[kind][p], ref);
          if (ex.result !== "success") { values[p] = {error: ex.value.map(e => e.message).join("; ")}; continue; }
          let v = ex.value.evaluate({zoom}, feature);
          if (v && typeof v === "object" && "r" in v && "a" in v) v = {color: unpremultiply(v)};
          else if (v && typeof v === "object" && v.sections) v = {text: v.toString()};
          else if (v && typeof v === "object" && "values" in v) v = v.values;
          values[p] = v;
        }
      }
      hits.push({layer: l.id, values});
    }
    results.push({sample: samples.indexOf(s), zoom, hits});
  }
}
const bg = style.layers.find(l => l.type === "background");
const background = [0, 10, 20].map(zoom => {
  const ex = spec.expression.createPropertyExpression(bg.paint["background-color"], spec.latest.paint_background["background-color"]);
  return {zoom, color: unpremultiply(ex.value.evaluate({zoom}))};
});
fs.writeFileSync(path.join(out, "style.json"), JSON.stringify({samples, results, background}));
console.log("samples", samples.length, "evaluations", results.length);
