// Writes the d3-geo reference values for the sr-geo tests.
//
//   mkdir /tmp/d3 && cd /tmp/d3 && npm install d3-geo@3 d3-interpolate@3 topojson-client@3 world-atlas@2
//   D3_DIR=/tmp/d3 node tools/fixtures/make_geo_expected.mjs
//
// It copies world-atlas's countries-110m.json (Natural Earth, public domain;
// world-atlas is ISC) next to the expectations: every country's projected area,
// bounds and ring count under each projection setup, full coordinates for the
// countries that cross the antimeridian or wrap a pole, fits, and flyTo views.

import fs from "node:fs";
import path from "node:path";
import {fileURLToPath} from "node:url";

const dir = process.env.D3_DIR;
if (!dir) throw new Error("set D3_DIR to a folder with d3-geo, d3-interpolate, topojson-client and world-atlas installed");
const mod = (p) => import(path.join(dir, "node_modules", p));
const d3 = await mod("d3-geo/src/index.js");
const {interpolateZoom} = await mod("d3-interpolate/src/index.js");
const topojson = await mod("topojson-client/src/index.js");

const here = path.dirname(fileURLToPath(import.meta.url));
const out = path.join(here, "../../crates/sr-geo/tests/fixtures");
fs.mkdirSync(out, {recursive: true});
const atlasPath = path.join(dir, "node_modules/world-atlas/countries-110m.json");
fs.copyFileSync(atlasPath, path.join(out, "countries-110m.json"));
const world = JSON.parse(fs.readFileSync(atlasPath, "utf8"));
const countries = topojson.feature(world, world.objects.countries).features;

const make = {
  equirectangular: () => d3.geoEquirectangular(),
  mercator: () => d3.geoMercator(),
  "equal-earth": () => d3.geoEqualEarth(),
  "natural-earth": () => d3.geoNaturalEarth1(),
  albers: () => d3.geoConicEqualArea(),
  "lambert-conformal": () => d3.geoConicConformal(),
  orthographic: () => d3.geoOrthographic(),
  stereographic: () => d3.geoStereographic(),
  "azimuthal-equal-area": () => d3.geoAzimuthalEqualArea(),
  "azimuthal-equidistant": () => d3.geoAzimuthalEquidistant(),
};

const setups = [
  {projection: "equirectangular"},
  {projection: "mercator", scale: 100, rotate: [-20, 0, 0]},
  {projection: "equal-earth", rotate: [150, 0, 0]},
  {projection: "natural-earth", rotate: [-10, 0, 0], center: [0, 20], angle: 15},
  {projection: "albers", parallels: [29.5, 45.5], rotate: [96, 0, 0], center: [-0.6, 38.7], scale: 1070, extent: [[0, 0], [960, 500]]},
  {projection: "lambert-conformal", parallels: [35, 65], rotate: [-10, 0, 0], center: [0, 52], scale: 600, extent: [[0, 0], [960, 500]]},
  {projection: "orthographic", rotate: [-10, -40, 0]},
  {projection: "orthographic", rotate: [100, 30, 20]},
  {projection: "stereographic", rotate: [0, -90, 0]},
  {projection: "azimuthal-equal-area", rotate: [-100, -40, 0]},
  {projection: "azimuthal-equidistant", rotate: [0, 90, 0]},
  {projection: "mercator", rotate: [150, 0, 0], scale: 300, extent: [[100, 50], [800, 450]]},
  {projection: "equal-earth", rotate: [0, 0, 0], precision: 0},
];

function build(s) {
  const p = make[s.projection]();
  if (s.parallels) p.parallels(s.parallels);
  if (s.scale) p.scale(s.scale);
  if (s.translate) p.translate(s.translate);
  if (s.rotate) p.rotate(s.rotate);
  if (s.center) p.center(s.center);
  if (s.angle) p.angle(s.angle);
  if (s.precision !== undefined) p.precision(s.precision);
  if (s.extent) p.clipExtent(s.extent);
  return p;
}

// Rings of a path string (full precision): polygons are not separated by d3's path output,
// so rings are compared as a flat list.
function rings(d) {
  if (!d) return [];
  return d.split("M").filter(Boolean).map((r) =>
    r.replace(/Z$/, "").split("L").map((pt) => pt.split(",").map((v) => Math.round(Number(v) * 1e7) / 1e7)));
}

const detail = new Set(["Russia", "Fiji", "Antarctica"]);
const cases = setups.map((s) => {
  const p = build(s);
  const path = d3.geoPath(p).digits(null);
  const sphere = rings(path({type: "Sphere"}));
  return {
    setup: s,
    scale: p.scale(),
    translate: p.translate(),
    countries: countries.map((f) => ({
      name: f.properties.name,
      area: path.area(f),
      bounds: path.bounds(f),
      rings: rings(path(f)).length,
      coords: detail.has(f.properties.name) ? rings(path(f)) : undefined,
    })),
    sphere: {area: path.area({type: "Sphere"}), rings: sphere.length},
    points: [[2.35, 48.86], [-74.0, 40.7], [139.7, 35.7], [151.2, -33.9], [-0.1, 51.5]].map((ll) => {
      const q = p(ll);
      // d3 projects every point; whether it is visible is the clip's business.
      const visible = path({type: "Point", coordinates: ll}) !== null;
      return {ll, xy: q, visible};
    }),
  };
});

const brazil = countries.find((f) => f.properties.name === "Brazil");
const fits = [
  {projection: "equal-earth", extent: [[10, 10], [950, 490]], object: "sphere"},
  {projection: "orthographic", extent: [[0, 0], [500, 500]], object: "sphere"},
  {projection: "mercator", extent: [[20, 20], [620, 460]], object: "Brazil"},
].map((f) => {
  const p = make[f.projection]();
  p.fitExtent(f.extent, f.object === "sphere" ? {type: "Sphere"} : brazil);
  return {...f, scale: p.scale(), translate: p.translate()};
});

// TopoJSON decoding: a few countries' first ring, degrees.
const decoded = countries
  .filter((f) => detail.has(f.properties.name))
  .map((f) => ({name: f.properties.name, id: f.id, type: f.geometry.type, first: f.geometry.type === "Polygon"
    ? f.geometry.coordinates[0] : f.geometry.coordinates[0][0]}));

// van Wijk and Nuij: d3-interpolate's interpolateZoom on [ux, uy, w].
const zooms = [
  {rho: Math.SQRT2, a: [0, 0, 400], b: [300, 200, 50]},
  {rho: 1.0, a: [-10, 50, 1000], b: [120, 30, 1000]},
  {rho: 2.0, a: [5, 5, 10], b: [5, 5, 300]},
].map((z) => {
  const i = interpolateZoom.rho(z.rho)(z.a, z.b);
  return {...z, duration: i.duration, at: [0, 0.1, 0.25, 0.5, 0.75, 0.9, 1].map((t) => [t, i(t)])};
});

fs.writeFileSync(path.join(out, "d3.json"), JSON.stringify({cases, fits, decoded, zooms}));
console.log("wrote", cases.length, "cases,", fits.length, "fits,", zooms.length, "zooms");
