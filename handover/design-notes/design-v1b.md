Written for: Urano (decides), with Mercurio (meshing and render) and Netuno (rigid bodies of cells) as readers of the interfaces.

# Design note V.1b: from a `voxelAsset` to cells in the scene

Facts. The schema, the readers (`.vox`, SRVOL), the voxelizer and `Occupancy` are on `feat/voxel-asset`. Nothing in `sr-eval` resolves a `voxelAsset` yet. `mesh_path` (sim3d.rs) is how a mesh asset key becomes a file (document index, base directory, `assets::resolve`, remote refused). `sr_eval::voxels::body` already turns an `Occupancy` into `Shape3::Voxels` with mass `cells x density x cell volume`. The renderer builds meshes by primitive in `render_three.rs` (`"mesh" => return None` is where assets are drawn separately).

## 1. The loader (sr-eval, `voxel_asset.rs`)
`load(program, key, limits) -> Result<VoxelModel, String>` with `VoxelModel { occupancy: Arc<Occupancy>, colours: Colours, materials: BTreeMap<u8, props>, cell_size: Option<f64> }`.
- The key is resolved like `mesh_path` (local file only, remote is an error that says so); `sha256` if given is checked on the bytes read BEFORE parsing; format from `format` or the extension; `model` selects one model of a `.vox`; `voxelGrid` names the grid of an SRVOL.
- `fromMesh` goes through `mesh_asset_triangles` then `voxel::from_triangles` with the asset's `cellSize`.
- Limits from `maxCells` (default 4,194,304) and `maxMemoryMiB` (default 128) become `Limits{max_cells, max_bytes}` before anything is built; the file size is bounded by the same memory number.
- A cache by (program identity, key) so that an object used many times and a frame loop read the file once; the entry keeps the `Occupancy` fingerprint.
- Cell size of an object: `object3D@cellSize`, else the asset's, else the SRVOL's, else 1 (the order the XSD says).
- Materials: `palette="file"` needs `colours == File` (else an error that says the file has no colours; the default palette is the format's, so `Colours::Default` is allowed only when the object names materials or has a `material`). An index used by a cell with no material from palette, object or file is an error that names the index.

## 2. Physics (Netuno's side, wired by me only as far as the loader)
`rigidBody@shape="voxels"` on a `primitive="voxels"` object uses `voxels::body(&occupancy, [s;3], density, pixels_per_meter)`. The object's origin is the corner of the bounding box of the occupied cells, so the loader returns the `Occupancy` shifted by its minimum key (the shift is part of the model, not of the file) and the body's frame is that corner; the centre of mass comes from the exact moments, as `Shape3::Voxels` already does. `rigidBody@shape` is Netuno's attribute (CRT13); I do not touch it, I only provide `VoxelModel`.

## 3. Render (Mercurio's meshing, V.3)
Two ways to draw the cells: (a) a mesh with every exposed face as a quad ("blocks"): O(surface faces) vertices, rebuilt only for the bricks whose revision changed; (b) one instance of a cube per cell: O(cells) instances, no face culling, overdraw. Cost for a solid cube of side n: (a) 6 n^2 quads, (b) n^3 instances; at n = 64 that is 24,576 quads against 262,144 instances, so (a). Faces are grouped by material (palette index to material), one draw per material. My part: the loader hands `VoxelModel` and a function `faces(&Occupancy) -> Vec<Face{cell, axis, sign, palette}>` in scan order (pure, tested) in sr-3d; Mercurio turns it into vertices and decides greedy merging (merging is allowed only inside one palette index and one plane; I do not do it).

## 4. Oracles
1. A 2 x 2 x 2 block of cells of side s and density rho has mass 8 rho s^3 and the inertia of a cube of side 2s, `m (2s)^2 / 6` about each axis: through the whole path (`.vox` file written by `make_vox.py` -> loader -> `body` -> world), against the closed form, to 1e-12 relative.
2. Faces: a block of a x b x c cells has exactly 2(ab + bc + ca) exposed faces; two blocks touching lose exactly 2 x the contact area; every face's cell is occupied and its neighbour across it is not.
3. Coverage: an asset drawn by the rasterizer and by the path tracer under the same camera covers the same pixels (the silhouette masks agree except for pixels on the edge, at most 1 % of the covered area); runs on the GPU queue (`sr-gpu`), with the adapter named.
4. Loader: the same document read twice has the same fingerprint; a wrong `sha256` is an error before parsing; an over-limit file is an error that names the number; a remote `src` is an error that says remote.

## 5. Order and files
1) `voxel_asset.rs` + oracle 4 + oracle 1 up to the Occupancy and the mass (no GPU); 2) `sr_3d::voxel::faces` + oracle 2; 3) with Mercurio: the mesh and oracle 3. Each its own commit on `feat/voxel-assets-eval` over main after V.1 merges.
