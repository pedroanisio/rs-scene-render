p='srep-0000-cinematic-impact.md'
s=open(p).read()

section = r'''### Voxel assets and objects (`voxelAsset`, `primitive="voxels"`)

A model of cells, each with a palette index 1 to 255, is declared once as an asset and used by any number of objects. It is the
body of cells of `sr_3d::occupancy` (sparse bricks of side eight, exact moments, connected components) that the rigid world already
reads, so what an object looks like and what it collides as are the same cells.

```xml
<assets>
  <voxelAsset id="castle" src="castle.vox" sha256="DIGEST" license="MIT" maxCells="2000000"/>
  <mesh id="rock" src="rock.glb"/>
  <voxelAsset id="rock-cells" fromMesh="rock" cellSize="0.25"/>
</assets>
<composition>
  <object3D id="keep" primitive="voxels" voxels="castle" cellSize="2" palette="file" surface="blocks"/>
</composition>
```

`voxelAsset` is version 1.3 (VOX1). It has exactly one source (VOX2): a file (`src`, with `format` `vox` or `srvol`, by extension if
absent; `model` is the number from 0 of one model of a `vox` file; `voxelGrid` names the grid of an `srvol`) or a closed mesh asset
(`fromMesh`, a mesh asset by VOX3, with `cellSize`). The attributes of the other source are an error, not ignored. `maxCells`
(at most 67,108,864) and `maxMemoryMiB` (at most 4,096) are types of the schema; the engine's defaults are 4,194,304 cells,
128 MiB and the grid `voxels`, and over either is an error that names the number, never a model cut short. The provenance
attributes are those of the other assets.

An `object3D` of primitive `voxels` names its asset (VOX4). `cellSize`, `palette` and `surface` belong to that primitive (VOX5).
`palette` is the word `file` or at most 255 material IDs, in the order of the palette indices 1, 2, ... (VOX6); the material of a
cell is the one of its index, else the object's `material`, else the colour of the file, and an index with none of the three is an
error that names it. `surface="blocks"` (the only value) draws every exposed face of a cell as a quad. The object has no `mesh`,
`volume`, `terrain`, `map`, `text` or `path`, and no `medium` or `pyro` child (VOX7). Position, scale and rotation are those of
every `object3D`; the origin of the cells is the corner of the bounding box of the occupied cells.

**Axes.** The lattice is the scene's: the cell `[i, j, k]` is `[i, i+1) x [j, j+1) x [k, k+1)` of the object's space in cells, x
right, y down, z away from the camera. MagicaVoxel is x right, y forward, z up, also right-handed, and its cell `[x, y, z]` becomes
`[x, -z-1, y]`: a half turn about x composed with a swap of y and z, determinant 1, so no face is mirrored and every cell stays
exactly on the lattice.

**A mesh cut into cells (`fromMesh`).** The lattice is aligned to multiples of `cellSize` in the mesh's own coordinates, and a
cell is filled, with the palette index 1, when its centre is inside the mesh by the test that the colliders of the smoke use
(`sr_sim::pyro::mesh::Mesh`: a closed, validated surface, the nearest oriented surface decides), so a mesh that is not closed is the
collider's error. The box of the lattice and the limits are checked before any cell is looked at, and the rows are cut in parallel
and put together in the order of the scan, so the result is the same on any number of threads.

**SRVOL as a voxel cache.** One grid, `voxels` by default, whose value at the index `[i, j, k]` is the palette index of the cell
(exact in a float, 0 for none) and whose transform is a uniform scale, the cell size. The bytes are canonical: the same cells give the
same file, and a model written and read comes back with the same fingerprint. A cache has no palette; a value that is not an integer
from 0 to 255 is an error that names the cell and the value. The reader's bounds are the ones of the SRVOL section above.

**What the `.vox` reader takes from the format, and from where.** The reader accepts the header `VOX ` with version 150 or 200,
and the chunks `MAIN`, `PACK`, `SIZE`, `XYZI`, `RGBA`, `MATL`, `nTRN`, `nGRP` and `nSHP`; every other chunk is skipped by its length.
Layers, hidden nodes, animation after the first frame and cameras are not read. Four facts of the format are not obvious and each
has its source:

1. *The palette is offset by one.* The colour of the cell index `c` is the entry `c - 1` of the `RGBA` chunk: "color [0-254] are
   mapped to palette index [1-255]" (`MagicaVoxel-file-format-vox.txt` of `ephtracy/voxel-model`, section 7). Proved by the knight
   of the same repository: its cells' colours are the file's entries one place down.
2. *A file with no `RGBA` chunk has the default palette* of the same description (section 8), 256 entries embedded in
   `sr_3d::voxel::default_palette` by `tools/make_default_palette.py` (the description is MIT licensed; the table is that of the
   description and nothing else). Proved by the cat and the soldier of the same repository, which have no `RGBA` chunk.
3. *A `MATL` id is the palette index.* The description does not say it; `ogt_vox.h` of opengametools (MIT) keeps
   `materials.matl[color_index]` beside `palette.color[color_index]`, and the real files agree: in `metal-material` the cells have
   the colour index 85 and the one material that is not the default is the `MATL` with id 85, and in
   `single-voxel-with-material` the cell is 249 and the odd material is the `MATL` 249.
4. *Placement under a scene graph.* A model's cells are placed about its centre, `floor(size / 2)` ("the centre pivot for that model
   is located at floor(size.xyz / 2)", `ogt_vox.h`, line 125), by `p = R q + t`, where `R` comes from the rotation byte of the `nTRN`:
   bits 0 and 1 are the column of the nonzero entry of the first row, bits 2 and 3 those of the second (the third is the column that
   is left), bits 4, 5 and 6 the signs of the three rows (section (c) of the extension file of the same repository, whose example
   `R = [[0, 1, 0], [0, 0, -1], [-1, 0, 0]]` is the byte 105 and is the fixture `spec-rotation`). Where the cells of two models fall
   on one place the later one in the graph wins.

*What is evidence and what is proof.* Fact 4 is proved by the description and by the fixtures that `tools/make_vox.py` writes (an
independent script that shares no code with the reader: 24 rotations, nesting, several models), not by a real file: none of the real
files has a rotation. The layout of `axes.vox` of the `dot_vox` crate, with its cube on the plane z = 0 and about x = y = 0, fits
the centre `floor(size / 2)` and is the evidence of it, not a proof. The real files are in `crates/sr-3d/tests/fixtures/vox/real`, with
the licence texts and the sources (`SOURCES.md`); the test that reads all thirteen sample files is run with `VOX_SAMPLES` set and is
skipped, not failed, when the files are not there.

**Bounds before allocation.** The reader checks, before it builds anything: the size of the file (default 1 GiB), the models
(65,536), the nodes of the scene graph (1,048,576), its depth (64) with a cycle check, and the count of cells that a graph places
(one model as many times as it is used, capped before it is built), then the limits of the grid. A `.vox` model larger than 256 on
a side is an error of the format's own limit, and every number is named in the message.

| Rule | Says |
|---|---|
| VOX1 | `voxelAsset` and primitive `voxels` need `version="1.3"` |
| VOX2 | exactly one of `src` and `fromMesh`; `format`, `model`, `voxelGrid` only with a file of that format; `fromMesh` and `cellSize` together |
| VOX3 | `fromMesh` names a mesh asset |
| VOX4 | an object of primitive `voxels` names a `voxelAsset` in `voxels` |
| VOX5 | `voxels`, `cellSize`, `palette`, `surface` belong to primitive `voxels` |
| VOX6 | `palette` is `file` or at most 255 material IDs |
| VOX7 | a voxels object has no mesh, volume, terrain, map, text or path, and no medium or pyro child |

The Schematron and `sr-model`'s `rules.rs` agree on all 373 documents of the corpus (and the independent `lxml` oracle of
`tools/build_corpus.py` with them): a valid document of each source and an invalid one for each rule.

'''
anchor = '## SRVOL cache version 1\n'
assert s.count(anchor)==1
s = s.replace(anchor, section + anchor)

inv = '''### `voxelAssetType`

| Attribute | XSD type or inline restriction | Presence/default |
|---|---|---|
| `id` | xs:ID | Required |
| `src` | xs:anyURI | Optional; exactly one of `src` and `fromMesh` (VOX2) |
| `format` | xs:string; enumeration=vox, enumeration=srvol | Optional; absent: from the extension; only with `src` |
| `model` | xs:nonNegativeInteger | Optional; absent: the whole scene of a `vox` file; only with format `vox` |
| `fromMesh` | xs:IDREF | Optional; a mesh asset (VOX3) |
| `cellSize` | positiveDecimal | Optional; required with `fromMesh`, refused with `src` (VOX2) |
| `voxelGrid` | volumeChannelType | Optional, no XSD default; only with format `srvol`; the engine uses `voxels` |
| `maxCells` | xs:positiveInteger; maxInclusive=67108864 | Optional, no XSD default; the engine uses `4194304` |
| `maxMemoryMiB` | xs:positiveInteger; maxInclusive=4096 | Optional, no XSD default; the engine uses `128` |
| `sha256`, `license`, `credit`, `proxy` | assetProvenance | As in `assetProvenance` |

'''
a2='### `mediumType`\n'
assert s.count(a2)==1
s=s.replace(a2, inv+a2)

# object3D rows
old='enumeration=globe, enumeration=volume | Required |'
assert s.count(old)==1
s=s.replace(old,'enumeration=globe, enumeration=volume, enumeration=voxels | Required |')
old='| `volume` | xs:IDREF | Optional; absent |\n| `terrain`'
assert s.count(old)==1
s=s.replace(old,'| `volume` | xs:IDREF | Optional; absent |\n| `voxels` | xs:IDREF | Optional; a `voxelAsset`, required with primitive `voxels` (VOX4, VOX5) |\n| `cellSize` | positiveDecimal | Optional, no XSD default; only with primitive `voxels` (VOX5); absent: the asset\'s, else the cache\'s, else `1` |\n| `palette` | xs:string | Optional; `file` or at most 255 material IDs (VOX5, VOX6); absent: `file` if the file has colours, else the object\'s `material` |\n| `surface` | xs:string; enumeration=blocks | Optional, no XSD default; only with primitive `voxels` (VOX5); the engine uses `blocks` |\n| `terrain`')
open(p,'w').write(s)
