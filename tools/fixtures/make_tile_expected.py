"""Writes the map-tile fixtures and their reference values.

    python -m pip install pmtiles mapbox-vector-tile pillow
    pmtiles extract https://build.protomaps.com/<date>.pmtiles baixa.pmtiles \\
        --bbox=-9.142,38.708,-9.133,38.714 --minzoom=12 --maxzoom=14
    python tools/fixtures/make_tile_expected.py baixa.pmtiles

* `baixa.pmtiles`: a Protomaps basemap extract of Lisbon's Baixa (z12-14),
  © OpenStreetMap contributors (ODbL), see NOTICE.
* `raster.pmtiles`: z0-2 PNG tiles written by the reference `pmtiles` writer,
  each a solid colour made from its z/x/y (red = 60·z, green = 40·x, blue = 40·y).
* `hill.pmtiles`: z10 Terrarium elevation tiles round 0°, 0° holding a 2000-metre
  Gaussian hill (σ 0.05°) for the 3D map tests.
* `tiles.json`: what the reference readers make of them: header fields, tile
  ids, and per tile and layer the feature count, vertex count and coordinate
  sums, with the first features in full.
"""

import gzip
import io
import json
import shutil
import sys
from pathlib import Path

import mapbox_vector_tile
from PIL import Image
from pmtiles.reader import MmapSource, Reader, all_tiles
from pmtiles.tile import Compression, TileType, zxy_to_tileid
from pmtiles.writer import Writer

OUT = Path(__file__).resolve().parents[2] / "crates/sr-geo/tests/fixtures"


def main(src):
    OUT.mkdir(parents=True, exist_ok=True)
    shutil.copyfile(src, OUT / "baixa.pmtiles")
    (OUT / "NOTICE").write_text(
        "baixa.pmtiles: Protomaps basemap (https://protomaps.com) extract, data "
        "© OpenStreetMap contributors, available under the Open Database License "
        "(https://www.openstreetmap.org/copyright).\n"
        "countries-110m.json: world-atlas (ISC licence), from Natural Earth (public domain).\n")
    expected = {"tile_ids": [[z, x, y, zxy_to_tileid(z, x, y)] for z in range(0, 12)
                             for (x, y) in [(0, 0), ((1 << z) - 1, 0), (0, (1 << z) - 1), ((5 * z) % (1 << z), (7 * z) % (1 << z))]]}

    with open(OUT / "baixa.pmtiles", "rb") as f:
        r = Reader(MmapSource(f))
        h = r.header()
        expected["header"] = {k: (v.value if hasattr(v, "value") else v) for k, v in h.items()}
        tiles = {}
        for (z, x, y), data in all_tiles(r.get_bytes):
            if data[:2] == b"\x1f\x8b":
                data = gzip.decompress(data)
            decoded = mapbox_vector_tile.decode(data, default_options={"y_coord_down": True, "transformer": None})
            layers = {}
            for name, layer in decoded.items():
                feats = layer["features"]
                verts, sx, sy = 0, 0.0, 0.0

                def walk(c):
                    nonlocal verts, sx, sy
                    if isinstance(c[0], (int, float)):
                        verts += 1
                        sx += c[0]
                        sy += c[1]
                    else:
                        for d in c:
                            walk(d)
                for ft in feats:
                    walk(ft["geometry"]["coordinates"])
                layers[name] = {"extent": layer["extent"], "features": len(feats), "vertices": verts, "sx": sx, "sy": sy,
                                "first": [{"id": ft.get("id"), "type": ft["geometry"]["type"], "properties": ft["properties"]}
                                          for ft in feats[:3]]}
            tiles[f"{z}/{x}/{y}"] = layers
        expected["tiles"] = tiles

    with open(OUT / "raster.pmtiles", "wb") as f:
        w = Writer(f)
        for z in range(3):
            for x in range(1 << z):
                for y in range(1 << z):
                    b = io.BytesIO()
                    Image.new("RGB", (256, 256), (60 * z, 40 * x, 40 * y)).save(b, "PNG")
                    w.write_tile(zxy_to_tileid(z, x, y), b.getvalue())
        w.finalize({"tile_type": TileType.PNG, "tile_compression": Compression.NONE, "min_zoom": 0, "max_zoom": 2,
                    "min_lon_e7": -1800000000, "min_lat_e7": -850511287, "max_lon_e7": 1800000000, "max_lat_e7": 850511287,
                    "center_zoom": 0, "center_lon_e7": 0, "center_lat_e7": 0}, {"name": "solid"})

    import math
    with open(OUT / "hill.pmtiles", "wb") as f:
        w = Writer(f)
        for x in range(510, 514):
            for y in range(510, 514):
                img = Image.new("RGB", (256, 256))
                px = img.load()
                for j in range(256):
                    for i in range(256):
                        n = 1 << 10
                        lon = (x + (i + 0.5) / 256) / n * 360 - 180
                        lat = math.degrees(math.atan(math.sinh(math.pi * (1 - 2 * (y + (j + 0.5) / 256) / n))))
                        v = 2000 * math.exp(-(lon * lon + lat * lat) / 0.05 ** 2) + 32768
                        r = math.floor(v / 256)
                        g = math.floor(v - r * 256)
                        px[i, j] = (r, g, math.floor((v - r * 256 - g) * 256))
                b = io.BytesIO()
                img.save(b, "PNG")
                w.write_tile(zxy_to_tileid(10, x, y), b.getvalue())
        w.finalize({"tile_type": TileType.PNG, "tile_compression": Compression.NONE, "min_zoom": 10, "max_zoom": 10,
                    "min_lon_e7": -3515625, "min_lat_e7": -3515065, "max_lon_e7": 3515625, "max_lat_e7": 3515065,
                    "center_zoom": 10, "center_lon_e7": 0, "center_lat_e7": 0}, {"name": "hill", "encoding": "terrarium"})

    (OUT / "tiles.json").write_text(json.dumps(expected, indent=0, sort_keys=True))
    print("wrote", len(expected["tiles"]), "vector tiles")


if __name__ == "__main__":
    main(sys.argv[1])
