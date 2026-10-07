---
disclaimer:
  notice: >-
    No information within this document should be taken for granted.
    Any statement or premise not backed by a real logical definition
    or verifiable reference may be invalid, erroneous, or a hallucination.
  generated_by: "Codex via OpenAI API"
  date: "2026-10-03"
---

```
SREP:            0 (number to be assigned upstream)
Title:           Native volumetric and large-scale cinematic effects
Author:          scene-render maintainers, implementation assisted by Codex
Status:          Draft — implementation in progress
Type:            Standards
Created:         2026-10-03
Schema-Version:  1.3 (proposed)
```

# Native volumetric and large-scale cinematic effects

## Abstract

Add volume assets, volumetric materials, three-dimensional pyro and particle
simulation, ocean surfaces and impulses, terrain deformation, fracture, and
animated mesh caches to scene-render. Require deterministic seeking, bounded
resource use, explicit units and errors, and native UHD delivery without changing
the requested rendering method. This proposal covers reusable cinematic tools;
it does not introduce a Chicxulub-specific scene node or claim predictive
geophysical accuracy.

The implementation acceptance ledger is
[`tools/evidence/cinematic-impact.json`](tools/evidence/cinematic-impact.json).
**This draft is not a claim that all proposed features already execute.**
The XSD/Schematron patch and final conformance references must accompany the
completed implementation before this proposal is ready for upstream acceptance.

## Motivation and existing behavior

At baseline `bc22bc8`, PBR rendering, cameras, materials, rigid bodies, animation,
image-sequence compositing and UHD encoding exist. Four distinct gaps prevent
native close-up impact effects:

1. `sr-sim::fluid` is a two-dimensional incompressible dye simulation. It cannot
   provide three-dimensional density, temperature or an ocean free surface.
2. `sr-sim::particles` stores x/y motion and draws compositor quads. It cannot
   place ejecta in the three-dimensional depth and lighting system.
3. The surface renderer has no heterogeneous participating-medium asset or
   rendering path. Mesh animation has transforms, skinning and morph weights,
   but no general topology-changing cache playback.
4. The path tracer allocates full-frame guides at 32 bytes per pixel. UHD needs
   265,420,800 bytes in one binding and falls back on 128 MiB binding devices.

The implementation must address these independently. Raising output dimensions,
rendering a precomputed movie, or adding an explosion preset does not implement
the missing native capabilities.

Research framing: [Collins et al. (2020)](https://doi.org/10.1038/s41467-020-15269-x)
compare impact simulations with crater observations and favor a 45–60° trajectory
from the northeast. [Range et al. (2022)](https://doi.org/10.1029/2021AV000627)
use a hydrocode for the first ten minutes, then a shallow-water model for global
tsunami propagation. These support an oblique approach and distinct impact and
propagation stages in an authored reconstruction. The inference for this engine
is that crater, ejecta, volume and ocean tools must work together; its cinematic
pyro and shallow-water solvers do not establish predictive shock physics. Native
UHD suitability requires the complete rendered sequence and measured resources
specified below, not only scientific plausibility or individual feature tests.

## Compatibility, identity and time

This is an additive proposal for schema release 1.3.0; XML documents opt in with
`scene/@version="1.3"`, following the existing major.minor document convention.
No existing 1.1/1.2 field, default, meaning or accepted value is removed or changed.
Older validators are expected to reject the new syntax. Extend the structural
version enumeration and add a semantic version gate covering every new element,
primitive and attribute. New enum values require this version gate even when
they occur on an existing element.

IDs remain document-wide `xs:ID`; references are `xs:IDREF` or `xs:IDREFS` with
target-kind rules. A composition owns its nodes and source/impulse children.
Assets are shared references and outlive any individual referring node. Parent,
field and simulation dependencies must remain acyclic. Reusing an asset must
not accidentally share a mutable simulation between independently timed nodes.

New activation and simulation times are finite f64 seconds on the containing
node's local timeline, not wall-clock timestamps. Numbered SRVOL sequences use
that local timeline; frozen SRVSEQ bakes explicitly use composition time. Existing animation children retain
their `timeBase` convention: composition by default, or explicitly local/normalized. Activation windows are half-open
`[start,end)`; absent `end` is unbounded. Parent offsets/scales, instances,
remapping, holds, backward seeking and shutter samples must use the same clock
mapping as existing simulations. Sources are integrated over the overlap of
their active interval and each simulation substep; their activation must not
depend on requested frame order. Caches identify their sample times explicitly.

Positions and lengths use scene units, with +x right, +y down and +z away from
the default viewer. Angles use degrees, velocities scene units/second, and
accelerations scene units/second² unless an existing reused attribute explicitly
defines a physical conversion. Temperature uses kelvin. Volume density is a
nonnegative dimensionless extinction multiplier, not an assertion of mass density.
Authored zero, absence and the empty string are distinct; null XML attributes
are not supported. All real-valued inputs and decoded samples must be finite.

## Required schema surface

These interface tables define the intended contract. The final XSD must encode
the cardinality, enums, defaults and numeric restrictions; Schematron and the
Rust rules must encode the cross-field and target-kind constraints. No validator
may accept these declarations and then silently ignore them at evaluation/render.

### Volume assets and sequences

Add `<volume>` under `<assets>`, with common asset integrity/path attributes:

| Attribute | Type and default | Meaning |
|---|---|---|
| `id` | required ID | Shared asset identity |
| `src` | required URI or printf-style filename pattern | Single file or numbered sequence path |
| `format` | `srvol`, `srvseq`, or `openvdb`; `srvol` | Explicit decoder selection; no extension-based guessing |
| `densityGrid` | channel name; `density` | Required nonnegative density channel |
| `temperatureGrid` | optional channel name | Kelvin channel; absent disables temperature emission |
| `velocityGridX/Y/Z` | optional channel names | All three required exactly when interpolation is `advect`; signed asset-world vectors in scene units/second |
| `first`, `last` | optional signed 32-bit integers | Both present for a sequence; inclusive bounds, first ≤ last |
| `fps` | positive existing fps type; project fps if absent | Cache sampling rate |
| `missingFrame` | `error`, `hold`, `transparent`; `error` | Explicit missing-file policy |
| `interpolation` | `hold`, `linear` or `advect`; `hold` | Temporal field sampling |
| `boundsMinX/Y/Z`, `boundsMaxX/Y/Z` | optional six finite scene coordinates | Explicit medium domain; all six or none, each minimum strictly below its maximum |

Channel names contain 1–64 ASCII letters, digits, underscore, dot or hyphen.
Linear sampling evaluates each frame in world space before interpolation, so
different sparse supports/transforms do not require matching voxel arrays.
Missing density/explicitly selected channels, invalid transforms and nonfinite
samples are errors. Velocity triples must have matching transforms within each
endpoint; density and temperature can have independent transforms and resolutions.
OpenVDB ingestion is implemented for little-endian seekable archives with file versions
222–225 and floating 5/4/3 trees: half, float, double, vec3s and vec3d, including
half-quantized storage, active-mask compression, ZIP and Blosc. Scalar values
are stored as f32; finite doubles are rounded to f32 and values outside its
finite range are rejected. Vector grids expose `<name>.x`, `<name>.y` and
`<name>.z` scalar channels. These channels can be selected explicitly; automatic
velocity-driven volume advection is not implied by importing a vector grid.

`interpolation="advect"` applies independent frozen-velocity midpoint traces to
the enclosing endpoints before linearly blending density and kelvin. For elapsed
seconds h = target time − actual loaded frame time, sample v(p), then
p_mid = p − h*v(p)/2 and p_source = p − h*v(p_mid). The later endpoint has
negative h. Velocity values are asset-world vectors: grid transforms locate
samples without rotating/scaling the vectors; the object's transform subsequently
places the advected field in the scene. This is motion-aware cache interpolation,
not an extra fluid solve or a mass-conserving remap.

`VOL9` requires all three selected channels for `advect` and prohibits them for
`hold`/`linear`, avoiding accepted but ignored channel declarations. Missing
selected channels fail even at exact sample times. Missing transparent endpoints
supply zero density, kelvin and velocity. Missing-frame holds preserve the actual
loaded frame timestamp. If both endpoints resolve to the same frame label, or
time clamps to a sequence endpoint, motion freezes. Existing hold and scalar
linear playback retain their previous semantics. SRVSEQ uses the same traces
on its composition clock; no second application of object retiming occurs.

Inferred density bounds expand by the componentwise maximum endpoint travel,
including background velocities. Authored bounds remain a clipping domain.
Finite elapsed time, finite travel, matched velocity transforms and representable
GPU coordinates are checked before rendering; velocity grids count toward cache
and GPU binding budgets. CPU tests cover transported density/kelvin, spatial
midpoint tracing, reflected/rotated object placement, bounds and invalid inputs.
Scene/GPU verification is tracked separately in the acceptance ledger; this
contract does not assert that the full cinematic acceptance gate has passed.

Affine, unitary, translation, scale and scale/translation transforms are
preserved, including reflections and shear. Background and non-background
inactive voxel/tile values are retained. Active-mask topology is decoded to
recover values but is not exposed as a separate engine field. Background-only
bricks are omitted; a nonzero density background requires explicit finite
medium bounds. Root/internal tiles expand into bounded 8³ engine bricks.
Instances preserve their own transform and share the parent's decoded values
semantically, with separately admitted storage for each output grid. Unsupported
tree/map types, non-seekable archives, invalid offsets, duplicate grids, missing
or cyclic instance parents, corrupt payloads and nonfinite values produce
errors rather than empty smoke.

The decoder independently bounds encoded file size and conservative decoded
storage with `CacheLimits.max_bytes`. Its admission estimate includes output
bricks, retained root/leaf bookkeeping and a 4 MiB node/codec workspace.
`max_bricks` and `max_grids` apply across the entire archive, including vector
components and instances. A 100-million-unit structural work allowance bounds
node visits and tile expansion. Blosc validates the compressed envelope and
expected output size before decompression; both compressed codecs decode into
bounded buffers. This is component admission accounting, not a total RSS cap.
The renderer retains its existing per-file device/storage limit (at most
128 MiB), 256 MiB frame cache and frame-pair budgets. Cache keys include both
path and declared format, so a failed or successful read with one decoder cannot
substitute for another. Input files remain immutable during a render session.

`format="openvdb"` supports both single files and numbered sequences through the
same local-clock, interpolation and missing-frame contracts as SRVOL. No
OpenVDB runtime installation or external conversion is needed; Cargo builds
the Blosc codec dependency. Rust schema generation consumes the XSD's new closed
enum value; the existing version-1.3 gate and VOL rules continue to apply.
The accompanying conformance corpus includes valid static/sequence VDB assets
and rejection of an unknown format. Official OpenVDB 10.0.1-generated version-224
fixtures cover sparse/dense compressed fields, double/vector values, affine
maps, inactive values, tiles and instances. GPU tests compare sparse VDB pixels
with an independently constructed SRVOL field in both renderers, and check
thermal sequence interpolation, retiming, backward replay and format cache
isolation. File-version compatibility beyond the checked-in version-224 corpus
is parser support, not a claim of independent fixtures for every version. Asset discovery and missing-frame checks expand the entire referenced
sequence under the existing bounded expansion policy. `sha256` authenticates a
single cache file; it is rejected on a numbered sequence (VOL7), because one
digest does not specify independent frame integrity. Numbered patterns do not provide per-frame integrity; the SRVSEQ manifest below
provides a pinned sequence with individual frame digests.
Sequences permit at most 1,000,000 inclusive signed 32-bit frame labels (VOL6).
`src` uses the existing `%d`, `%0Nd` or consecutive `#` placeholder, with at most
64 padding digits. Both labels and a valid placeholder are required; `fps`,
non-default `interpolation` and non-default `missingFrame` require a sequence.
The source type is a union of `xs:anyURI` and a filename-pattern string, because
raw `%d` and repeated `#` are not valid URI spellings. Expand the frame placeholder
before URI decoding or resolution. Signed frame labels follow XSD integer lexical
rules, including surrounding XML whitespace and an optional leading plus sign.
Their time is the
object's local time after time remapping, with both endpoints clamped. Hold
interpolation selects the floor frame; linear interpolation selects the enclosing
pair, without loading a second frame at an exact sample time. `missingFrame=hold`
searches backward to the nearest available frame within the declared range;
if none exists, it fails. This choice cannot depend on playback history.
`transparent` supplies zero fields only for an absent file. A corrupt file or
missing selected channel remains an error under every missing-frame policy.
Loaders bound individual decoding and the combined resident frame pair, and
share identical frame references when both samples resolve to the same cache.
Absent explicit bounds, derive the finite medium domain from the density grid's
active voxel bounds plus interpolation support. A nonzero density background
requires explicit bounds; an empty zero-background grid is transparent. Outside
the medium domain density is zero even when the grid's sampling background is
nonzero. This distinguishes an infinite mathematical field from the bounded
region that is rendered, including a uniform medium with no active bricks.

### Frozen native pyro bakes (implemented)

`scene-render bake-volume scene.xml --object plume -o plume-cache` evaluates an
instantiated native pyro object and writes a new cache directory. `--first` and
`--end` select a half-open range of project frame indices; defaults cover the
project. `--param id=value` uses the evaluator's parameter overrides. `--max-mib`
bounds total unique frame files plus the manifest (default 65,536 MiB).
The CLI emits progress on stderr and a receipt/digest on stdout; `--json` emits
one JSON receipt. Existing output paths are rejected. Evaluation or write
failures abort the transaction, and the writer removes its owned partial
output on ordinary error/drop. A process crash can leave an incomplete directory;
it has no published manifest and must not be treated as a completed bake.

The new asset syntax is:

```xml
<volume id="plume-cache" src="plume-cache/manifest.srvseq" format="srvseq"
        sha256="DIGEST_PRINTED_BY_BAKE_COMMAND" densityGrid="density"
        temperatureGrid="temperature" interpolation="hold"/>
```

The digest token in this explanatory fragment must be replaced with the emitted
64 hexadecimal characters. To use the bake, replace the original object's
`pyro` child with `volume="plume-cache"`, retain its transform, medium, conditions,
visibility and timing, and add the asset above. Source fields have already been
sampled through the entire composition clock, including nested time remapping,
`animationSpeed`/`animationOffset`, forces and rigid-body collision proxies.
Playback therefore selects fields using **composition time**, and does not apply
those clocks again. Object transforms and shading still evaluate normally.
One bake belongs to the instantiated object and composition timing that produced
it. Reusing it in another shot is an explicit frozen-data choice, not an automatic
simulation cache hit. Changing source rates, seeds, dependencies or parameters
requires an explicit new bake. There is no silent claim that a source fingerprint
was revalidated or that a changed live simulation matches a frozen asset.

`VOL8` requires a manifest `sha256`, prohibits `first`, `last`, `fps`, and any
missing-file substitution on `format="srvseq"`. `interpolation` may be `hold`,
`linear`, or `advect` (with the three velocity channels required by VOL9); timing
and the sample count come from the manifest. Numbered SRVOL
sequence semantics remain unchanged. SRVSEQ samples include all five native
fields: density, temperature, velocity.x, velocity.y and velocity.z. An absent
object at a sampled composition time is a transparent entry. The interval is
`[start, start + sampleCount/fps)`. Rendering outside it is an error rather than
silently extending a partial take. Linear sampling clamps to the final sample
within its last frame interval. Integral sample times are snapped only within a
floating-point error bound capped at one millionth of a frame, preventing a
rounding error in a nonzero start from selecting the previous frame.

SRVSEQ version 1 has this canonical little-endian binary layout:

| Field | Encoding |
|---|---|
| Magic | Eight bytes `SRVSEQ\r\n` |
| Version | u32, exactly 1 |
| Start | f64, finite nonnegative composition seconds |
| Sampling rate | f64, positive finite frames/second |
| Sample count | u32, 1–100,000 |
| Entries | Sample-count repetitions of SHA-256 (32 raw bytes) and byte length (u64) |

The manifest has exactly `32 + 40*sampleCount` bytes; trailing or truncated data
is invalid. A zero length with an all-zero digest represents an absent sample.
Other entries name `<lowercase-hex-digest>.srvol` in the manifest's directory;
no authored relative path or external URI occurs in the binary manifest.
Repeated digests share a file and must agree on its length. Every file is decoded
with SRVOL byte/brick/channel limits and verified against the digest of the bytes
actually decoded. Invalid headers, versions, times, lengths, dimensions, sparse
fields, hashes or budgets are errors. The asset verifier checks every distinct
frame; the renderer verifies frames when first loading them. Loaded caches are
immutable during one renderer's lifetime. Reload/recreate the renderer after
changing assets; modifying an admitted frame in place is outside that contract.

The writer hashes canonical fields before creating each unique frame, bounds
individual frames and total output, flushes completed files, and publishes the
manifest by rename only after all samples succeed. Identical fields deduplicate
without retaining every sample in RAM. The renderer bounds manifest retention
to eight manifests (at most approximately 32 MB of binary entries before Rust
container overhead), frame-cache retention to 256 MiB, and per-file decoding to
its storage binding limit capped at 128 MiB. A requested frame pair has an
independent 256 MiB resident budget. The generic writer/reader defaults allow
256 MiB per frame; a larger authoring frame can consequently exceed the target
renderer limit and must be downsampled or rendered on a compatible path.

### Volumetric objects and shading

Add `object3D@primitive="volume"` and a `volume` IDREF to a volume asset.
Reuse existing object transforms, visibility, parent, start/end and motion blur.
Add one optional owned `<medium>` child to volume and pyro objects:

| Attribute | Type and default | Meaning |
|---|---|---|
| `densityScale` | finite real ≥ 0; 1 | Multiplies sampled density |
| `extinction` | finite real ≥ 0; 1 | Extinction per scene unit at density 1 |
| `albedo` | color; white | Scattered fraction of extinguished light, each RGB channel 0–1 |
| `anisotropy` | finite real in [-0.99,0.99]; 0 | Henyey–Greenstein phase parameter |
| `emissionColor` | color; black | Scene-linear radiance source color |
| `emissionScale` | finite real ≥ 0; 0 | Emissive source strength |
| `blackbody` | boolean; false | Use temperature-dependent emission |
| `temperatureScale` | finite real > 0; 1 | Multiplies kelvin values before shading |
| `stepSize` | finite positive length; 1 | Maximum world-space integration step |
| `maxSteps` | integer 1–65536; 2048 | Ray integration budget |
| `lighting` | `exact` or `grid`; `exact` | How the path tracer lights in-scattering (see Light grids) |
| `lightGridCell` | integer 1–64; 1 | Grid node spacing, in voxels of the finest density grid |
| `lightGridDomeDirections` | integer 8–512; 64 | Fixed environment directions of an anisotropic medium's grid |
| `lightGridMemoryMiB` | integer 1–4096; 128 | Largest memory the grids of the pass may take |

Extinction integrates over world distance, including nonuniform object scale.
Overlapping media contribute to total extinction and source terms; reversing
node order must not change the result. Opaque geometry bounds integration;
transparent surfaces and medium self-shadowing must respect transmittance.
Camera rays starting inside a volume are valid. Bounds must include interpolation
support, and voxel empty-space skipping must not skip nonzero background values.
If `maxSteps` cannot satisfy `stepSize`, report it instead of silently truncating
the medium. Temperature emission requires an available temperature channel.

For `blackbody="true"`, the volume asset must declare `temperatureGrid` (VOL5),
and the cache must contain that channel. Its grid transform and resolution may
differ from density. Interpolate kelvin in that grid's world space, then multiply
by `temperatureScale`, then evaluate emission; interpolating pre-shaded RGB is
not equivalent. Scaled stored values, including the background, must lie in
0–50,000 K. Invalid values fail before transport, even when `emissionScale=0`.

Thermal RGB is the Planck spectrum integrated from 360 to 830 nm at 5 nm intervals
using trapezoidal quadrature and the CIE 1931 approximation in
[Wyman, Sloan and Shirley (2013), equation 4](https://jcgt.org/published/0002/02/01/).
Divide XYZ by the same integral's Y at 6500 K, convert to linear sRGB, and clip
negative channels. Convert that RGB to the scene's linear working primaries,
again clipping negative channels. This fixed reference retains the increase in
visible brightness with temperature; it does not normalize every temperature.
Zero kelvin emits zero. The per-density source is
`emissionScale * (emissionColor + thermalRGB)`, where `thermalRGB=0` when
blackbody is disabled. Density and world-space integration are applied afterward.
This is an explicitly normalized cinematic source, not an SI gas emissivity model.

The GPU uses 1025 RGB samples at `T_i=50000*(i/1024)^2` and linear interpolation
in `sqrt(T/50000)*1024`. Tests compare this bounded table against direct spectral
integration, and compare GPU transport against a separate CPU integrator.
Temperature and emission scales may be animated through the medium's existing
animation children; playback and backward seeking must produce the same frames.

The initial transport implementation uses scalar extinction and scene-linear RGB
albedo/emission. It integrates Beer–Lambert attenuation and constant-coefficient
emission analytically within midpoint-sampled steps; heterogeneous density is
trilinearly interpolated. It splits rays at medium domain boundaries and combines
overlapping source and extinction coefficients before integration. Single
scattering includes analytic lights, sampled dome illumination, surface shadows
and medium self-shadowing. Authored ambient light remains an isotropic local
lighting approximation. This is a cinematic single-scattering model; it does not
claim multiple-scattering convergence. The coefficient and phase conventions
follow [PBRT's volume transport](https://pbr-book.org/4ed/Volume_Scattering/Transmittance).

Object opacity multiplies density, including emission density. Existing
`castShadow` and `receiveShadow` control light attenuation while camera-ray
attenuation remains active. Passes containing volumes use the shared transport
pipeline so surfaces and media have consistent depth, including when the camera
selects raster rendering. In that case the initial quality settings are four
samples, two surface bounces and denoising; an explicit path-tracing camera uses
its authored settings. Failure to represent the volume or meet resource budgets
is an error, never a successful surface-only fallback.

The initial portable implementation allows 64 domains in one 3D pass and packs
medium records and sparse bricks into the same bounded binding as surface data.
It checks transformed domain diameters against the smallest integration step and
the smallest `maxSteps` across the pass. This is conservative: a short particular
camera ray can fit even when the domain-wide bound fails. Unsupported f32
transforms, coefficients or large voxel index coordinates produce actionable
errors. The cache decoder and renderer both bound file bytes, brick counts and
cached storage; schema validation alone cannot establish these data-dependent
limits.

**Cells of negligible density are skipped, and bricks are found through a
directory** (the skip: commits 574d621 and 2afdecd; the directory: 73f9535 and
eb45dd7; 2026-10-04; each pair is one change on two branches). The in-volume shadow
march does not sample cells that cannot matter. A per-volume map of 4-voxel cells,
built on the CPU, marks the cells whose readable voxels could add more than 1e-10 to
the optical depth along any ray (the largest density times density scale times
extinction times the longest path in the domain; about 300 times under half an ulp of
an f32 near 1), and the march jumps over the others. It is a bound, not zero, so the
transmittance is unchanged at its resolution. Media with a second frame or advection
and grids with a non-negligible background are marched as before, and the primary
march is untouched. A voxel read finds its brick through a table with one row index
for each possible brick position, packed beside the record, when the grid's bricks
span at most 2^20 positions (at most 4 MiB, counted in the volume's memory); a larger
extent keeps the binary search, and the velocity and second-frame grids have no
directory. Only the lookup changes: the trilinear arithmetic is untouched. Frozen
plume, 1280 x 720, trace seconds (original, cell skip alone, both changes): sun only
5.76, 2.68, 1.14; step 3 6.94, 4.14, 1.40; dome only 19.56, 12.39, 3.75; full 26.82,
15.32, 4.66; the frame's PNG hash is identical in all four, in each stage. The commit
messages do not record the adapter or the load of these timings. Tests (unit tests of
`crates/sr-gpu/src/volume.rs`): `skip_map_clears_only_cells_whose_readable_voxels_are_negligible`,
`the_skipped_density_shrinks_with_extinction_density_scale_and_domain` (a thin, very
dense medium has no negligible density at 1e-6; no extinction makes every cell
negligible), `media_with_a_second_frame_or_a_nonzero_background_are_not_skipped` and
`brick_directory_covers_the_bricks_and_gives_way_to_the_search_when_too_large`.

#### Light grids

`lighting="exact"` (the default) marches a shadow ray from every in-scattering
sample to every analytic light and one environment direction. It is the reference:
its pixels do not depend on whether grids exist. `lighting="grid"` replaces those
rays, for the path tracer only, with lookups into grids built once per frame:

- one world-space regular lattice per path-traced 3D pass, covering the union of
  the bounds of the media that ask for it, with spacing equal to the finest density
  voxel among them times `lightGridCell` (the finest request wins);
- per analytic light, one scalar per node: the medium's transmittance toward the
  light's centre times the visibility of opaque and alpha-blended surfaces. Surface
  visibility is the mean of eight points jittered within one cell around the node,
  so a surface thinner than a cell shadows smoothly instead of aliasing;
- for the environment, radiance pre-integrated over `lightGridDomeDirections`
  fixed directions when every asking medium has `anisotropy` 0; otherwise one
  scalar grid per fixed direction, weighted by the medium's phase function;
- trilinear lookup. A scattering point outside the lattice (including all
  points of media that did not ask) is lit by the exact march, so a scene can mix
  both modes.

Grids are rebuilt every frame and every motion-blur sub-frame, because media and
colliders move. A frame whose lattice needs more nodes than one dispatch can hold
(about 4.19 million) or more memory than the smallest `lightGridMemoryMiB` among
the asking media is a render error that names the attribute to change; it never
falls back to exact lighting or a coarser lattice. `VOL10` rejects `lightGrid*`
attributes without `lighting="grid"`.

Dust derived from solids is thick. A dust density that is a volume fraction of
solids (about 1e-3 per cell) has an extinction coefficient of roughly
3Q/(2d) per unit of solid fraction, with Q about 2 and d the grain diameter in
scene units, so `medium@extinction` is of the order of 1.5 divided by the grain
diameter. On the impact smoke at 1 m cells (100 micrometre grains, so 30000):
300 is faint but visible, 3000 reads as smoke and 30000 is an almost opaque dome
with a hard edge (1280×720 frames judged by eye, 2026-10-04, commit e144806; ledger:
"Light grids for the in-scattering of media"). Such a medium is thick across a cell
for grid lighting (see the known limits below).

Tiled and whole-frame renders of a grid frame are identical. Surface shading of
volume shadows (a surface lit through the medium) still uses the exact march.

Known limits, against `lighting="exact"` at 512 samples per pixel with the default
`lightGridCell` 1 and 64 dome directions, on the plume of a 1280×720 frame, over the
region above the horizon (PSNR and CIEDE2000 ΔE; measured on the NVIDIA adapter on
2026-10-04 with the shader of commit ce124c6, load not recorded; ledger: "Light grids
for the in-scattering of media"):

| Case | PSNR | max ΔE | mean ΔE |
|---|---|---|---|
| Plume, sun and anisotropic dome, no ejecta | 56.4 dB | 1.50 | 0.25 |
| The same plume with 100,000 ejecta fragments in and around it | 47.8 dB | 3.89 | 0.48 |

- Surfaces much thinner than a cell are not resolved: the grid sees them as a
  smoothed shadow, so the ejecta case stays under the 50 dB and ΔE 2 figure that
  holds without thin occluders. Part of the difference is the two renders'
  independent sampling noise at 512 samples; it was not separated out. Finer cells
  help little once the visibility sampling is smooth (cells of 2 and 3 voxels gave
  45.4 and 44.3 dB against 46.2 dB at 1 on the sun-only variant).
- An area light (rectangle, disc or sphere) close to the volume has a penumbra
  the grid replaces with the visibility toward the light's centre: 65, 53 and 43 dB
  for a rectangle light at 60, 14 and 9 scene units from the medium, 66, 56 and
  50 dB for a sphere light (same measurement). Use `lighting="exact"` for lights
  within about ten times their own size of the medium.
- Optically thick cells are lit less accurately. The grid interpolates
  transmittance linearly between nodes, which is a poor model where transmittance
  falls from 1 to near 0 inside one cell. The renderer measures the optical depth
  across one cell (extinction × `densityScale` × the peak density × the node
  spacing) and, above 4, reports it in the frame's unsupported notes with the value
  and the advice to use `lighting="exact"` or a smaller `lightGridCell`; the frame
  still renders. On the impact smoke at 1 unit cells and extinction 30000 (depth 40
  per cell; 640×360, region around the dome, against an exact render with `stepSize`
  0.05 at 32 samples per pixel, so the reference is itself noisy; 2026-10-04, commit
  00874e2, load 10 to 15) the grid gives 34.6 dB as rendered and 40.2 and 41.0 dB
  after Gaussian blurs of 2 and 4 pixels, which remove the noise and keep the
  structure. The horizontal and vertical lines
  visible in very thick smoke come from the 1 unit voxels of the density field,
  each about 40 deep optically, and appear equally in the exact render (mean
  row-to-row step in the dome interior: exact 0.0089, grid 0.0091). Depths 13, 6.6
  and 4 (extinctions 10000, 5000 and 3000) show faint lines, barely visible ones
  and none (one frame, judged by eye).
- Transmittance, not optical depth, is what the grid interpolates. Interpolating
  optical depth with the visibility kept separate was measured on a prototype and
  not adopted (ledger: "Light grids for the in-scattering of media").
- A medium with `anisotropy` ≠ 0 costs one scalar grid per fixed dome direction;
  the lattice, `lightGridDomeDirections` and the number of lights must fit in
  `lightGridMemoryMiB`.
- Spherical harmonics, per-tile grids and multiple scattering are not part of the
  grid.

### Pyro simulation

Add an owned `<pyro>` child to `object3D primitive="volume"`. Exactly one volume
source is required: either `@volume` or one `<pyro>`, never both (VOL1); `@volume` names an
existing volume asset (VOL2); an object has at most one `<medium>`, and `medium`, `@volume` and
`<pyro>` belong to the volume primitive only (VOL3); explicit medium bounds are all six
coordinates, each minimum below its maximum (VOL4). The object
supplies the existing 3D pose, timeline, visibility and `<medium>` behavior.
This composition reuses the volume renderer and avoids a second transform or
material implementation. Native pyro always supplies density, temperature and
three velocity channels; a blackbody medium therefore needs no cache asset.

The implemented configuration is:

| Attribute | Type/default | Meaning |
|---|---|---|
| `width`, `height`, `depth`, `voxelSize` | required positive lengths | Object-local, centred domain; each dimension divided by voxel size must be an integer in 2–1024 (PYRO1) |
| `dt` | positive seconds; 1/60 | Fixed simulation step |
| `boundary` | `closed` or `open`; `closed` | No-through-flow walls or zero-pressure exterior |
| `ambientTemperature` | kelvin in (0,50000]; 300 | Initial temperature and exterior/background temperature |
| `dissipation`, `cooling` | nonnegative rates/second; 0 | Exponential density decay and decay of temperature above ambient |
| `buoyancy` | nonnegative scene units/second²/kelvin; 0 | Acceleration toward negative y for excess temperature |
| `vorticity` | nonnegative confinement strength; 0 | Restores small-scale rotation lost to advection |
| `turbulence` | nonnegative acceleration amplitude; 0 | Seeded three-component force, followed by pressure projection |
| `seed` | unsigned 64-bit integer; 0 | All bits participate; no intermediate floating-point conversion |
| `pressureIterations` | integer 1–10000; 200 | Maximum pressure-solver iterations |
| `solver` | `jacobi` or `multigrid`; `jacobi` | Preconditioner of the pressure solve; see the numerical contract. Absent equals `jacobi` and reproduces earlier results bit for bit |
| `advection` | `semilagrangian` or `maccormack`; `semilagrangian` | Transport scheme for density, temperature and velocity; see the numerical contract. Absent equals `semilagrangian` and reproduces earlier results bit for bit |
| `pressureTolerance` | positive 1/second; 0.000001 | Maximum RMS divergence residual |
| `maxMemoryMiB`, `checkpointMemoryMiB` | integers 1–4096; 256 each | Separate solver/workspace and checkpoint budgets |
| `meshMemoryMiB` | integer 1–4096; 128 | Aggregate conservative charge for distinct mesh regions in this domain |
| `forceFields` | optional IDREFS | Physics force fields acting on this domain; absent selects all (R34 validates references) |
| `useForceFields` | boolean; true | Disable all scene force fields when false |
| `colliders` | optional IDREFS | Up to 4096 distinct rigid collision proxies; absent selects none (PYRO7) |
| `colliderThickness` | optional positive length; twice `voxelSize` | Thickness of referenced planes, in each plane's local units |

**Memory budget by resolution.** The two budgets are checked before any grid is
allocated. `maxMemoryMiB` must be at least `ceil((cells × 288 + 8192) / 2^20)`: 288
bytes per cell is the worst-case live memory of one step (the committed state, the
working state, the list of solid-face velocities if every cell were solid, the
pressure workspace and the multigrid hierarchy, about 229 bytes) plus headroom for
allocator overhead. `checkpointMemoryMiB` must be at least
`ceil(state bytes / 2^20)`, where the state (density, temperature, three face
velocity arrays and the solid mask) takes about 41.2 bytes per cell plus a few
hundred bytes of bookkeeping, independent of colliders. A checkpoint is taken every
second of simulation time; when the budget fills, the spacing doubles and the
checkpoints between are dropped. Below these minimums the errors are
`grid and step workspace memory budget` and
`checkpoint budget cannot hold the initial state`. When only the initial state
fits, a backward seek replays from the start.

| Grid (cells) | Cells | Minimum `maxMemoryMiB` | Minimum `checkpointMemoryMiB` (one state) | States in 256 MiB |
|---|---|---|---|---|
| 64×52×64 | 212,992 | 59 | 9 | 30 |
| 128×104×128 | 1,703,936 | 469 | 67 | 3 |
| 192×156×192 | 5,750,784 | 1580 | 226 | 1 |

The minimums are the least values the constructor accepts, found by running the
solver, and they match the formulas above. Peak resident memory measured on the
impact scene (6 steps, multigrid, no colliders) was 326 MB at 128×104×128 and 928 MB
at 192×156×192, below the estimate because untouched pages of zeroed arrays are not
resident; those figures were measured on a loaded machine and depend on the
allocator.

`<pyroSource>` accepts `shape="sphere|box|mesh"`, with radius (default 1),
required positive width/height/depth for boxes (PYRO3), or a mesh asset IDREF
in `mesh` (PYRO5). Its x/y/z, rotation,
rotationX/Y and scaleX/Y/Z transform the shape into object-local domain space.
Scales must be nonzero (PYRO4). `start` defaults to zero; optional `end` must be
greater than start (PYRO2). `densityRate` and `temperatureRate` add nonnegative
density and kelvin per second. Signed `velocityRateX/Y/Z` add object-axis velocity
per second, in scene units/second². Signed `expansion` specifies target divergence
in 1/second. These source properties support the existing animation children,
sampled at each fixed step's source time (use `timeBase="local"` for keys relative
to the object start), and source windows contribute only
their overlap with that step.

Shape selection and mesh asset selection are static (PYRO6); source transforms
and rates can animate. Imported meshes use the asset's rest pose and scene-axis
conversion, including node transforms and reflected winding. Exact coincident
positions are welded across normal/UV seams. Open, nonmanifold, degenerate and
inconsistently oriented surfaces are rejected. Closed, non-self-intersecting
surfaces are required: inside tests use the nearest oriented surface, with a
globally reversed winding normalized and oppositely oriented cavities retained.
Geometric self-intersection detection is not implemented. Mesh regions are
shared within the domain and checked against their budget before BVH creation;
input-file size and flattened geometry are also bounded. Material, texture and
external-buffer allocations inside the existing asset importer are outside this
region budget. Included documents resolve their own mesh assets and base paths.

`<pyroImpulse>` shares the shape attributes, has a required nonnegative `time`,
and adds one-shot `density`, `temperature`, and `velocityX/Y/Z` totals.
Its `expansion` is integrated divergence, divided by the receiving step's dt.
Impulse declarations are static and have no animation children: animating an
event's time must not repeatedly fire the same authored event. Events on an
exact step boundary belong to the following half-open interval, including when
decimal times require floating-point rounding correction.

The CPU implementation stores centred density/temperature and staggered MAC
velocities in bounded dense arrays, then exports sparse SRVOL-compatible fields.
It uses midpoint semi-Lagrangian advection, timed injection, exponential decay,
buoyancy, confinement and matrix-free Jacobi-preconditioned conjugate-gradient
pressure projection. The discretization follows
[Bridson and Müller-Fischer's SIGGRAPH 2007 notes](https://www.cs.ubc.ca/~rbridson/fluidsimulation/),
with confinement following
[Fedkiw, Stam and Jensen (2001)](https://www.graphics.stanford.edu/papers/smoke/).
An unconverged projection returns an error; it does not silently accept a
divergent field. In particular, a sealed domain cannot sustain net positive
expansion. Failed steps preserve the previous state and clock.

**Pressure solver (`solver`).** `jacobi`, the default, is the diagonal-
preconditioned conjugate gradient above, with reductions that add in cell-index
order; it defines the results of every earlier release. `multigrid` runs the same
conjugate-gradient iteration with a different preconditioner and different
reductions, and these choices define its result bits:

- Coarsening is algebraic aggregation of 2×2×2 cell blocks (`ceil(n/2)` per axis;
  the last block of an odd axis has fewer children) with the Galerkin operator
  `PᵀAP` for piecewise-constant `P`: the weight between two aggregates is the
  number of open fluid faces between their children, and the diagonal is the number
  of open-boundary (zero-pressure) faces plus the incident weights. Solid cells
  have no unknown. Coarsening stops at 128 cells or fewer, and that level is solved
  directly by dense Cholesky.
- One V(1,1) cycle per application: a red-then-black Gauss–Seidel sweep from a zero
  guess (colour is the parity of `i+j+k`), restriction of the residual by summing
  children, the coarse correction scaled by 1.5, then black-then-red
  post-smoothing. The two sweeps are adjoint, so the cycle is a symmetric positive
  definite operator, as conjugate gradients requires.
- Reductions add fixed blocks of 2048 consecutive cells in index order, then
  combine the block sums pairwise (0+1, 2+3, …) in a fixed binary tree, so no value
  depends on the thread count.
- In every connected fluid component without a zero-pressure face (a closed domain,
  or a pocket sealed by solids) the operator is singular, so the mean of the
  right-hand side over that component is removed and one coarse unknown per such
  component is pinned to zero. The removed mean stays in the divergence measured
  after the solve, so a component that genuinely cannot be made divergence-free
  still fails the tolerance check instead of being absorbed.

For both solvers `pressureIterations` is the maximum number of conjugate-gradient
iterations, iteration stops when the RMS residual over fluid cells is at most
`pressureTolerance`, and a step whose recomputed RMS divergence error exceeds 1.01
times the tolerance is an error. That criterion is a global RMS over all fluid
cells, so an error confined to a few cells of a large domain can pass. Multigrid
results are identical for any thread count but differ from `jacobi` in the last
bits (the evaluator test compares them to within 1e-6 relative on a 16³ domain).
Iteration counts stay roughly independent of resolution: measured on the impulse
step at 128³, `jacobi` needed 275 iterations and `multigrid` 14. Baked SRVSEQ
caches identify a solver only through their frame contents (the digests of the
exported frames), so two solvers that produce identical bytes share frames.

**Advection (`advection`).** `semilagrangian`, the default, is the midpoint
semi-Lagrangian scheme above; it defines the results of every earlier release and
is unconditionally stable but numerically diffusive. `maccormack` is the
backward-forward error correction of Selle, Fedkiw, Kim, Liu and Teran (2008)
with an extrema limiter. For every element at position `p` of a channel:

1. `hat` is the semi-Lagrangian value at `p`.
2. `til` is `hat` sampled where the forward trace of `p` (the same trace with the
   step negated) lands.
3. The value is `hat + 0.5 * (old[p] - til)`, clamped to the minimum and maximum of
   the eight old values that the interpolation blends around the backward-trace
   origin (an out-of-range corner with nonzero weight contributes the channel's
   background value), so the scheme creates no new extremum: density stays
   non-negative and temperature stays within its previous range.
4. Where a collider cuts either trace short, the value is `hat`.

The scheme applies to density, temperature and the three velocity components;
density and temperature share their traces and each velocity component has its
own. Dissipation and cooling apply afterwards to the limited value, as they apply
to a semi-Lagrangian sample, and solid cells keep their cleared values. Each
element is computed from immutable inputs by the same expression, so the result
does not depend on the thread count. The advection stage costs about 2.5 times the
semi-Lagrangian stage (2.5 at 64³ and 2.4 at 128³ on the impact scene with one
thread). On a free Gaussian vortex over 60 steps the scheme kept 96.6% of the
kinetic energy and 95.7% of the enstrophy, against 68.8% and 61.6% for
semi-Lagrangian advection (measured with the detail-retention test setup; the test
itself requires only 1.25 and 1.3 times the semi-Lagrangian values and no energy
gain). `solver="multigrid"` with `advection="maccormack"` is a supported
combination and defines its own result bits.

Checkpoints are thinned before cloning to respect their independent hard budget.
Backward requests replay the same fixed steps and input samples. Source animation
uses the existing ancestor-clock inversion and clock overrides; nonlinear parent
clocks rebuild branch-dependent history. The object's animationSpeed/Offset map
its local time into simulation source time. Exported grid values and transforms
contribute to compositor cache keys, including isolated groups.
When a condition omits the owner during part of its source history, existing
fields continue to advect and cool, with no new source injection in that interval.

Scene force fields use the existing particle/fluid selection: `affects="all"`
and `affects="particles"` apply; body-only fields do not. Activation windows and
animated field properties are sampled at each fixed step's mapped scene time.
Positions and velocities are transformed into scene space, acceleration is
evaluated there in three dimensions, then transformed back into domain axes.
Physics-driven domain parents are sampled at that substep as well. Drag reads
the pre-step velocity, including after checkpoint restoration. Fields retain
their existing unit conversion through `physics@pixelsPerMeter`; velocity
vectors follow the domain's linear transform. This local-domain smoke model
does not add inertial forces for accelerating reference frames.

Scene colliders reference box, sphere, globe, plane, cylinder, cone, capsule,
torus, mesh, text, extrude or clay objects. Text/path extrusions and clay use their
closed rendered surfaces, preserving glyph counters, path holes and carved clay.
Without a crater, the other curved primitives use analytic regions;
plain globes use spherical collision proxies and planes use finite slabs.
Globes with elevation use the shared closed terrain surface described below.
Meshes use the closed rest-pose geometry contract above, sharing the domain's
mesh budget with sources and relieved globes. Primitive, mesh and dimension
selection remain static (PYRO8); animate position, rotation and scale for rigid
motion. Text/font/path/bevel edits, clay field/finish edits, animated blob inputs,
and nonzero fingerprint boil also count as changing geometry. Skinning, morphing
and topology changes are not interpreted as
deformation of these rigid proxies. Owned craters use the deforming closed-mesh
contract below. Unsupported reference types are rejected
rather than replaced with a box. Instances and includes resolve references in
their lexical scope; 3D transform parent and constraint links use the same
compiled target IDs in rendering and simulation.

At each fixed step, start/end windows and conditions determine collider
presence. Render visibility is independent, allowing `visible="false"`
collision proxies. Collider and domain poses, including rigid-body simulation,
are sampled at the beginning and end of the step. Boundary motion is relative
to the domain, so moving both together introduces no relative flow. For local
point q and poses p0=A0*q+t0, p1=A1*q+t1, the midpoint velocity relation is
`(p1-p0)/dt = G*((p0+p1)/2 - (t0+t1)/2) + (t1-t0)/dt`, giving
`G = (A1-A0)*inverse((A1+A0)/2)/dt`. For pure rotation this gradient is
skew-symmetric, avoiding artificial volume expansion. The affine field is
evaluated at MAC faces. Singular midpoint motion (for example a reflection
change across one step), invalid transforms and overflowing fields return an
error without committing the failed step; reduce the step or correct the motion.
When a condition removes the next pose, the last available pose is held for
that interval. Window boundaries retain pose sampling across the boundary.

Geometry is voxelized at cell centres; surfaces and motion must be resolved by
the grid and fixed step. Overlapping colliders use the first listed collider's
velocity in each occupied cell. Coupling is one way: collider motion drives
smoke, without pressure forces feeding back into rigid bodies. These are
cinematic collision proxies, not a compressible impact/contact solver.

The implemented `bake-volume` command freezes the resulting fields into an
SRVSEQ render asset, described below. It is not a serialized solver checkpoint:
f64 MAC faces, pressure state and checkpoint history are not restored from the
f32 render fields. Source rates are per second; an impulsive source declares a
time and a total amount separately.

Colliders reference 3D geometry and are sampled at substep time. Source shapes
are transformed into the domain, including rotation and scale. Pressure
projection, advection, temperature buoyancy, dissipation and cooling are actual
solver operations, not animation of a displayed noise texture. Expansion is an
authored cinematic divergence source, not a shock-physics equation of state.
Grid and checkpoint memory use explicit budgets; exceeding a budget is an error.

Sources near an open face (warning `W02`). An open face is a zero-pressure outlet and an inlet for the
surrounding air, so a source close to one changes the flow of the whole cloud, not only the part that leaves.
Measured on the plume of the hero scene (64 x 52 x 64 cells of 3 units, the impulse sphere of 7.3 cells radius
above the bottom face): the peak density at 6 s is 0.307, 0.476, 0.525, 0.673, 0.675, 0.688 and 0.686 with the
sphere's edge 3, 5, 7 (as authored), 13, 19, 33 and 59 cells from the bottom face, the tail of the plume 28, 28,
28, 34, 39, 40 and 41 cells down, and with the top face moved 26 cells away instead nothing changes (peak 0.481
against 0.525, same extent). A document that gives a pyro source or impulse a place (a sphere or a box, no
animation, not from a crater) in a volume with `boundary="open"` that has room on that axis for the source and
12 cells at each end, and puts its edge less than 12 cells from an open face, is warned (`W02`, not an error;
the message names the face and the distance). The 12 comes from one plume, one face (the bottom) and one kind of
source; the side faces were not measured; and a source with a rotation or a scale is bounded by a sphere
or by its extent along the axis.

#### A window that follows its plume (`pyro@follow`)

An open domain cuts a plume that rises past its top face: the face is a zero-pressure outlet and what crosses it is
lost. `follow="true"` (PYRO9: only with `boundary="open"`) makes the window of the domain move, in whole cells and in the
object's own axes, to keep the smoke `followMargin` cells (PYRO10: only with `follow`; the engine's value is 12, the
distance from which a plume's top face was measured to have no effect on it, see W02; PYRO11: it leaves a cell between the
faces) from the faces it is going toward, plus the cells the fastest air along the axis goes in one step; the number of
cells does not change, so the memory and the cost of a step do not either (a move is a copy of the state, and about a state
clone has been measured at 102 ms at 192 x 156 x 192 under load). Absent or false the domain is where it began for good, and
with a follow that never moves the state is bit for bit the one without it (tests
`a_following_window_that_the_smoke_never_nears_the_faces_of_is_bit_for_bit_not_following`, the nine reference hashes of the
solver, and `a_follow_that_is_false_or_absent_is_the_domain_it_always_was`).

Where the window moves is a function of the state and of the sources that act in the step, never of the history of
requests. The smoke along an axis is the span of slabs of cells between the ends that hold at most half of `followLoss` of all
the density each (the lowest and highest cell index, integers, from sums taken in the order of the cells, so the same on any
number of threads). A face is asked for room only if the air in the smoke nearest it is going toward it (the density-weighted
velocity along the axis, summed over the slabs of the smoke within the margin of that end, points to the face): a face behind a plume
that rises away from it is no reason to take from the room it rises into (test
`a_window_does_not_move_against_the_drift_of_its_smoke_to_make_room_behind_it`, the oracle of the policy: a puff at rest or rising
away from a face must not move the window toward it), and smoke at rest is going toward none. The hero scene is not what justifies
the policy: the window of the hero is the same under the first policy (d87fc65, every face asked for room) and under this one (80f70d5),
the first cell of the grid at y = -78, -72, -72, -72, -75, -90 over the six times measured with the first and -78, -72, -72, -75, -90 at
1, 1.25, 3, 5 and 5.96 s with this one (6 units down at about 1.1 s in both). A figure of 22 units that an earlier version of this section
gave for the first policy on the hero is withdrawn: it is not a measurement of either commit. On the hero scene with `followMargin="3"` the window still moves down 6
units (two cells) at 1.1 s, when the expanding fireball comes within the margin of the bottom face, and stays there to 4 s; a
smaller margin keeps it where it began for longer, at the price of smoke nearer the face. The sources are the
shapes of the `pyroSource` and `pyroImpulse` that act in the step, and **a window never leaves a slab that holds a cell of such a
source**, whatever the loss, because a plume begins at its source and a window that left it would part the plume from the
ground. The inputs of a step are sampled again if the window moved, so that they are those of the window the step has, and
the seeded turbulence is keyed by the cell of space (the cell of the window and the cells the window has moved by), so that a
point keeps its noise. The state keeps the cells it has moved by (so its origin is `base + cells * voxelSize` computed afresh)
and the density it has let go of (`State::lost`); both are in the checkpoints and in the identity of the state, and a seek
replays the same moves. The volume exported to the renderer places its grids by the origin of the state, so the smoke does
not move in the world; a frozen bake of a following plume keeps the transform of each frame (test
`a_baked_sequence_of_a_following_plume_is_the_frames_it_was_baked_from`), and any order of times and a fresh evaluator give the
same frames.

`followLoss` (0 to 1, the engine's default is 0) is the share of all the smoke that a move may leave behind on each side.
**With a loss of zero a window lets go only of slabs that hold no density and no heat, and that is almost never**: the
interpolation of the advection leaves a tail that is never exactly zero, and a plume drags a stem of smoke down to its source.
Measured (sr-sim `follow`, a blob of smoke rising in air that accelerates upward at 2 units a second squared, 140 steps of
0.05 s, cells of 0.5 units, a window of 80 rows; deterministic; the reference is a window of 640 rows): the reference holds 164.3
of smoke with its centre at -38.5 units; a window that stays holds 12.9 (centre -28.5); a loss of zero never moves and holds
the same 12.9, a loss of 1e-9 holds 42.6, of 1e-6 holds 174.0 (centre -37.9) and has let go of 9e-4 in all, of 1e-4 178.6 and of 1e-3
181.0. The follow holds up to 10 % more smoke than the reference because the open bottom face is not where the reference's is, a
difference of the two domains and not of the follow. What a move lets go of is counted in `lost`, never silent, and a move lets go of
at most the share asked (`what_a_move_lets_go_of_is_counted_and_is_never_more_than_the_share_asked`: the smoke before a move
equals the smoke after it plus what it let go of, to a trillionth, and no move lets go of more than 1.5 times the loss of all the
smoke, the three axes together). In the evaluator (`crates/sr-eval/tests/volume/pyro_follow.rs`: a blob in a domain of
8 x 60 x 8 units that a force field accelerates upward at 6 units a second squared) the fixed domain holds nothing at 5 s and the
following one holds the blob 14 units above the old top face.

On the hero scene (`examples/cinematic-impact/hero.scene.xml`, plume of 192 x 156 x 192 units in cells of 3, open, 24 steps a
second; tool `crates/sr-eval/examples/pyro_extent.rs`, deterministic; the cut figures are the scene's own solver, Jacobi, the others the
multigrid one) the plume is cut by the top face from about 5 s: the smoke in the three cells at the top face is 0.30 of 3526 at 5 s
and 98 of 2972 at 6 s. A domain of 468 units
(the same cells below, two more windows above) shows what the plume is: at 6 s the smoke above a thousandth of the peak spans y = -83
to 62 (145 units), above a hundredth -77 to 53, above a tenth -74 to 8, and the whole smoke is 3016. A window of 156 units
holds that with a margin of 3 cells (9 units) only just: with `follow="true" followMargin="3" followLoss="0.001"` the window moves
down 6 units at 1.1 s (the fireball) and then up, to 12 units above where it began at 5.96 s, the smoke in the three cells at the top
face at 5.96 s is 0.17 of 2999 (88.5 of 2996 without follow, a plume cut by the face) and the plume reaches the margin; a loss of
zero lets go of nothing, never makes room and holds 158 at the top face at 6 s, as little help as a fixed window. A plume that the
hero film follows to its end needs 145 units and its margins, about 240 for a margin of 12 cells, and that is 64 x 80 x 64 cells
against 64 x 52 x 64, 54 % more cells and the memory and cost of a step with them.

The hero scene sets `follow="true" followMargin="3" followLoss="0.001"` on its `<pyro>`: the default margin of 12 cells (36
units there) does not fit a window of 52 rows with a plume of 145 units, and a loss above zero is what makes the window follow
at all. The margin is in cells, so a copy of the scene with cells of 1.5 units (`hero-hires.scene.xml`) needs a margin of 6 for the
same distance. The frame at 3 s of the 720p probe does change, and was expected to: the window has moved at 1.125 s, before 3 s, so the frame cannot be
the one of the scene without follow. Its sha256 has three values, each with its reason: `24c6ab56...` (963b614, the scene without the
follow attributes), `eeb71e14...` (d87fc65, follow merged, the noise keyed by the cell of the
window) and `c8a66483...` (from 80f70d5, and the same on 28887eb and later: the noise keyed by the cell of the box the state began in). The
two commits between the last two (8c30823, the noise key, and 80f70d5, the policy of the faces) change 185,284 pixels of 921,600 (20.1 %,
largest difference 157 levels, mean 0.153). The window of the hero is the same under both, so these pixels are attributed to the noise
key (8c30823: the hero has turbulence, and the noise of a cell depended on where the window was) unless a measurement says otherwise (a probe at
3.0 s with only 8c30823 would decide; it has not been made); the frames without follow are bit for bit the same
in both (frames 120 and 143, sha256 `a9d92b13...` and `ea8a614c...`), so the path without follow did not change. A frame is a function of
the policy, and a change of policy changes the hero's frames after the first move of the window: the hash is not a fixed reference of
the scene but of the scene at a commit.

Limits. A plume has to fit in its window: the window follows the head of the plume only while the stem that joins it to its
source, and the smoke that numerical diffusion spreads about it (the blob above spreads over 68 rows at 140 steps), fit between
its faces with the margin, and a plume that does not fit is cut as it was; the remedy is a larger window, and `follow` is the
saving in cells for a plume that has left its source behind, not a free height. A `followLoss` large enough to let go of the stem
loses visible smoke: the stem is part of the plume (a heated puff keeps 11 % of its smoke in a stem below the rows its head has
left). The margin and the loss are the engine's values with no published source. The decision reads the density, the velocity and
the temperature of every cell once a step (one serial pass), and its cost, measured (sr-sim `follow`, ignored test `cost_of_the_decision_against_a_step`, ci profile, one core for the
decision and the solver's threads for the step, best of five), is 0.0093 s against a step of 0.423 s at 128 x 104 x 128 (2.2 %)
and 0.0334 s against 1.407 s at 192 x 156 x 192 (2.4 %).

#### A blast in the smoke (`pyroBlast`)

`<pyroBlast time energy x y z ambientDensity ambientPressure gamma/>` is a child of `<pyro>` (version 1.3 with it): at `time` seconds `energy`
joules are released at (x, y, z) in the pyro's own axes into air of `ambientDensity` kg/m^3 (1.2) and `ambientPressure` Pa (101325), a gas of
`gamma` (1.4, from 1.1 to 3). A pyro with a blast needs `boundary="open"` (PYC5: a blast is a source of divergence, which a closed domain cannot let
out; the evaluator and the solver refuse it too) and the blast's place is in the domain (PYC6). An energy of zero is a blast that does nothing, and a
pyro with no blast is bit for bit the pyro it was (the reference hashes of the solver, and a test of a puff with turbulence).

**What is solved.** The front is the Sedov-Taylor blast: `R(t) = xi0 (E t^2 / rho0)^(1/5)` metres, where the constant is not quoted but found
(`sr_sim::sedov`): with `xi = r / R`, `u = Rdot U`, `rho = rho0 G` and `p = rho0 Rdot^2 P` the equations of an ideal gas reduce to three ordinary
differential equations that are integrated from the shock (`U = P = 2 / (gamma + 1)`, `G = (gamma + 1) / (gamma - 1)`) to the centre, and the
energy inside the front, `E = (16 pi / 25) J rho0 R^5 / t^2` with `J` the integral of `G U^2 / 2 + P / (gamma - 1)` over `xi^2 dxi`, gives
`xi0 = (25 / (16 pi J))^(1/5)`: 1.0328 for gamma 1.4 (Landau and Lifshitz print alpha = 0.851) and 1.1517 for 5/3 (published 1.15), the mass inside the
front is that of the sphere of ambient gas to 1e-6 (it is not given), the pressure at the centre of the monatomic blast is 0.3062 of the one behind
the shock (published 0.306), and an independent integration in Python agrees to 2e-6. The front stops at `0.3 (E / p0)^(1/3)`, the end of the strong
phase: the radius at which the pressure behind the strong shock, `2 rho0 D^2 / (gamma + 1)` with `D = 2 R / (5 t)`, has fallen to `11.85 xi0^5 / (gamma + 1)`
times `p0` (5.8 for air, 9.0 for the monatomic gas): a few times the ambient pressure, where the blast stops being strong. It is a choice of the radius
and not a result: the engine's, with no published value (the constant 0.3 is not the 0.3 to 0.6 m per cube root of a kilogram of the fireballs of explosives:
`0.3 (E / p0)^(1/3)` is 1.04 m per cube root of a kilogram of TNT).
The smoke solver is incompressible, so the shock is not carried. What is carried is the displacement of the air by the front, as an incompressible
spherical piston. In each step the volume that the front swept, `4 pi (R1^3 - R0^3) / 3` between the sphere it began the step in and the one it ends it in,
is given to the sphere's cells (the cells whose centres are in the sphere of the end of the step; at least a cell: a front smaller than that is the sphere of one cell,
and the volume given is still the volume swept), spread evenly over them: each is given the divergence `volume / (cells h^3 dt)`, so that the air the domain lets out is
exactly the volume swept whatever the cells make of the sphere, an energy of 1 J included (1.1e-6 cubic metres, not the volume of a cell). The cells that share the volume are the
sphere's WHOLE (a window that the sphere cuts holds only some of them, and the others take their share), so that the divergence in the window is the one that the sphere
makes in free space and agrees with the flow outside. The analytic flow of the piston (inside the sphere `u = d (x - c) / 3`, outside `u = Q / (4 pi r^2)` away from the centre `c` of
the blast, `Q` the volume over the step) is put in a velocity of its own and projected ALONE (the projection is linear, so this is the share of the step's flow
that the blast makes), with the faces open at zero pressure and the solids and obstacles of the step; that flow carries the smoke (the density and the temperature) once, over
the step, and it is NOT kept in the velocity of the smoke. Kept, it would stay for ever: a potential flow with open faces is not removed by a projection with
no divergence (the first design of the blast kept it and left 50 to 200 percent of the pulse a step after the front had stopped), so the velocity of the smoke is what it
would have been with no blast, to the bit, at every step (`Simulation::blast_flow` gives the flow of the last step to whoever wants it). A window that the sphere cuts, or that is
not centred on the blast, is pushed from the blast and not from its own middle. Nothing is heated and no smoke is made (a fireball is a `pyroSource` or
`pyroImpulse` that the author adds). The order of a step with a blast is the smoke's own (advection, sources, forces, projection) and then the blast's: its flow is projected alone, with
the solids of the step at rest for it (what a moving collider does to the air is the smoke's own flow, made first, and the two add up to the whole), and it carries the density and the
temperature once, by the same advection with no decay and no cooling (the step has made those: a decay of 1 a second would otherwise be made twice in a step with a blast). The cost of a step with
a blast is a second projection and one more state while the first is held (about 41 bytes a cell, which carries the smoke by exchange of its density and temperature and not by copy, and a copy of the
three faces of its flow, which the advection replaces): the live bytes at the peak of a step are counted by an allocator (`tests/pyro_blast_memory.rs`, 48^3 cells) at 206 bytes a cell with a blast and 141
without, inside the 288 of the budget (a first version of this, which copied the whole state and kept the last step's flow through this one, was 248; 1.1 GB more than a step with no blast at 256^3),
the simulation holds 66.6 bytes a cell after a step with a blast (its state, 42, and the flow of that step, 24, which is dropped when the next begins or the window moves), and the time is in the profile as `blast`.

**Units.** `x`, `y`, `z` are in the pyro's own axes, as those of a source; `energy` is in joules, the air in SI; a metre is the physics element's
`pixelsPerMeter` scene units (100 if there is none) and the volume's axes turn that into its own units (a volume that is not scaled the same on every
axis has no sphere, and is an error: the images of the three axes must have one length and be at right angles, so a shear is refused as well as a stretch). The radius is worked out in metres and then multiplied by that; the divergence is a ratio of volumes per second and
needs no conversion. Tested at 1, 100 and 37 pixels to the metre: the same radius in metres, the same displacement of a puff of smoke in metres
(to 1e-3, the density being a single float), and the same kinetic energy in joules to 1e-6.

**The time step.** With `dt` of the pyro (1/24 s) the strong phase of a plausible blast is over within the first step: the energy above which it is not is
`E* = [xi0 (dt^2 / rho0)^(1/5) p0^(1/3) / 0.3]^(15/2)`, which goes as `dt^3`: 1.91e12 J at 1/24 s and 2.64e10 J at 1/100 s, and below it the blast is ONE PULSE of the volume
of `R_max`, in the step that contains `time`. Sub-steps would not change what the projection gives (the potential flow depends on the volume swept, not on its
history within the step), so there are none. The front, for three energies in air at 1/24 s:

| energy (J) | R(dt) (m) | R(2 dt) (m) | R_max (m) | R_max reached at | steps of the strong phase |
|---|---|---|---|---|---|
| 1e6 | 4.43 | 5.84 | 0.64 | 0.3 ms | one pulse of R_max |
| 1e9 | 17.6 | 23.3 | 6.44 | 3.4 ms | one pulse of R_max |
| 1e15 | 279 | 369 | 643 | 0.336 s (8.06 steps) | 9 (the ninth is the last to sweep) |

The sphere of the front, which stops at `R_max`, reaches the centres of the faces of a window of side `L` if `R_max` is `L / 2`, `E = p0 (L / 0.6)^3`: 9.0e7 J for the
hero's domain of 5.76 m (at 100 pixels to the metre) and 9.0e13 J for one of 576 m; it covers the whole window, corners included, at `R_max = sqrt(3) L / 2`, 4.66e8 J for
5.76 m. A window that the sphere does not cover whole holds the cells it has of the sphere, each given the divergence that the whole sphere gives in free space (the window's
flow is the free-space flow of the sphere, to 5 percent in the test), and the flow of the piston from the centre of the blast.

**What it does.** Measured on a domain of 64 cells with the blast of 3.75e9 J (the strong phase ends at 10 m; `crates/sr-sim/tests/pyro/blast.rs`): the air that
the blast's flow lets out of the domain is the volume swept over `dt` to 1e-6, over one step and over all the steps of the strong phase (the sum is the volume of the sphere of `R_max`), with
a solid in the sphere, and the smoke's own flow in the same step lets out the source's (the two are made apart); the cells of the sphere are its volume to 5 percent; the speed outside the sphere is `Q / (4 pi r^2)` to 15
percent at 1.25 and 1.5 radii (the open faces of the box change it farther out); a window cut by the sphere, with the blast 1 m from its face, has the air go away from the blast, the flow inside linear in the distance from it (0.6 to 3 percent) with the divergence
of the whole sphere (10 percent), and the flow at the middle the one that free space gives (5 percent); the kinetic energy in a ball of 1.5 radii is `rho Q^2 / (8 pi R) (1/5 + 1 - R / a)` to 0.9 percent at half a metre and 0.85 at a
quarter (the piston's own energy is `2 pi rho Rdot^2 R^3` outside and a fifth of that inside: for the strong phase the energy that the solver's flow has is that of the displacement,
which is part of the blast's `E`, not all); a puff of smoke at 6 m is displaced as the volume swept says (`r1^3 = r0^3 + R^3`, 0.51 m) to 0.984 and 0.990 of it at half a metre
and a quarter. With a pulse of many cells in one step (1e11 J, the puff at 9 m, the displacement 1.5 m, a Courant number of 3 at half a metre and 6 at a quarter) the semi-Lagrangian
trace neither leaves the domain nor crosses the sphere, and the displacement is 0.990 and 0.981 of the exact one. **The advection does not conserve the smoke**: it changes by
3.6 percent at half a metre and 0.70 at a quarter (1.8 and 1.4 percent for the pulse of many cells), which is the interpolation of the semi-Lagrangian scheme and falls with the cell;
it is not the 1e-6 of a conservative scheme. This is a limit of the smoke solver as a whole and not of the blast: any large velocity (a pulse, a gust, a fast plume) gains or loses
smoke by the same interpolation, and a conservative advection or a correction of the mass in each step (with the puff of this test as its oracle) is work for the solver. The same
document through the evaluator, after the puff and the first two pulses (the front at 5.14 m; the volume swept says 1.06 m for the puff as a whole), moves the centre of the smoke on the line
through the puff by 1.19 m (the centre of the line, not of the whole puff: 12 percent over) at every scene unit; the test fixes that number to 0.05 m.

**Limits.** (1) The solver is incompressible: no shock, no sound, no overpressure field; the front is prescribed by Sedov's law and not found by the
flow, and the smoke is moved by the displacement and not by a shock. (2) The interior flow is that of a uniform divergence (linear in `r`), not Sedov's
profile, and only the displacement of the front is the blast's: the energy of the flow is the energy of the displacement. (3) The strong phase is shorter
than a step of the pyro for any `E` below `E*`; a blast faithful in time needs a `dt` of the pyro smaller than a tenth of the time of the strong phase (`dt` of 3e-5 s
for 1e6 J), at the cost of that many steps. (4) After `R_max` nothing is modelled: no negative phase, no reflection, and the walls are the solver's. (5) No heat and
no smoke are injected. (6) Because the blast's flow is not kept in the velocity, the smoke's own velocity gets no impulse from it: an updraft or a vortex stays where it was while the density and the temperature move, a plume made after the blast is not deflected by its wind, and the two advections in a step (the smoke's, then the blast's) are a first-order splitting with the numerical diffusion of both. A blast has no compressible after-flow and no negative phase. In a potential incompressible flow no velocity is left when the source stops, which is right. Not done: the coupling to bodies (the front's pressure `2 rho0 D^2 / (gamma + 1)` and the time it takes to pass a body's size give an impulse that the
world's own conservation test can check) and to the ocean (the same pressure as a `waterImpulse`): each its own step with its own oracle.

### Three-dimensional particles

The `sr-sim::particles3d` CPU core and `<particles3D>` scene binding are
implemented. The evaluator uses parent/source clocks, selected 3D force fields,
scoped collider references and simulated body poses. Sphere, billboard, streak
and mesh particles join the native surface pass for depth, lighting, shadows
and shutter sampling. Raster passes batch compatible prototypes; path tracing
shares object-space geometry and BVHs for repeated immutable cached meshes.
Large ejecta-set throughput and the complete UHD impact remain
unverified. The CPU API's verified contract is:

- Births have exact event timestamps; rate quotas integrate over fixed intervals,
  and repeated bursts fire once per event. Burst events at `start` are visible
  at that instant. The emission window clips rate integration at `end` and
  excludes bursts at or after `end`; existing particles live until expiry.
- Point, box and solid-sphere emitters sample local positions. Mesh emitters
  sample open or closed triangle surfaces by **local triangle area**, with
  bounded geometry storage and validation of indices, finite vertices and
  nondegenerate faces. Nonuniform emitter scaling does not reweight faces by
  their transformed area. Unreferenced vertices do not affect sampling bounds.
- Cone directions are uniform in solid angle. All 64 seed bits contribute to
  independent channels for position, direction, lifetime, size, rotation and
  angular velocity. A live cap uses deterministic drop-new behavior, with
  checked emitted/dropped counters; a large burst does not allocate or loop
  once for every rejected birth.
- A driver can also supply births: per fixed step `(lo, hi]` (and `time == lo` on the first step)
  particles with their own instant, position, velocity and mass, which join the rate and burst
  births in time order with consecutive ids. The mass is kept in the particle (zero for rate and
  burst births); the render size still comes from `size`. The driver must answer a window as a
  pure function of the window, since a seek asks again. A birth outside its window, with nonfinite
  state or a negative mass is a driver error; more births than `maxParticles` leaves room for is
  a limit error and, unlike a rate or burst quota, never a truncation. Frames, replays and checkpoints
  of emitters without driver births are bit-identical to those before this was added.
- Each particle retains the emitter's full birth affine, including scale and
  shear. Local velocity passes through that affine's linear part, then adds
  explicit inherited world velocity. Gravity and driver accelerations act in
  world scene units. Birth Euler rotation is `Rz * Ry * Rx`; the constant spin
  vector is in world degrees/second and is evaluated directly from particle
  age. Later emitter motion does not move already emitted particles.
- Birth scale is uniform on `[1-v, 1+v)`, where `0 <= v < 1`. It multiplies
  instance geometry and the explicitly configured **world-space** collision
  radius. Emitter affine scale affects rendered geometry but does not silently
  turn that radius into an ellipsoid; callers choose a conservative sphere
  radius for an elongated prototype.
- Constant acceleration plus nonnegative linear drag is integrated analytically;
  varying forces use a midpoint sample. Curved flight is split before sphere
  sweeps: for a frozen acceleration `a` and drag `k`, the bound
  `|a-k*v0| * dt^2 / 8` limits deviation from the chord. The positive
  `collision_tolerance` defaults to `0.001` scene units. This bounds the local
  integration segment, not arbitrary unresolved variation of an external force.
- The rigid-collider adapter uses Parry continuous shape queries, including
  translating and rotating shapes, moving-surface velocity, initial penetration
  recovery, restitution and tangential friction. Collision response is one-way;
  particle/particle coupling and debris-driven rigid-body impulses are not
  implied. Dissipative rebounds whose estimated height is below the authored
  collision tolerance settle into supporting contact. For normal rebound speed
  `u`, frozen relative normal acceleration `a_n < 0`, and `0 < bounce < 1`,
  the criterion is `u*u <= -2*a_n*collisionTolerance`. The response removes the
  rebound normal speed and separates by at most that tolerance; tangential
  friction remains unchanged. Elastic and force-free rebounds retain restitution,
  and zero-restitution contacts retain their existing response. This prevents
  finite-time accumulation of shrinking gravity-driven bounces without raising
  the collision-count ceiling. More than 16 contacts that carry a particle forward in time in a step, more than 64 penetration recoveries (contacts at fraction zero, of a particle that starts inside a surface or is pushed into a neighbouring facet, which advance no time) in a step, more than 90 degrees of collider
  rotation in a sweep, unsupported queries and nonconvergence return errors.
- Fixed checkpoints and replay make forward, backward and fractional sampling
  deterministic for a deterministic driver. A failed request preserves the
  published frame; completed fixed steps may remain cached. Live/workspace,
  checkpoint, event and per-seek work budgets are separate. Default budgets are
  10,000 particles, 16,384 events per step, 256 MiB live/workspace, 64 MiB
  checkpoints and 100 million work units; a seek also permits at most one
  million fixed steps. Exhaustion returns an error instead of partial output.
  Caller-owned collider geometry and driver caches require separate budgets;
  the emitter cannot account for arbitrary allocations inside a driver.

These semantics are covered by `crates/sr-sim/tests/particles3d.rs`, including
real thin/rotating collider queries, curved flight whose endpoints miss the
obstacle, fractional-rate quota conservation, seeded mesh area sampling,
world-space spin and scrubbing after checkpoint eviction. They do not yet
establish rendered ejecta quality or UHD particle throughput.

`<particles3D>` has an explicit 3D pose and emitter shape (`point`, `box`,
`sphere`, `mesh`). Reuse rate, lifetime, bursts, seed, force-field references,
appearance curves and timeline conventions from 2D particles, without changing
2D behavior. Add depth, velocityZ/gravityZ and a declared 3D emission direction.
Mesh emission references a mesh asset; mesh particles reference a mesh prototype.

The render shape discriminator is `sphere`, `billboard`, `streak` or `mesh`.
Billboards require a sprite/image asset when textured; mesh requires a mesh
prototype. Width/size and trail length use scene units. Particle positions and
velocities remain in 3D world space after birth. The live-particle cap applies
before allocation, with deterministic drop-new behavior and a reported count.
Collision shapes and fields are evaluated at particle-step time; 2D collisions
must not be accidentally substituted. Render depth, shadowing and motion blur
must follow the particle geometry rather than compositor draw order.

The XML interface is additive in `nodeChoice`. It owns ordinary animation
elements, parent transform constraints and existing `<burst>` children. The
generated Rust model, compiler asset discovery, evaluator simulation selection,
render cache keys and whole-pass motion blur recognize this node.

| Attributes | Contract and defaults |
|---|---|
| `id`, `name`, `parent`, `condition` | Existing node identity and scope semantics; `id` is required |
| `x`, `y`, `z`, `rotation`, `rotationX`, `rotationY`, `scaleX`, `scaleY`, `scaleZ` | Numeric scene-space emitter pose; translations/rotations default 0, scales 1; `z` is depth and never sibling order |
| `start`, `end`, `visible`, `opacity`, `motionBlur` | Node timeline and rendering; defaults 0, absent, true, 1, inherit; hiding its rendering does not move existing particles |
| `emissionStart`, `emissionEnd`, `dt` | Local seconds since node start, independent of its visibility interval; defaults 0, absent, 1/60; group clocks scale the simulation timeline |
| `rate`, `lifetime`, `lifetimeVariance` | Defaults 10 births/s, 2 s, 0 s; rate is sampled at interval start, pose at each birth |
| `emitterShape`, `emitterWidth`, `emitterHeight`, `emitterDepth`, `emitterRadius`, `emitterMesh` | point/box/sphere/mesh, default point; dimensions/radius default 1; mesh requires a mesh asset and is a surface source, area-weighted; its vertices are the imported ones, as for every mesh asset (a file in metres, y up, is turned half a turn about x and is 100 scene units to the metre), placed by the emitter's own transform, so a ring that a file measures in scene units is 100 times larger than measured unless the emitter is scaled by 0.01 |
| `seed`, `maxParticles`, `maxEvents`, `maxWork` | Defaults 0, 10000, 16384, 100000000; upper bounds 2^64−1, 1000000, 1000000, 1000000000 |
| `velocityX/Y/Z`, `directionX/Y/Z`, `speed`, `speedVariance`, `spread` | Local launch velocity; defaults zero velocity, direction (0,−1,0), zero speed/variance and zero full-cone degrees; spread is at most 360 |
| `inheritedVelocityX/Y/Z`, `gravityX/Y/Z`, `drag` | Explicit world velocity added at birth, world acceleration and nonnegative linear drag; all default 0 |
| `rotation0X/Y/Z`, `rotationVarianceX/Y/Z`, `angularVelocityX/Y/Z`, `angularVelocityVarianceX/Y/Z`, `scaleVariance` | Birth rotation, world spin and their seeded symmetric variances; all default 0; scale variance is less than 1 |
| `forceFields`, `useForceFields` | Select existing physics fields by ID; absent list uses all; false disables all; the existing fields' physical-to-scene unit/axis conversion still applies |
| `colliders`, `collisionRadius`, `collisionTolerance`, `bounce`, `friction` | Explicit rigid surface objects, default none; world radius 0.5, chord tolerance 0.001, restitution 0.5, friction 0 |
| `shape`, `mesh`, `sprite`, `segments` | sphere/billboard/streak/mesh, default sphere; mesh requires a triangle mesh asset; a billboard optionally uses an image asset; tessellation defaults to 12, maximum 256 |
| `material`, `castShadow`, `receiveShadow` | Existing surface material reference and shadow flags, both flags default true; no splat prototype substitution |
| `size`, `sizeEnd`, `sizeCurve`, `trail` | Default size 1, end equal to start, linear life interpolation; sphere diameter, billboard width/height, streak width; mesh uses a multiplier on imported scene-unit geometry; `trail` is streak length in scene units, default 1 |
| `color`, `colorEnd`, `colorCurve`, `opacityEnd` | Surface color multiplier (literal or token), default white, same end color, linear and 1; interpolation uses normalized age; size clamps at zero and opacity at [0,1] |
| `maxMemoryMiB`, `checkpointMemoryMiB`, `meshMemoryMiB` | Defaults 256, 64, 128; each at most 4096. The CPU workspace and renderer instance list each check maxMemoryMiB independently; emission mesh and aggregate collider geometry each check meshMemoryMiB independently |

Solver settings and shape selections are static. Pose, rate, size/end size,
color/end color, opacity/end opacity and trail length may animate. Per-life
curves are resolved with the same curve defaults as 2D particles. A textured
billboard uses the image decoder's declared transfer, color space, alpha and
embedded-profile policy; resulting surface maps use the existing RGBA8 texture
representation. Its plane faces the camera. Streaks align their cylinder with
velocity and extend behind the particle; mesh and sphere prototypes retain
the complete birth affine and spin.

Raster passes instance consecutive compatible draws in their existing sorted
order. A storage-buffer indirection preserves each instance's transform, opacity
and shadow-reception flag even when depth sorting changes the object order.
Prototype geometry, material bytes, texture bindings and culling/blending state
must agree; deformed meshes and material boundaries split batches. Shadow views
and the depth/normal prepass also batch compatible prototypes. This requires no
new XML fields. Object records have a 256-byte storage stride; admission checks
the device binding/buffer limits before appending each record, including shadow
records. Exceeding those limits reports a render error. This is a component bound,
not a whole-process memory guarantee.

The regression compares 128 shared-prototype instances against independently
uploaded meshes, with scrambled depths, varying per-instance opacity and
alternating materials. Compatible primary-pass cases use one draw with identical
pixels. CPU preparation still retains one draw record per particle/prototype;
its production scaling remains part of final UHD validation.

Path tracing builds a world-space BVH over surfaces and instance bounds, with
shared object-space BVHs for repeated immutable cached meshes. Each instance
stores its transform, inverse, normal transform and material independently.
Local rays retain world-distance parameters, including reflected and nonuniform
scales. UV scale and separate-UV layout define geometry sharing boundaries;
displaced, deformed, unique, singular or non-affine draws retain expanded
geometry. Prototype bounds include rounding padding. BVH depth is bounded to
keep the fixed traversal stacks safe. Admission counts stored prototype
triangles plus instance records and checks material storage separately. This
requires no XML additions and does not provide a total process memory bound.

The regression compares 64 shared instances with independently expanded meshes
under opaque and lit textured materials, normal maps, opacity, participating
media and coincident placement. Pixel differences stay below 0.003. Shared
geometry fits a 2 MiB binding limit that rejects expansion. The unchanged UHD
impact at 1.5 seconds reports 101,272 physical triangles, 44.977 seconds GPU
frame time, 52.91 seconds wall time and 573,220 KiB peak host RSS on RTX 6000 Ada
Vulkan; the expanded frame used 44.094 seconds GPU time, 55.77 seconds wall time and
584,708 KiB host RSS (single observations on a build before 2026-10-04, commit not
recorded, not repeated). They demonstrate a small host-memory reduction, not a GPU
speedup or sequence-throughput improvement.
Geometry sharing is rebuilt per frame; persistent caching remains unverified.

Scene colliders use primitive surfaces or mesh rest poses, including finite
planes, torus holes, text/path extrusions and carved clay. Static procedural
surfaces are constructed independently of render visibility, conditions and
activation windows; normal per-step frames decide participation. Their retained
geometry and fragment structures share `meshMemoryMiB`. Ordinary collider geometry
and dimensions remain static;
crater-owned surfaces follow the deformation contract below. Rigid and deforming
boundary motion are sampled over canonical particle steps, and hidden explicitly
referenced proxies participate. Ordinary rigid surfaces reject shear or changing
scale within a step. Crater surfaces interpolate transformed world-space vertices.
Invalid geometry and query failures produce diagnostics rather than bounding-box
fallbacks. Contacts affect particles only; they do not push terrain or fragments.

Rules `P3D1`–`P3D6` cover the 1.3 version gate, typed asset/shape references,
variance/direction/emission ranges, static solver configuration, distinct typed
collider references (at most 4096), and static collider geometry. `P3D7` to `P3D10` cover a
`burst` with `crater` (the instant, `repeat` and `interval` are not given, the crater grows from an
impact, `angle` and `angleSpread` belong to such a burst and stay within 0 to 90 degrees together) and
`P3D11` that `gas` names an object3D with a native pyro volume. Their independent XSD/Schematron
fixtures are `valid/particles3d.scene.xml`, `valid/ejecta-crater.scene.xml` and `invalid/p3d1`
through `p3d11-gas.scene.xml`. Budgets, finite transformed values, importer failures and device
precision limits remain runtime checks with explicit diagnostics.

Particles dragged by smoke (`particles3D@gas`). A reference to the object3D whose native pyro volume drags
the particles of an emitter; absent is the behaviour there always was. The integrator already solves
`p'' = a - k v` in closed form with `k` the emitter's `drag`, and the driver adds `k u` to `a`, `u` the gas's
velocity at the particle's place and time, so a particle obeys `p'' = k (u - v)` plus gravity and fields:
dragged by the gas inside the volume's domain and by still air outside it with the same coefficient (the
gas fades linearly to rest across one cell outside the domain, so what the particle feels has no jump at the
boundary, for an open volume and for a closed one), and not at all with `drag` zero. The position is taken to
the volume's axes by the inverse of its world matrix at that instant and the velocity back by its linear part. In
time the gas is the linear interpolation of the two smoke steps around the instant (a frame between two canonical steps takes the particle on by a partial segment of its step, which asks a handful of times per
particle (two to five, counted in test `particles_gas`, commit 0c9df98, 2026-10-04) from the step's own context: no smoke step is simulated and no field fetched, so
motion-blur samples add queries and no simulation). The particles run before the smoke in the
frame and ask it, in order, for the steps they need, so that the smoke is simulated once for each of its steps (30 steps
in 3 s for an impact scene whose ejecta are dragged by a smoke and fall into an ocean, where 69 were simulated when
the smoke ran first, test `gas_and_splash`, commit 8c3ed9b, 2026-10-06) and the frame's own volume finds the step
it is at among the states the particles kept; going back restores the smoke's own checkpoint as any seek of it does, and any order of frames, a fresh
evaluator and a smoke that kept no checkpoint but the first give the same bits. The velocity fields of the steps in use
(the window of a particle step, `ceil(dt_particles / dt_smoke) + 3` of them) and the two states the smoke keeps for them (the frame's own volume needs the step the readers started at, and the
timeline is a step past it: 1.2 MB each at 32 cells, about 110 MB each for 128 x 104 x 128) are charged to the
volume's `maxMemoryMiB` (three face arrays of 8 bytes: 5.1 MB for the 64 x 52 x 64 plume, 41 MB for 128 x 104 x 128), and a
smoke that fails, or a window that does not fit, fails the particles with its message, never as still air. P3D11:
the target must hold a native pyro volume. The context of a canonical step (the volume's matrix and the
smoke's clock at its start, the smoke steps that cover it) is built once per step, and the driver is asked
one particle at a time, so the coupling is serial. Cost, test `particles_gas` (release, one core, 20 000 particles
over 25 canonical steps of 0.1 s in a 32-cell smoke; commit 0c9df98, 2026-10-04, load of the machine not
recorded): 0.77 s more than naming the gas with no drag, 1.5 microseconds per particle step (about five gas
queries of 300 ns, since the integrator asks at several points of a step); 100 000 particles over a 6 s film at
24 steps a second extrapolate to about 22 s, not measured. The coupling is linear in the relative velocity and one
way: the particles do not push the gas. For 0.34 m rocks of 2100 kg/m3 in air at 50 m/s relative the quadratic
drag is a rate of about 0.03 per second (computed from the drag law, not measured), so the coupling moves
dust-sized particles (or an authored `drag`) and not the ejecta of the impact scenes.

Ejecta falling into an ocean (`ocean@splash`). An ocean lists the emitters whose particles fall into it
(OCN13: each throws out the ejecta of a crater, so its particles have a mass). An emitter belongs to one ocean,
which the evaluator checks when the scene is evaluated, not the rule. The particle solver takes a plane of water (a point, the normal out of the water, the rectangle it
covers): a particle whose centre crosses it downward inside the rectangle, before any contact it would make later
in the segment, is removed there, and the driver is told which (instant, place, velocity, mass), for every fixed
step, an empty list too, and again the same when a seek replays the step. The evaluator takes the plane from
the ocean at its pose when the emitter starts (a function of the document; the ocean must have no scale and both
must be on the composition clock), sums what fell by cell and by canonical step of the ocean (the volume is the
mass over the density of the target of the crater that threw them, the momentum is the mass times the horizontal
velocity in the ocean's axes over the density of the water, `ocean@density`, in scene units) in the order of the particles' ids,
and writes it once per (emitter, fixed step) in a log of at most 16 MiB: a replay of a step must reproduce its
entries to the bit or it is an error. The ocean reads a canonical step by the entries of the fixed steps that
overlap it, and a step the particles have not reached is an error that names it, never an empty splash. At the
sample that closes each canonical step the ocean passes the solver the entries of that step, and the solver takes
the volume out of the cell and gives it equally to its eight neighbours (at most 90 % of the cell's water) and
adds the momentum to the cell when it keeps a depth. A particle that fell in is gone: it does not sink, its
vertical momentum and its energy are not given to the water, and a particle that has no mass is refused by the
rule.

Scheduling. The ocean at a time is given the splash of the step that holds the time, which ends after it, so
the emitter is computed until the end of that step before a frame is taken (an ocean that runs after the
particles of its frame), or, for an ocean that loads rigid bodies and so runs before them and steps ahead of
them, the ocean asks for the emitters when a canonical step needs them: they are made known to the log and
computed to the end of that step, a step at a time with the ocean, each needing only the loads the ocean has
already written. The particle solver reads the rigid world, so a rigid world that answers with a problem (a
load from a step of the water that has not been computed) fails the fixed step of the particles that asked,
before the step is kept. Cost of the scheduling, whole-frame evaluation of 144 frames at 24 fps, three
alternated runs each, commit 3078153, 2026-10-05, load1 6 to 13: on the authored sea (no particles) 0.058 to 0.062
s per frame, peak 109 to 112 MiB (the comparison with the binary before the scheduling is in the ledger); with 3000 ejecta, 11 of which fall in (4 m of
water), 0.106 to 0.119 s per frame without `splash` and 0.109 to 0.115 with it; with 21 000 ejecta of which 9131
fall in (1 m of water) 0.217 to 0.230 s without and 0.176 to 0.192 with (the particles that fell in are not
simulated any more), peak 133 and 132 MiB. The cost of the aggregation, the per-step read and the pull is under
the saving in that last case, about 0.04 s per frame, and was not isolated. With the authored 20 m of water at
most 34 of 3000 ejecta of the biggest rock of the sweeps reach the surface (they are born on the seabed).

Measured on the impact scene's rock at 60 degrees with an ocean 1 m under the ground (400 m, 4 m cells, no
ground collider for the ejecta), tests `ejecta_splash`, commit 613e87f, 2026-10-05 (the results do not depend on
the load): all 4000 particles fall in by 7 s and the log holds 80.7815 m3, the volume of the law's crater share
to the last digit; by speed 60, 100 and 150 m/s 37.2, 80.8 and 148.2 m3; the horizontal momentum per volume
along the rock's travel grows from 90 to 60 to 30 degrees. In a closed basin the water volume is the one it had to
1e-9 of itself (the 80.8 m3 are taken from the cells and shared with their neighbours, the surface moves 3 mm),
the horizontal momentum in the ocean is the momentum in the log to 1e-6 while the waves have not reached the
walls, an ocean driven directly by the log, sparse or with every cell in every step, is the same to the bit, and
a step given one late is another ocean. On a beach (the rock lands 25 m from the shore on a ground that ends
there, the ejecta collide with the ground, the ocean begins at the shore; commit 4d91c1e, 2026-10-05) 3879 of
4000 stay on the ground and 121 fall in (2.44 m3) from 4.5 s on; the ocean has the momentum it was given in the
steps that follow the first of them, before the shore wall of the closed basin turns the waves back.

### Ocean surfaces and impulses

**Implementation status:** The CPU solver, version-1.3 `<ocean>` node, owned
waves and impulses, image/mesh bathymetry, local-clock replay, native surface
meshes, reflective/transmissive rendering, and native foam/spray tracers are
implemented, and so are the objects that act on the water and what acts back
(`colliders` that move the bed or occupy the water, the cavity of a body's entry,
the depth response `bedResponse`, the form drag of bodies, the per-body samples and
the pressure credited to each body, buoyancy and the full coupling with the rigid
world, and the splash of falling ejecta: rules OCN6 to OCN13). The completed UHD
film remains unverified.

The domain lies in local x/z, centred on the node's origin. Scene y increases
downward. Poses, parent transforms, opacity and visibility use the native 3D
rendering path. Animating the node moves the entire domain; it does not inject
inertial forces into the local solver. Group clocks and the node's `start` define
local simulation time. Hiding a node by condition or visibility does not reset
its water or suppress authored local-time impulses: a later visible sample
replays from local time zero.

| Ocean attribute | Default | Contract |
|---|---|---|
| `width`, `depth` | 64, 64 | Positive scene lengths; integral multiples of `cellSize` |
| `cellSize` | 1 | Square cell spacing; at most 4,000,000 cells |
| `waterLevel` | 0 | Initial local scene-y surface ordinate |
| `bottomDepth` | 10 | Nonnegative fallback flat depth and reference depth for default swell speed |
| `gravity`, `damping` | 9.81, 0 | Positive acceleration magnitude and nonnegative exponential drag |
| `dt`, `dryTolerance` | 1/60, 1e-10 | Canonical time interval (at least 1e-6 s) and positive dry-depth threshold |
| `initialVelocityX`, `initialVelocityZ` | 0, 0 | Horizontal velocity assigned to initially wet cells |
| `boundary` | closed | `closed`, `open` or `periodic`; semantics below |
| `seed` | 0 | Full unsigned 64-bit seed for unspecified wave phases |
| `bathymetry` | absent | Scoped image or mesh asset reference |
| `bathymetryEncoding` | red | `red`, `terrarium` or `mapbox`; packed encodings require an image |
| `order` | 1 | `1` or `2`; `2` selects the second-order scheme in the numerical contract below. Order 1 reproduces earlier results bit for bit |
| `bathymetryScale`, `bathymetryOffset` | 1, 0 | Multiply and then offset decoded scene-y bed ordinates |
| `material` | absent | Material reference; default is white, roughness .05, transmission 1, IOR 1.333, double-sided |
| `colliders` | absent | Up to 4096 distinct `object3D` ids that move the bed or occupy the water (OCN6, OCN7); see the numerical contract. Absent leaves the bed fixed |
| `bedResponse` | `depthFiltered` | `depthFiltered` or `hydrostatic`: how the surface answers what `colliders` do to the water; no effect without `colliders` |
| `bodyCoupling` | `none` | `none`, `buoyancy` or `full`: what the water does to the rigid bodies in `colliders` (OCN8) |
| `density` | 1000 | Density of the water in kg/m3: the weight of what a body displaces, the force the water gives back, the momentum the ejecta of `splash` bring (divided by it) and the target of the cavity of `waterImpulse` |
| `bodyDrag` | 1.0 | Form-drag coefficient of a body in the water, vertical (with `bodyCoupling`) and horizontal (`depthFiltered`) (OCN9) |
| `splash` | absent | Emitters of crater ejecta whose particles fall into this ocean (OCN13) |
| `maxMemoryMiB`, `checkpointMemoryMiB` | 256, 64 | Solver workspace and separate checkpoint ceiling; zero checkpoints disables retention |
| `meshMemoryMiB`, `surfaceMemoryMiB` | 128, 128 | Bathymetry decoding/sampling and generated surface geometry ceilings |
| `maxWork` | 100000000 | Work allowance per solver seek, at most 1,000,000,000,000; also bounds bathymetry sampling and swell evaluation separately |

Memory attributes are at most 4096 MiB; all except checkpoint memory are positive.
Admission budgets cover the named operation, not aggregate GPU use or copies
retained by external API callers. Mesh importer internal dependency allocations
still need hardening; the flattened geometry is checked against its budget.

The generated surface is charged 256 bytes per vertex plus 48 bytes per cell
against `surfaceMemoryMiB`: 721 × 721 vertices of a 720 × 720 cell ocean cost 151
MiB, above the default of 128, so the target-resolution example declares 256. That
example also declares `maxMemoryMiB="512"` (a second-order solver on 518,400 cells
is charged 400 bytes per cell, 198 MiB) and `checkpointMemoryMiB="256"` (each
checkpoint holds about 12.4 MB).

`<wave>` has `wavelength=16`, `amplitude=1`, `direction=0` (degrees from +x toward
+z), optional phase in degrees and optional nonnegative speed. Missing phase is
seeded independently by wave index. Missing speed is `sqrt(gravity*bottomDepth)`,
including when a bathymetry asset is present. At most 64 waves are accepted and
wavelength must be at least twice the cell spacing. Waves are analytic cinematic
swell, applied to a copy of the simulated water. Their height contribution is
`a*cos(2*pi*(directionUnit·position-speed*time)/wavelength + phase)`, with each
wave's amplitude tapered to at most half the local water depth. Subtract the
wet-cell mean, clamp negative depths and normalize to preserve the wet-cell
water volume. Shore clipping and this normalization can alter authored amplitude.
Horizontal wave velocity adds `speed*heightContribution/depth` along propagation.
It is never fed back into the shallow-water solver, avoiding repeated injection.

`<waterImpulse>` has `time=0`, `x=0`, `z=0`, `radius=1`, `amplitude=1`,
`velocityX=0`, `velocityZ=0`, and `type=displace` (`displace` or `add-water`).
Its local-time and conservation semantics are specified below. Negative amplitude
is permitted only for displacement. Wave and impulse children are static owned
data, not independently animated scene nodes.

Water entry. A `<waterImpulse source="body"/>` is the cavity that a body makes of the water it
enters, and nothing is authored about when, where or how big: `time`, `x`, `z`, `radius`,
`amplitude`, `velocityX`, `velocityZ` and `type` may not be given (OCN10), the body must be a
dynamic rigid body without a crater that the ocean lists in `colliders` (OCN11) and at most one
impulse names it (OCN12). Without `source` no cavity is made. The entry is the first canonical step in which the
lowest point of the body goes from above the rest level (`waterLevel`) to at or below it while its
centre moves down, found from the body's poses at the two ends of each step, with the instant
placed by linear interpolation of the lowest point inside the step; a body that starts in the water,
that never reaches it or that enters outside the ocean makes nothing, and a body that goes down
through the level again makes no second cavity. The scaling law is the transient crater of
Holsapple (1993) in the material `water` (the engine's `cratering` law, whose constants are
those of the calculator note), with the body's mass and density (its mass over the volume of its
closed surface), the speed of its centre downward over the step (the component normal to the
surface) and the ocean's gravity, in metres through `physics@pixelsPerMeter`. The law gives the volume `V` and
the radius `R` of the cavity. The solver's `cavity` impulse is applied at the instant of entry, centred
under the body's centre: it empties a central disc of radius `R = sqrt(3 V / (pi d))`, `d` the law's depth (the central kernel
`(1 - 4 (r/a)^2)^2`, with the impulse radius `a = 2R`, whose volume per unit of peak depth removed is
`pi a^2 / 12`, so the peak removal is `12 V / (pi a^2)`, equal to `d`) and puts the water into the ring from `R` to `2R`
(`sin^2` weight), conserving all of it. It does not form at once: the law's formation time is
`T = 0.8 sqrt(V^(1/3) / g)` and the cavity grows over it, in one part for every canonical step, with the
share of the whole that `p(t) = 3 s^2 - 2 s^3` (`s` the time since the entry over `T`) gives in the step, each
part applied in the middle of the stretch of the step in which the cavity is forming (so the parts of all the
steps are the whole). The shape of the growth is the engine's choice; the law gives only `T`. A disc or a
ring narrower than four cells across (`a` under eight cells) is widened to `a = 8` cells, keeping `V`, so
that the water goes to a smooth ring that the grid resolves.

How the limit by the water layer is applied. The impulse removes from each column in proportion to
the column's water, and takes the wanted volume or `0.9` of the water the central disc holds,
whichever is less (a part of share `q`, after a fraction `b` of the cavity has formed, takes at most
`0.9 q / (1 - 0.9 rho b)` of the water the disc holds then, `rho` being how much of what a part takes the
disc's weighted water falls by, so that if the water stayed put the parts together would take what one whole
cavity does, 90% of what the disc held when it began; the moving water makes that hold to a few per cent):
a column therefore loses
at most 90% of its depth and the cavity never exposes the bed, and a layer too shallow for the wish gives a
shallower cavity, never an error (a negative
`displace` is an error in that case; this is a separate kind). A body that reaches the bed
excavates the bed's crater (`crater@source`) instead.

The size of the disc and the role of the limit. The disc that is emptied has the bowl profile
`(1 - (r/R)^2)^2`, whose volume per unit of depth at its centre is `pi R^2 / 3`, with `R = sqrt(3 V / (pi d))`
for the law's volume `V` and depth `d`: the peak of the removal is the law's depth and the profile holds the
law's volume (for water `R` is about 1.4 times the law's own radius). A disc with the law's own radius would
have a peak of about twice the law's depth, which asks the water for more than a layer of a few times the
depth holds; the 90% limit is therefore a protection for shallow water and does not shape the results in
deep water. Figures, with the cell size and the commit, are in the ledger entries 'The cavity of a water
entry has the law's depth, not twice it' and 'Corrections to the figures of the ocean entries' (acceptance
sweeps on 3 m cells: none of the parts of the three cavities of 60000, 90478 and 270000 kg limited; the
numbers of the first profile are in the ledger).

What the cavity gives. Two waves are called a far wave in this document and the crest is a third thing; they are
told apart here once. The cavity's far wave is the largest difference, in the ring 20 to 40 m from the entry point, between the
water with the cavity and without it, for a body of 2 m radius in 100 m of water on a 128 x 128 ocean of 2 m cells
(tests water_entry). It grows with the speed (0.0050, 0.0121 and 0.0211 for 10, 20 and 30 m/s at 2000 kg) and with the
mass (0.0054, 0.0121 and 0.0277 for 500, 2000 and 8000 kg), as the law's volume does (commit 187deaf, 2026-10-05; the disc for these bodies is the minimum of eight cells, so
the disc of the law's depth leaves it unchanged; the values before the cavity formed over time are in the ledger). The sea's far
wave is the highest the water stands above its rest level in the ring 20 to 40 m from where the rock enters of the
impact-ocean scene (the acceptance sweeps in the conformance section). The crest is the highest surface anywhere. The model is hydrostatic: no jet, no crown of spray, no air. The law is that of a
crater in water, not a validated model of water entry.

Depth response of the surface (`ocean@bedResponse`). A shallow-water solver lifts a whole column by
what raises its bed, which is right for a displacement much wider than the water is deep and wrong for a
compact one in deep water: with the hydrostatic response a sphere of 2 m radius resting on the bed of 20 m of
water raises the surface by 3.00 m the instant it appears, where linear water-wave theory gives 0.024 m (about
125 times less; commit 76618aa, 2026-10-04, tests ocean_depth_filter and ocean_lift). The
ocean therefore has `bedResponse`, `depthFiltered` (the default) or `hydrostatic`. `hydrostatic` is the
long-wave response exactly as it was (the occupancy of a body lifts the column by its thickness, and the
water relaxes toward the body's velocity in the columns it occupies), kept for comparison and as the mode
of the tests of the first version; `depthFiltered` attenuates what craters and bodies do to the water by
its depth. Without `colliders` the attribute has no effect.

The filter. In linear potential flow an instantaneous displacement `zeta` of the bed of water of depth `h`
raises the free surface by `zeta(k) / cosh(k h)` in Fourier space: the filter that tsunami models apply to a
seafloor displacement to start a simulation (Kajiura 1963, "The leading wave of a tsunami", Bull. Earthq.
Res. Inst. Univ. Tokyo 41:535-571; read here only as quoted by the tsunami literature, for example the
NHESS 24:2773 (2024) article on tsunami initial conditions from seafloor displacement, Geist and Dmowska
(1999, Pure Appl. Geophys. 154:485) and the comparison of methods for coupled earthquake and tsunami
modelling (Geophys. J. Int. 234:404, 2023), and not the original paper). The generalisation to a displaced volume at height `z0` above the bed, which the
engine needs for bodies, is the engine's own derivation, not a citation: for a unit volume source at height
`z0` in water of depth `h`, with a rigid bed and, for an impulse, zero pressure at the surface, the potential
of each wavenumber is `A cosh(k z)` below the source and `B sinh(k (h - z))` above it; continuity at the
source and a jump of `-q` in the vertical derivative give `B = q cosh(k z0) / (k cosh(k h))`, so the vertical
velocity of the surface, and with it the response, is `cosh(k z0) / cosh(k h)`. The two limits that check it:
`z0 = 0` is Kajiura's `1/cosh(k h)`, and `z0 = h` (a source at the surface) is 1, no attenuation.

How it is applied. For a crater, the displacement of the bed (the deformed surface less where it lay at time
zero) is filtered as a source at `z0 = 0` over the depth, `h`, that the bed was moved under, weighted by how
much it moved. For a body, one kernel per body: the thickness of the body in each column (the part of it
between the rest level and the bed) is filtered with `z0` the height above the bed of the middle of the
displaced volume (the volume-weighted mean of the middle of its part in every column) and `h` the rest depth
under it (weighted by thickness). The result is the lift of the bed, in place of the thickness; the thickness
itself still marks where the body is (owners, the samples of the water around each body). The transform is a
radix-2 FFT over a window of the footprint and four depths on each side, where the kernel has fallen to 0.2%
(`exp(-pi r / (2 h))`), at most 1024 cells wide; a displacement wider than a window is left as it is, its
response being long-wave there. What is displaced is what is lifted: the part that falls on wet columns of
the domain is renormalised to the whole, the positive and negative parts of a crater's displacement each
keeping their total, and the lift is cut to the water the column holds, so the volume is kept to
rounding except where a cut-off column loses some. A lake at rest is untouched (nothing is displaced, and
the result is identical to the hydrostatic one); a body at rest sits on a static bed, over which the
scheme is well balanced; the filtered bed is a function of the time and of the scene, interpolated in time
over a canonical step as before, nothing new is stored in a checkpoint, a replay is identical and the
result is the same with 1, 2 and 8 threads. On a 720 x 720 ocean a step took 0.14 s with one body and
0.14 to 0.16 s with ten, filtered or not, and a crater 0.123 s and 0.128 s (commit 76618aa, 2026-10-04, load1 8
to 12; the filter is within the noise of the measurement; ledger entry 'Depth response of the ocean surface').

Momentum, in `depthFiltered`. Giving the water the velocity of a body in its columns (the relaxation above)
is the same long-wave hypothesis: it spreads the momentum of the displaced volume over the whole depth at
once, a bow wave of a few metres where a small body deep in the water makes none at the surface. In this
mode the water receives, per canonical step, the form drag of the body through it,
`(1/2) rho C_d A |U - u| (U - u)` per unit water density, as a push: `U` is the body's horizontal velocity
(thickness-weighted over its columns), `u` the mean horizontal velocity of the water under its footprint at
the end of the last step (the sample of the water around the body), `A` the area its part below the rest
level (and above the deepest bed under it) presents to a flow along `U - u`, from the surface of the body
itself (half the projected area of the cut triangles), and `C_d` is `ocean@bodyDrag` (default 1.0, an engine
parameter and not from the impact literature, also the coefficient of the exchange with the water, as it
already was of the vertical motion). The push is spread over the same columns and shares as the body's lift,
a stated approximation: the transfer function of a horizontal impulse at a height was not derived, and the
vertical-source kernel stands in for it. The solver applies it evenly over the substeps, column by column,
never past the body's velocity (a column's momentum goes toward `U` times its depth and no further), and
credits the body with exactly what it applied, so what the water receives is what each body is credited with
and the reaction of `bodyCoupling="full"` is consistent. A sphere through still water gives its first step
`(1/2) C_d (pi a^2) U^2` of impulse to within 1% (104.05 against 104.72 at 2 m radius and 20 m/s; commit
4308a71, test in the evaluator), the order of the drag of a sphere (`C_d` 1 against about 0.47 for a real
sphere); the relaxation of the hydrostatic mode gives about four times that by its formula (`rho A U^2` of
momentum per step against `(1/2) C_d rho A U^2`; computed, not measured); the water is pushed less as it comes
to move with the body.

Pressure of the water on a body (`BodySample::pressure`). The momentum a body is credited with (`impulse`) is
what the solver applied to the water; across a step in the bed the scheme also adds momentum to the water, the source
term of the hydrostatic reconstruction, `1/2 g [(r^2 - hr^2) - (l^2 - hl^2)]` per interface (`l` and `r` the depths on
the two sides, `hl` and `hr` the reconstructed ones), which belongs to no one and is, on the body that raises the bed,
the wave drag. The driver gives the solver what each body raises the bed by, column by column (`Forcing::lifts`,
sparse and per body, in both responses), and the flux sweeps accumulate that term itself at every interface between
columns of different bed, to the owner of the interface (that of the column lifted more of the two, the lowest owner on
a tie; a column with several owners counts for the one that lifts it most), the wrap face of a periodic basin included, for the stages of the step weighted as the scheme weights them. At the end of every
canonical step each body's sample carries `pressure`, the sum over the step. The sum is reduced in the order of the
rows and then of the bands, so it does not depend on the number of threads, and it is part of the state and of the
checkpoint, so a replay from a checkpoint, from zero or without a checkpoint gives the same bits. Without lifts
nothing is accumulated and the water is bit for bit what it was. The credit is the scheme's term and not the
continuous `-g h grad(raise)`: it includes the reconstruction and its numerical diffusion, and a bed variation that
belongs to no owner (a crater) is not credited. In this phase `pressure` is credited and recorded and is not applied
to the body: the body, in `bodyCoupling="full"`, gets back `impulse` and nothing else.
The balance of the momentum of the water along x, in a closed basin of 160 m with cells of 2 m and 10 m of water,
dt 0.05, where a ball of 2 m radius and 16755.16 kg at 3 m/s crosses (the ball of the coupled-ocean tests,
`bodyCoupling="full"`), at 1, 2 and 3 s: the water has what the bodies pushed into it and what was credited to them,
to what the walls give. The residual `(water - push - credit)` over the water of the step is 0.0000% (below 5e-5%) at
all three times in the hydrostatic response in both orders, and in the filtered one 0.0150, 0.0381 and 0.0942% (first
order) and 0.0135, 0.0351 and 0.0663% (second order); without the credit the same residual is 3.8, 3.1 and 11.0%
(hydrostatic, first order), 4.3, 4.1 and 9.1% (second), 10.7, 12.3 and 14.6% (filtered, first) and 10.6, 9.3 and
10.1% (second) (commit d685256, 2026-10-05, test the_water_has_what_the_bodies_pushed_into_it_and_the_bed_source_credited_to_them).
On a periodic basin of 64 x 64 cells 6 m deep, in which a Gaussian mound of 1 m and 3 m width moves along x at 4 and
10 m/s for 10 steps and the total momentum of the water changes only by this term, the residual is under 1e-9 of the
momentum (the bound of the test; about 1e-13 measured on a still mound) (same commit, test ocean_pressure). The earlier form of the credit, an estimate at
the end of the step by the central difference of the continuous term, was 49 to 65% of the term (6.3 and 6.2% off
in first order and 0.7 and 2.3% in second on the mound, commit fdfb0b0) and is replaced. The cost of the credit at
720 x 720 cells, with the code of the previous commit against this one, medians of five rounds: with no owners, with owners
and with owners and lifts the ratio new over old is 0.922, 0.962 and 0.956 (first order) and 0.995, 0.988 and 1.006
(second order), within the noise of a machine with load1 between 5.6 and 11.5 and under 2% where it is not.

Against an exact solution (test `ocean_reference` of `sr-sim`). The only reference the wave of the ocean has outside the code is the
linear shallow-water wave of a Gaussian hump on still water, which has a closed form: with `g = 10`, depth 10 m, `sigma = 8` m and an
amplitude `A` of 1e-3 of the depth, the surface at 4 s is `A` times the integral of `s exp(-s^2/2) J0(s r/sigma) cos(c s t/sigma)` over `s`,
`c = sqrt(g h)`, evaluated in the test by quadrature. In a closed basin of 160 m the largest error of the surface over the cells is, as a
fraction of `A`, 6.9e-2, 4.4e-2 and 2.6e-2 in first order and 1.0e-2, 3.1e-3 and 9.3e-4 in second for 4, 8 and 16 cells to `sigma`, which is
the observed order 0.70 in first order and 1.71 in second over the two halvings, and the basin keeps its water to 3.3e-14 (commit 166a7b3,
2026-10-06; the results are deterministic; the test takes 15.8 s in release at a load of about 15). At 32 cells to `sigma` the second-order error
is 3.9e-4 (order 1.27 from the step before), near the floor of a linear reference for a hump of 1e-3 of the depth, and was measured in a
disposable program and not in the test. This checks the scheme's dissipation and order against an answer from outside the code; it does
not check the wave of an impact, whose cavity, depth response and dispersion are not in it.

Where the shallow-water wave stops being the wave of water (a declared limit). The solver propagates what it is given without
dispersion, and real water at the depth `h` has `omega^2 = g k tanh(k h)`; a bed that rises is, in the linear theory of the surface at
rest, attenuated by `1 / cosh(k h)` (Kajiura 1963; Hammack 1973; the problem of an initial surface hump is that of Cauchy and Poisson, solved for
an axisymmetric hump by Kranzer and Keller 1959: all cited from memory and not read). The exact linear answer for the hump of the paragraph
above with `g = 10`, `h = 10` m is the same integral with `cos(omega(k) t)` in place of `cos(c k t)`, and it was evaluated by a disposable program
(2026-10-06, not in the repository; deterministic): for a hump of `sigma = 40` m (`sigma / h = 4`) at 20 s the leading crest is `0.1516 A` at
`r = 214.5` m against `0.1568 A` at `220.5` m for the shallow-water wave, 3 % lower; for `sigma = 16` m (`sigma / h = 1.6`) at 8 s the largest
excursion is a trough of `-0.1714 A` at `41.5` m where the shallow-water wave has a crest of `0.1568 A` at `88` m; for `sigma = 8` m
(`sigma / h = 0.8`) it is a trough of `-0.2374 A` at `14` m at 4 s (against the crest at `44` m) and of `-0.1366 A` at `42.5` m at 8 s (against `0.1148 A` at
`84` m). The same comparison for the surface response of a bed uplift (`1 / cosh(k h)`): the shallow-water peak `0.1467 A` at `91` m against a dispersive
trough of `-0.1535 A` at `34` m for `sigma = 20` m. A hump whose width is a depth or less is therefore not a wave that the solver
gets slightly wrong but one it does not have: its exact linear form is a dispersed train. The cavity of the 90 478 kg rock in 20 m of water has
a radius of the order of the depth (an estimate, not measured here), so the far-wave heights of the sea sweeps below are properties of this
model and are not predictions of what water would do; no figure of this section has been compared with any measurement of a real impact,
and none is claimed to be.

What a sphere crossing deep water makes. A sphere of 2 m radius at 10 m below the surface of 20 m of water,
crossing at 20 and 50 m/s, raises the highest surface by 8.69 and 5.52 m in the hydrostatic mode and by 0.177 and
0.198 m in the filtered one (commit 4308a71, 2026-10-04, test ocean_depth_filter). On the authored impact-ocean
scene (1.5 m cells, the rock of 90478 kg at 100 m/s and 60 degrees, `bodyCoupling="full"`, 2.5 s into the film) the
highest surface above the rest level is 0.81 m at (32.2, -3.8) with the cavity and the filtered response, 1.41 m at
(21.8, -0.8) with the cavity and the hydrostatic one, 0.20 m at (18.8, -0.8) without the cavity and filtered and
1.24 m at (18.8, -8.2) without it and hydrostatic (commit fa63e5d, 2026-10-05, load1 about 10; ledger entry
'Corrections to the figures of the ocean entries'). The cavity is a displacement at the surface and is not
filtered. A crater 40 m across in 6 m of water makes a trough 5% below its hydrostatic one, and one 6 m across in
40 m of water a trough 6.7 times smaller (test ocean_depth_filter).

Limits. The filter is the response to an impulse; what comes after, in a shallow-water solver, is not
dispersive (short waves travel at the speed of long ones). For a body that moves it is applied at each
canonical step, an approximation and not the solution for a moving source, which was not computed (the
stationary sphere of the reference matches linear theory; the moving ones are checked only to be far below
the hydrostatic crest). The momentum is a form drag with a single coefficient, with no added mass and no
wake; the kernel for the momentum is the one of the volume. The cavity of a water entry is not filtered.
Regions wider than 1024 cells are not filtered. The tests that assert long-wave numbers set
`bedResponse="hydrostatic"`.

Splash (ocean side; the particles' side, the plane and the log are in the section on three-dimensional particles).
The driver can give the water what fell into it in a canonical step as a sparse
list of cells (`Forcing::splash`: cell, volume of solid, horizontal momentum per unit density), sorted, no
cell twice, applied when the step reaches its end, after the impulses due then, to the water itself and not
through the lift of any bed. Of each cell's water the depth `volume / cell area` goes, at most 90% of what the
cell holds (the rest of the volume is dropped, not an error), and is given, all of it, in equal parts to the
neighbours the cell has among the eight around it (fewer at an edge or a corner); the momentum is added to the
cell's own unless the cell is left with less than `dryTolerance` of water, when it is dropped. The cost is the
entries, not the cells. Water is conserved except for what is dropped, and momentum except for what is dropped
(commit 3d3df6c, tests ocean_splash; the solver does not report what it dropped).

Bathymetry images map the full raster to the domain, using bilinear samples at
cell centres and clamped edges. Red encoding reads normalized numeric red values
without color-space or transfer conversion. Packed RGB8 encodings decode upward
elevation then negate it: Terrarium is `-(R*256+G+B/256-32768)`; Mapbox is
`-(-10000+(R*65536+G*256+B)*0.1)`. Scale and offset are applied afterward. Mesh
bathymetry samples the topmost transformed local-y intersection at every cell
centre; uncovered cells are errors. Images must resolve locally, encoded input
is capped at 128 MiB and the configured budget, and header dimensions are checked
before decoding. The numerical reader uses the image's primary raster; assets declaring a layer
selection are rejected by OCN2 instead of silently using a different raster. Imported meshes use their rest pose.

Surface corners average adjacent wet-cell surface ordinates, with upward normals,
UVs and tangents. Dry cells emit no triangles. The shoreline therefore has cell
resolution and the mesh cannot represent overturning water. Device buffer limits
are checked before upload. Native ocean meshes participate in shared depth,
materials, lights, path tracing and shutter samples. The rules, each in the Schematron and
in the Rust validator: OCN1 (version), OCN2 (reference kinds of material, bathymetry and
layers), OCN3 (the domain is a whole number of cells, at most 4,000,000, with finite settings and a `dt` of at least 1e-6), OCN4 (static solver configuration),
OCN5 (one whitewater source, ordered window, material references), OCN6 (what `colliders` may
name: at most 4096 distinct objects, a plane or mesh with a crater or a closed body without
one), OCN7 (collider geometry is static), OCN8 (`bodyCoupling` needs `colliders`), OCN9
(`bodyDrag` belongs to an ocean with `colliders`), OCN10 to OCN12 (a `waterImpulse` with
`source`: no derived attribute given, a dynamic rigid body without a crater listed in
`colliders`, one impulse per body) OCN13 (`splash` names emitters of crater ejecta) and OCN14 (a foam coverage radius of at most 64 cells; it is checked in the particles mode too, where the radius is ignored, so that a document does not become invalid by changing the mode).
Only pose and opacity are animatable; the ocean corpus fixtures are checked
against the independent XSD/Schematron oracle.

#### Foam and spray

An ocean may own one static `<whitewater>` child. This is a secondary tracer
simulation: it samples simulated water plus procedural swell and never feeds
water mass or momentum back into the primary solver. Separate source, particle
lifecycle and rendering stages also appear in [SideFX's whitewater
workflow](https://www.sidefx.com/docs/houdini/fluid/sopwhitewater.html).
The formulas below are this engine's cinematic controls, not an implementation
of Houdini's FLIP-based emission model or a validated impact-water model.

| Whitewater attribute | Default | Contract |
|---|---|---|
| `emissionRate` | 10 | Expected particles per cell per second per unit of excess activity |
| `threshold` | .5 | Nonnegative activity threshold |
| `start`, `end` | 0, absent | Local emission window `[start,end)`; end must not precede start |
| `lifetime` | 3 | Positive seconds from birth to removal, unchanged when spray becomes foam |
| `sprayFraction` | .4 | Probability that a new tracer starts as spray; remainder starts as foam |
| `launchSpeed` | 3 | Nonnegative upward initial spray speed |
| `drag` | .1 | Nonnegative spray velocity decay rate |
| `radius` | .05 | Positive initial geometry radius, shrinking linearly over lifetime |
| `seed` | 0 | Independent full unsigned 64-bit seed |
| `maxParticles` | 10000 | Live count ceiling, at most 1,000,000; overflow is an error |
| `maxMemoryMiB` | 64 | Independent state/workspace ceiling, at most 4096 MiB |
| `maxWork` | 100000000 | Per-request whitewater work ceiling, at most 1,000,000,000,000. A request charges 8 units per cell per step it replays (plus 8 per tracer and per birth). The tracers keep checkpoints within their own byte budget, one per second of simulated time at first and every second, fourth, eighth... second when the budget fills, so a request that goes back in time replays from the nearest checkpoint at or before it; a cold seek to 6 s on 518,400 cells with a 1/24 s step still costs about 600 million units |
| `checkpointMemoryMiB` | 64 | Byte ceiling of the retained tracer checkpoints, 0 to 4096 MiB, separate from `maxMemoryMiB`; zero keeps none, so every request that goes back in time replays from zero |
| `foamMaterial`, `sprayMaterial` | absent | Optional scoped material references |
| `foamMode` | `particles` | `particles` draws each foam tracer as a triangle batch; `albedo` makes the foam tracers a coverage of the water surface that the path tracer's shading takes toward the foam (below) and draws no foam triangles |
| `foamRadius` | one cell | In `albedo` mode, radius in scene units of the coverage one tracer gives the surface, at most 64 cells (OCN14); ignored otherwise |
| `foamAlbedo` | .9 | In `albedo` mode, diffuse albedo of fully covered water, 0 to 1 |
| `foamRoughness` | .8 | In `albedo` mode, roughness of fully covered water, 0 to 1 |

At each canonical ocean `dt` endpoint, compute central differences of surface
height and horizontal Froude number `speed/sqrt(gravity*depth)`. A dry neighbour
uses the current wet cell's height for the gradient. Activity is the larger of
gradient magnitude and Froude number. The expected count is
`max(0,activity-threshold)*emissionRate*dt`. Emit its integer part plus a seeded
Bernoulli trial for the remainder, with seeded positions within each cell.
This rate is explicitly per cell: changing grid resolution changes density
unless the author adjusts the rate. Calm level water emits nothing.

Births occur only at canonical endpoints, never at fractional shutter samples.
Foam advects with the sampled horizontal flow and stays just above the bilinearly
sampled wet surface. Spray follows constant gravity plus exponential drag; a
sample at or below the water surface becomes foam. Landing is detected at the
canonical/fractional sample endpoints, so collision accuracy depends on `dt`.
Dry samples remove tracers. Open edges remove out-of-domain tracers, periodic
edges wrap, and closed edges reflect position and velocity. The whole tracer
domain follows the ocean's local pose; it does not become an independent rigid
body simulation when the ocean moves.

The last canonical particle state is retained; a backward seek reconstructs it
from time zero. Fractional samples are discarded after publication. Failed
requests leave published and canonical whitewater states unchanged. Work charges
eight units per grid cell and live tracer per step, and eight per attempted
birth. Source ocean/swell evaluations retain their separate bounds. Constructor
admission includes worst-case particle copies, owned bed capacity and sample
storage; externally retained frames and GPU allocations are outside this budget.

Native rendering builds one foam mesh and one spray mesh, both charged against
the ocean's remaining `surfaceMemoryMiB` after its water surface. Foam uses
8-triangle horizontal discs; spray uses 8-triangle octahedra. Default foam is
white, roughness .8 and opaque; default spray is white, roughness .12,
transmission .6 and IOR 1.333. Both are double-sided and support the ocean's
pose, opacity, depth and shadow settings. Material references override these
defaults. This bounded geometry is tested in raster and path-traced passes;
it is not volumetric mist or a liquid-sheet reconstruction. OCN5 enforces the
single source, ordered window and material-reference constraints.

With `foamMode="albedo"` foam is the water's own: no foam mesh is built, and
each foam tracer instead gives the vertices of the water surface within
`foamRadius` (default one cell, so the coverage is at mesh resolution) a share
`(1 - x^2)^2` of its distance `x` to the radius, full until 60 % of the
tracer's life and fading linearly to zero after it; shares of several tracers
combine as `1 - prod(1 - c)`, in particle order, so the result is deterministic.
The coverage is made on the CPU for every surface vertex, per frame, and is
budgeted (the distances against a budget of their own, equal to the whitewater's
`maxWork` for each frame, that is not shared with the solver's): it counts the distances it will take (every living tracer takes one
for each vertex of the 3 x 3 squares around the one it is in, of a grid of squares that are at least
`foamRadius` a side and at most about as many as there are vertices) before
taking any, and fails with its cause when they pass the whitewater's `maxWork`;
and the arrays of the coverage (16 bytes a vertex and 4 for each square of its
grid, exactly) count in the surface's `surfaceMemoryMiB`.
Foam that is not drawn as triangles is not charged to the whitewater mesh budget.
The share rides in the alpha of the vertex colour and is the share of the
surface's area that the foam covers: a path-traced sample on a covered surface
is on the foam with that probability, and then the surface is the foam's (a
diffuse white of albedo `foamAlbedo`, .9, and roughness `foamRoughness`, .8,
that lets no light through), and on the water otherwise, so that the picture is
the mean of the two weighted by the share and the light that gets through the
uncovered part keeps the water's tint. The refracted shadow ray scales its
transmittance by one minus the same share, so opaque foam stops the sun that
clear water lets through (in expectation: the sun's ray is not drawn at random but
scaled, which is the mean of what the randomly drawn samples of a camera path see, and so the
same approximation as the mix itself), and the albedo guide of the denoiser takes the mean
albedo. The lobe of the first hit of a sample is not drawn at random but from a
sequence with a random offset in each pixel (the offset a hash of the pixel, so that
neighbours are not correlated, and the sequence the golden-ratio one, so that the
samples of a pixel are spread over the share): at 8 samples a pixel, a window of
16 x 16 pixels and a share of one half, the standard deviation of the luminance is
0.0155 (0.156 of the mean) where the random draw gave 0.0354 (0.345) and the
continuous mix before it 0.0061 (0.111); with the denoiser 0.0003 (0.003) against
0.0118 (0.125) and a mean of 0.0984 against 0.0996 of many samples (it was about
5 % under). The correlation of the residuals of neighbouring pixels is 0.014 to the
right, -0.002 below and 0.011 on the diagonal, and a share of 0 or of 1 is as noisy
as before the mix (0.0101 and 0.0087). The variance that remains under a dome is
that of the draws at the later hits of the path, which are random, and of the
sampling of the lobes themselves; a light of the scene is evaluated at every hit
with the lobe of the sample, so a direct light adds the draw of that lobe to the
noise (evaluating both lobes for it is a possible further step). The mix is the path tracer's: the raster renderer reports an error for
a scene with `foamMode="albedo"` instead of drawing no foam, and the water
needs an opaque alpha mode (also an error otherwise). Three warnings say it before
a renderer runs: W08 (a water that is not opaque, is unlit or shines, by the criterion the renderer applies,
read from the document's attributes at the start: a material attribute that animates is read by the renderer at each frame and not seen by it), W06 (a `foamMaterial` with `foamMode="albedo"`, which is not
used) and W07 (`foamMode="albedo"` in a scene where no camera has
`renderer="pathtrace"`); the corpus has a document for each, and the valid
albedo document has the path-traced camera. A metallic water is covered by foam like any other (the foam sample is a
non-metal surface, so a wholly covered metallic sea is the same white as a
dielectric one: 0.1926 against 0.1928 of the quadrature); an unlit water, which
draws its own colour, and an emissive one, whose emission the foam samples would add
to their own, are refused with an error that names the cause and the ocean is not
drawn (W08 says so in the document). Scenes without the mix keep the text of the
plain shaders and their pipelines; the water shader gains the declaration
`override FOAM` and one branch that is off without foam, so that its identity is
a measurement (the same picture, bit for bit, on an NVIDIA adapter) and not a
structural fact. Foam on a surface that lets no light through uses a variant
with only this hook, because forcing the water variant (the refracted shadow
rays) cost 6.7 times the plain shader: 21.25 s against 3.2 s of the GPU pass
`pathtrace trace` on `examples/cinematic-impact/hero.scene.xml` with its
whitewater at threshold 1000 (no tracer) and `foamMode="albedo"`, t = 3.0,
640 x 360, the scene's own samples, an NVIDIA RTX 6000 Ada through `sr-gpu`,
`tools/probe_render.py --size 640x360 --short`, on the first form of the
renderer change (the one that selected the water variant for a foam mix). On
an NVIDIA adapter: water wholly covered by foam equals the quadrature of a
diffuse .9 surface (0.1926 against 0.1928), the picture at shares of .25, .5
and .75 is the weighted mean of the bare and the covered picture to 5 %, a
share of 0 is the unmixed picture bit for bit for opaque and for transmissive
water, and the seven comparison frames and the hero frame keep their hashes.

#### Implemented numerical contract

The CPU solver evolves depth and two horizontal momenta on square x/z cells.
Bathymetry contains scene-y bed ordinates; a surface is at `bedY-depth` because
scene y increases downward. Public cells contain nonnegative depth and x/z
velocity. Spatial samples are cell centres, x fastest. The first-order finite
volume method combines local Lax–Friedrichs fluxes with hydrostatic
reconstruction. Its source is [Audusse et al. (2004), DOI
10.1137/S1064827503431090](https://doi.org/10.1137/S1064827503431090);
the depth/momentum equations and characteristic speed are also documented in
the [Clawpack Riemann book](https://www.clawpack.org/riemann_book/html/Shallow_water.html).
These sources support the numerical method, not a claim that this implementation
predicts a Chicxulub impact. Depth averaging excludes overturning sheets and
vertical jets. Coarse grids introduce diffusion and bathymetry resolution error.

At each face, reconstruct each water depth above the higher of the two bed
elevations. Add the side-specific hydrostatic pressure correction to momentum
flux, retaining the same mass flux on both sides. The unsplit time bound is
`dt * (max(|u|+sqrt(g*h)) + max(|v|+sqrt(g*h))) / cellSize <= 0.45`.
Both maxima are over current cells. Cell depths below `dryTolerance` retain
their water but lose momentum. Only roundoff-size negative depths can clamp to
zero; a larger negative or nonfinite result is a numerical error. Closed edges
reflect normal momentum; periodic edges wrap both axes; open edges use
zero-gradient extrapolation and are **not** an absorbing boundary.

**Second order (`order="2"`).** Each row is reconstructed from the surface
elevation (depth minus the downward bed ordinate) and the velocity, limited with
the monotonized-central limiter (van Leer, 1977): the difference across a cell is
`sign * min(2|a|, 2|b|, |a+b|/2)` for backward and forward differences `a` and
`b` of equal sign, and zero otherwise. The elevation difference is also capped at
twice the cell depth so that face depths stay non-negative. Cells with a
neighbour or themselves below `dryTolerance`, and cells at non-periodic edges,
keep their cell value, so wet/dry fronts and edges are first order. The face
states enter the same hydrostatic reconstruction and local Lax–Friedrichs flux.
The bed stays constant per cell: **order 2 holds for waves over a smooth bed; the
bed-source term remains first order.** Time stepping is the strong-stability-
preserving two-stage Runge–Kutta method (Heun) with the time-step bound halved to
`0.225`; drag is split symmetrically (half a decay before and after). Non-negative
depth at this bound follows the reasoning of Audusse et al. (2004) for MUSCL
schemes; it is verified by the wet/dry conformance tests, not proven for the
unsplit two-dimensional scheme. A negative result beyond round-off remains an
error. Because the flux is unchanged, strong currents keep the first-order
dissipation `|u|+sqrt(g*h)`.

Amplitude lost by a `1e-3` periodic wave after five wavelengths (the conformance
test), by cells per wavelength:

| Cells per wavelength | Order 1 | Order 2 |
|---|---|---|
| 10 | 99.96% | 77.2% |
| 20 | 97.9% | 17.9% |
| 40 | 85.3% | 2.6% |
| 80 | 61.7% | 0.34% |

Resolve waves with at least 20 cells per wavelength when using order 2. A
second-order substep costs about three times a first-order substep (measured
2.8–3.0 on a 256×256 grid) and the halved bound doubles the substep count when
it limits the step; seek work is charged accordingly.

Timed impulses subdivide the canonical step at their exact timestamps. Events
at zero belong to the initial state. Equal-time events retain authored order.
An add-water impulse adds `amplitude * max(0,1-r*r)^2`, where `r` is distance
from the centre divided by radius. A displacement moves water between a central
disc (`max(0,1-4*r*r)^2`) and its annulus (`sin(2*pi*(r-0.5))^2` for
`0.5 <= r < 1`). Positive amplitude moves annular water inward; negative amplitude
moves central water outward. Transfer amount is `abs(amplitude)*sum(discWeights)`
in cell-depth units; multiply by cell area for volume. Donors contribute in
proportion to their weight times available depth; receivers gain in proportion
to their weights. Unresolved support or insufficient donor water is an error.
No cell outside the radius supplies water. A separately authored horizontal
velocity kick uses the full-disc falloff. These are cinematic forcing controls,
not an energy-conserving impact coupling.

Default core settings are 64×64 cells, unit spacing, gravity 9.81 scene units/s²,
zero damping, closed boundaries, canonical `dt=1/60`, dry tolerance `1e-10`,
256 MiB resident workspace, 64 MiB checkpoints, and 100 million work units per
seek. Each substep and impulse charges eight units per cell (24 per substep for `order="2"`). A cold seek to time `t` therefore costs (canonical steps up to `t`) × (substeps per step) × (8 or 24) × (cells), plus the impulses. For example, a 720 × 720 cell second-order ocean with a 1/24 s step costs about 12.4 million units per substep, so a cold seek to 6 s (144 steps of two or three substeps) needs between 3.6 and 5.4 billion units, which is why the ceiling is a trillion while the default of 100 million stays a guard against accidental work. At most four
million cells, 16,384 impulses and 4,096 retained checkpoints are admitted.
Resident input capacities are included in the memory ceiling, which is 256 bytes per cell (400 for `order="2"`: two more state copies and sweep row buffers; measured peak 158 and at most 278 bytes per cell at one million cells). Checkpoint memory
is separately bounded; zero disables retention. Failed seeks leave the published
frame and caches unchanged. Fractional samples never become canonical state,
and replay after checkpoint eviction is bit-identical in the conformance tests.

**Moving bed and bodies (`colliders`).** An ocean that names objects in `colliders`
takes its bed from the scene at every canonical step instead of from its
bathymetry alone; without the attribute nothing changes, bit for bit. An object
with a `<crater>` (a plane or a mesh) deforms the bed: the crater deforms the
object's surface at the step's time, the surface is brought into the ocean's frame
with the inverse of the ocean's pose, and the bed becomes the bathymetry plus the
topmost surface ordinate over each cell centre minus the same ordinate at ocean
time zero. Columns the surface does not cover are unchanged, so the object need
not coincide with the bathymetry. Any other supported primitive without a crater
is a closed body (a plane without a crater is neither and is rejected by OCN6);
its tessellated surface is crossed by vertical lines through the cell centres,
shifted by a fixed irrational fraction of a cell so that no line runs along a
shared edge, and an odd number of crossings is an error. Scene frames come from
the clock mapping, frame cache and rigid-body pass of the smoke solver, so bodies
of the rigid world work; the bed is sampled at both ends of each canonical step
and each substep uses its linear value at the substep's midpoint.

The water column follows the bed: depth is kept and the surface shifts, and
gravity radiates the change. For a given bed this is the exact depth-averaged
form (the kinematic conditions at the bed and the surface cancel in the depth
equation), it conserves water to round-off, and it is how earthquake tsunamis are
started. A bed that does not move reproduces the static solver bit for bit. What
the objects in `colliders` do to the bed is answered by `ocean@bedResponse`: the
default `depthFiltered` attenuates it by the depth of the water and gives the
bodies a form drag (the sections "Depth response of the surface" and "Momentum, in
`depthFiltered`" above); `hydrostatic` is the long-wave answer of the rest of this
paragraph, where a body raises the bed over its footprint by its vertical extent
between the rest level and the bed, so the water it displaces appears first as a
bulge that radiates, and at equilibrium the surface is flat with thinner water
under the body. In that mode the velocity of the water in an occupied column
relaxes toward the body's: with
`t` the occupied thickness and `h` the depth, the fraction `f = t / (h + t)` of the
velocity difference closes over a canonical step, and a substep of length `dt`
closes `1 - (1 - f)^(dt / step)`, so the momentum given does not depend on how many
substeps the CFL bound chose (it was identical to 1e-15 for 2 to 46 substeps).
The momentum given during the last whole step is kept per body and read by the
group for the reaction on the body (`ocean@bodyCoupling="full"`, below). In the
filtered response the water receives the form drag of the body instead, and the
body is credited with what it applied.

Limits of this model, in plain terms. The water is hydrostatic: there is no
vertical velocity, so a bed or body that moves fast produces no jet and no
vertical splash, only the surface following it. With `bodyCoupling="none"` a body
is kinematic with infinite mass: the momentum and energy it gives the water are
not taken from it, so neither is conserved, while the volume of water is; with
`buoyancy` it is loaded by the water's weight and a drag on its vertical motion,
and with `full` it also gets back the horizontal momentum it gave the water, one
canonical step late (below), but roll is not damped and there is no added mass. Occupancy is measured against the rest
level, not the instantaneous surface, because the bed must be a function of time
alone for replay to hold. The column under a floating body is treated as blocked:
water does not flow under it. A crater deeper than the water around it drains the
ring that feeds it; the measured case (radius 52 and depth 25 under 12 units, rim
7, grown over 1.5 s on 2-unit cells) kept the least depth at 1.26 (first order) and
1.11 (second order) and the volume to 1.4e-14, with no negative depth.

Measured before 2026-10-04 and not repeated (tests ocean_bed and ocean_coupling,
`bedResponse="hydrostatic"`): a Gaussian uplift of 1% of the depth in a channel
launched two pulses at 7.95 (first order) and 7.85 (second order) after 4 s against
the 8.00 of `sqrt(g h) t`, with 78% and 98% of half the uplift as height. A falling
sphere of radius 4, 7 and 10 left a far-field wave of 0.22, 0.66 and 1.11 in 12 units
of water; entry speed raised it only until the entry became quicker than the wave
takes to cross the body (0.637, 0.661 and 0.660 for x0.5, x1 and x2).

Resident memory of a driven ocean is charged per cell: 24 bytes for the three bed
vectors (`moving_bed`), 72 more with bodies, 12 more for the owner tags of the
per-body samples (any ocean with bodies) and 32 more for the pushes of the form drag
(the filtered response). Per call, the depth filter charges 24
bytes per cell and 40 per cell of its largest transform window (1024 x 1024 at most)
to `meshMemoryMiB`, together with the collider geometry and its per-column samples.
Checkpoints hold 32 bytes more per body owner (the impulse and the pressure credited). The frame key of a driven ocean
includes the bed, because the same depths over another bed are another surface.

### Terrain, crater evolution, fracture and mesh caches

#### Globe elevation (implemented)

Version 1.3 implements radial elevation for `object3D primitive="globe"`
with an existing `terrain` tiles asset and `map` drape. Plain globes and existing
flat-map terrain keep their previous version and unit semantics. Planetary and
close-up shots can use different scene scales: elevation is in metres, while
`radius` and the resulting mesh positions are in scene units.

| Attribute | Type; default | Contract |
|---|---|---|
| `terrain` | tiles IDREF; absent | Local/resolved PMTiles v3 archive of numeric elevation tiles |
| `terrainEncoding` | `terrarium` or `mapbox`; `terrarium` | RGB numeric height decoding, without a color transfer function |
| `planetRadius` | positive decimal; `6378137` | Reference planet radius in metres; applies to globes |
| `exaggeration` | nonnegative decimal; `1` | Dimensionless elevation multiplier; zero produces the reference sphere |
| `segments` | positive integer; `32` | Effective longitude subdivisions clamped to 24–512; latitude subdivisions are floor(segments/2) |
| `terrainTileSize` | power of two, 1–4096; `256` | Exact width and height of an unbuffered tile |
| `terrainZoom` | integer 0–22; automatic | Explicit sampling level, within the archive's advertised zoom range |
| `terrainMissing` | `error` or `zero`; `error` | Missing tile errors by default; `zero` explicitly substitutes datum elevation |
| `terrainMemoryMiB` | positive integer ≤4096; `128` | Per-surface CPU construction and numeric tile working-set ceiling |

For decoded height `h`, physical reference radius `Rp`, scene radius `Rs` and
exaggeration `e`, the displaced radius is `r = Rs * (1 + h/Rp * e)`.
This follows by converting metres to scene units with `Rs/Rp`. Longitude
`lambda` is east-positive and latitude `phi` is north-positive, in radians for
the following coordinate equation:
`p = (r*cos(phi)*sin(lambda), -r*sin(phi), -r*cos(phi)*cos(lambda))`.
Thus longitude zero faces negative z and north points negative y, matching the
existing globe drape. Nonfinite or nonpositive displaced radii, unresolved
triangles and coordinates outside device precision return errors.

[Terrarium](https://github.com/tilezen/joerd/blob/master/docs/formats.md)
decodes `h = R*256 + G + B/256 - 32768`.
[Mapbox Terrain-RGB](https://docs.mapbox.com/data/tilesets/guides/access-elevation-data/)
decodes `h = -10000 + (R*65536 + G*256 + B)*0.1`.
Supported archive payloads are PNG or WebP with RGB8/RGBA8 channels; alpha does
not encode elevation. Use lossless numeric tiles. Header/payload format and
exact dimensions must agree; grayscale, 16-bit and buffered 260/516-pixel tiles
are rejected rather than reinterpreted. Archive compression uses the existing
none/gzip decoder; Brotli and Zstandard archives are not supported.

Sampling is bilinear across tile edges and wraps longitude at the date line.
Automatic zoom is `ceil(log2(effectiveSegments/terrainTileSize))`, clamped to
the archive's range and the supported maximum 22. It matches sample density to
the authored mesh; it does not create view-dependent adaptive terrain.
Mercator coverage ends at `L = atan(sinh(pi))`, approximately 85.051 degrees.
Beyond either boundary, its sampled elevation is multiplied by smoothstep
`u*u*(3-2*u)`, where `u = clamp((90-abs(latitude))/(90-L),0,1)` in degrees.
Both poles therefore close at the reference radius. These are constructed
polar caps, **not measured polar terrain**; missing-tile substitution and polar
closure must not be interpreted as geophysical reconstruction data.

The date-line vertices and all vertices at each pole share exact positions.
Degenerate pole faces are omitted. Area-weighted normals are accumulated in
f64 across welded aliases, then stored with the GPU geometry. Tangents follow
the local east direction projected into the normal plane. The resulting
triangle surface is shared by rendering, automatic rigid-body colliders,
particle colliders and pyro's closed-mesh obstacle regions. Explicit rigid-body
shape overrides still request their declared approximation. Terrain geometry
is static when used as a collider; position, rotation and scale remain available
for rigid motion. A render-only globe can animate radius, segments and
exaggeration, and backward frame evaluation uses the matching surface.

Admission checks precede surface construction, archive section allocation and
image decoding. Each sampler owns its file and decoded tiles, with at most
4096 retained tile entries and four directory lookup levels. The scene budget
covers conservative surface construction/upload-mirror storage and the
remaining tile working set; it does not bound total process memory. A separate
process cache retains at most 128 MiB or 64 completed surfaces. Active renderer
and collider copies retain their own resources and budgets. GPU vertex/index
buffers are checked against device limits. Cache identities include the local
path, file length/modification time and all geometry/sampling parameters; source
files are expected to remain immutable while rendering.

`GEO1` requires version 1.3 for globe elevation or explicitly authored new
options. `GEO2` checks globe-only physical scale, sampling-option scope and
power-of-two tile sizes. `GEO3` keeps globe sampling configuration and
`planetRadius` static, and rejects animated relief geometry on owned rigid-body
colliders. `P3D6` and `PYRO8` likewise reject animated exaggeration when their
referenced collider is a relieved globe. Includes resolve DEM assets relative
to their owning document and namespace. Failures produce diagnostics rather
than silently replacing relief with a sphere.

The conformance fixture `tests/corpus/valid/globe-relief.scene.xml` uses a
synthetic 2×2 Terrarium tile at exactly 100 metres in
`tests/corpus/media/terrain.pmtiles`; it is a unit/scale example, not Earth data.

#### Crater evolution (rendering, rigid, particle and pyro integration)

Version 1.3 adds one owned `<crater>` to a surface `object3D`. This is authored
cinematic deformation; its parameters do not predict excavation from impact energy.
The same stateless kernel serves raster rendering, path tracing, rigid terrain,
particle contacts and smoke obstacles.

| Attribute | Type; default | Contract |
|---|---|---|
| `centerX/Y/Z` | finite double; 0 | Center in object-space scene units |
| `normalX/Y/Z` | finite double; 0, 0, -1 | Nonzero outward direction, normalized by the kernel |
| `radius`, `depth` | positive / nonnegative decimal; 50 / 10 | Final bowl radius and central excavation depth |
| `rimHeight`, `rimWidth` | nonnegative / positive decimal; 2 / 10 | Final rim height and half-width; width cannot exceed radius |
| `influenceDepth` | positive decimal; computed | Defaults to twice max(radius, depth, rimHeight); at least twice max(depth, rimHeight) |
| `start`, `end` | nonnegative decimal; 0 / 1 | Ordered interval in the owning object's local clock |
| `curve` | enum; ease-in-out | linear, ease-in, ease-out, ease-in-out or step |
| `maxMemoryMiB` | integer 1–4096; 128 | Output vertex allowance and collision component estimates; exclusions below |

Let `p` be the eased fraction clamped to [0,1], `n` the normalized outward
vector, `q = vertex - center`, `h = dot(q,n)` and `r = length(q-n*h)`.
Define `B(x) = (1-x²)²` for `abs(x)<1`, zero otherwise. For `p>0`, displace
along `n` by

```
p * (-depth * B(r/(radius*p))
     + rimHeight * B((r-radius*p)/(rimWidth*p))) * B(h/influenceDepth)
```

At `p=0` the map is exactly identity. The compact axial envelope leaves distant
back surfaces unchanged. Normals use the inverse-transpose differential;
tangents use the differential and are orthogonalized against the new normal.
UVs, vertex colors and tangent handedness are preserved. For this authored map,
`max(abs(B')) = 8/(3*sqrt(3))`; the influenceDepth constraint bounds the axial
derivative and gives `det(J) >= 1-4/(3*sqrt(3)) > 0` in exact arithmetic.
Nonfinite or noninvertible numerical results are errors. Geometry remains at
its authored tessellation; this feature does not refine or change topology.

Rendering applies the map after imported basis/node and skin/morph transforms,
in the owning object's axes. Deformed bounds drive shadow fitting and
transparency sorting. A failure discards that object's partial draws and
reports a render diagnostic. Gaussian splats do not provide triangle surfaces
and are rejected at runtime.

Owned rigid bodies require explicit static or kinematic type and auto/trimesh
shape. Their triangle surfaces are rebuilt at fixed-step endpoints using the
object's local clock and its initial signed scale. Replacements are validated
before installation, wake sleeping dynamic bodies, and retain revision identities
in replay checkpoints. Invalid replacements produce a frame error and stop that
step; physics baking also reports the error. Authored body poses remain governed
by the existing static/kinematic contract. Tests cover falling into a growing
bowl, mirrored/stretched kinematic terrain, delayed object start and seek replay.

The render allowance covers all additional vertex arrays of this object's draws.
Each rigid replacement is admitted using `96*vertices + 524*triangles + 4096`
bytes for geometry and acceleration storage. These are component allowances,
not total-process memory guarantees. Plane and analytic-primitive tessellation
counts are checked before initial allocation; an allocation-observing regression
verifies rejection of a 256-segment plane under a 1 MiB allowance. Import decoding,
extrusion, complete relief construction accounting and retained source meshes
still need tighter accounting for the full feature.

Rigid replay admits optional checkpoints before cloning. The engine defaults
to a 256 MiB admission estimate and at most 64 optional checkpoints, plus its
mandatory initial snapshot. Estimates include current triangle/convex/compound
geometry, contacts and world bookkeeping; shared geometry is conservatively
charged per retained copy. When a new copy would exceed either limit, temporal
spacing doubles and old checkpoints are removed before allocation. An individually
oversized copy is skipped. `World3::with_checkpoint_budget` configures the CPU API;
zero disables optional copies and resetting it restores the initial cadence.
Tests reproduce unbounded growth, distinguish dense meshes from small bodies and
verify identical dynamic/contact replay with tiny or disabled caches. This is an
admission estimate, not measured heap/RSS: initial/current states, input source
geometry, allocator overhead and solver scratch remain outside this allowance.

Particle collider references to crater objects use the same object-space
kernel and sampled local clocks as rendering. The plane collider uses its authored
render tessellation. World-space vertices are sampled at the endpoints of each
canonical particle step, anchored at emissionStart, and move linearly between
those endpoints. All collision refinements within that step share these endpoints;
resampling an accelerating surface after each impact would change the trajectory
and create repeated near-zero-time impacts. This is a piecewise-linear approximation
to the authored deformation and affine motion, controlled by particle `dt`.

A swept triangle BVH prunes candidate surfaces. Sphere contacts solve polynomials
for faces, edges and vertices, then verify the actual closest point and whether the
particle is approaching. For linear relative vertex vector `w`, linear edge `e`
and quadratic face normal `n`, the contact equations are:

```
vertex: dot(w,w) - r² = 0                         (degree <= 2)
edge:   dot(cross(w,e),cross(w,e)) - r²*dot(e,e)=0 (degree <= 4)
face:   dot(w,n)² - r²*dot(n,n) = 0               (degree <= 6)
```

Derivative roots split the interval into monotone pieces; sign changes are
bisected, while critical points also capture tangencies. Finite-precision
candidates are checked against the triangle's closest-point location. Penetrating
candidates are refined by geometric-distance bisection from the preceding outside
candidate: squared polynomials alone can lose a small radius over a long sweep.
A positive radius below 1e-15 of the normalized query extent is rejected; reduce
the step or scene extent. These are numerical predicates, not certified interval
arithmetic. Contact velocity interpolates vertex velocities barycentrically.
Queries cannot extrapolate beyond the sampled interval. Unresolved contacts,
nonfinite endpoints and exceeded geometry budgets produce errors.

The particle trajectory solver refines a curved flight when its chord reports a
hit while the actual velocity is still separating. Repeated supporting contacts
on nearly aligned normals use separation bounded by `collisionTolerance`, avoiding
an accumulation of grazing impacts on changing slopes. Isolated impacts retain
the existing numerical bias. This is contact stabilization within the existing
spatial tolerance, not additional rigid-body coupling or fragment interaction.

Particle surface admission charges `192*vertices + 524*triangles + 4096` bytes
for retained source data, endpoint arrays, construction and the swept BVH. It uses
the smaller of remaining particle `meshMemoryMiB` and crater `maxMemoryMiB`.
Primitive tessellation is checked before construction. One sweep interval is
retained; it is released before its replacement is allocated. Ordinary rigid
collider scale replacements similarly release the old surface first.

Pyro collider references sample the same crater kernel at both ends of each smoke
step, after transforming each endpoint into its corresponding domain axes. The
closed region uses the midpoint of those endpoint vertices; material velocity is
`(end-start)/dt`. A nearest-surface query interpolates vertex velocities by the
triangle's barycentric coordinates. This extends boundary motion to the voxel
faces. Each occupied cell stores prescribed velocities at its six MAC faces,
reusing the prior affine-boundary storage budget. Cell-centre occupancy and linear
endpoint motion are grid/time approximations; they do not resolve subvoxel contact
or predict compressible impact flow.

Crater plane proxies retain the existing centred `colliderThickness` slab and
use the authored plane tessellation on both faces, with closed sides. Rotational
primitives use indexed closed surfaces with shared seams and single polar vertices;
their curved surfaces approximate the analytic primitive at authored `segments`.
A horn torus (tube radius equal to ring radius) is self-touching and is rejected
for deforming smoke regions. Imported rest meshes and relieved globes use the
existing shared loaders. Open, nonmanifold, collapsed or numerically unresolved
regions and inconsistent material motion across welded seams produce errors.
Scene transforms, parent motion, local clocks, visibility-independent proxies and
backward replay follow the existing pyro contract. Relative domain motion is
included in endpoint sampling, so common parent motion does not add relative flow.

A smoke crater surface reserves `512*vertices + 800*triangles + 8192` bytes for
retained topology, endpoint/midpoint buffers, velocity storage and construction
scratch. The smaller of remaining domain `meshMemoryMiB` and crater `maxMemoryMiB`
applies; source meshes share that domain allowance. Exact procedural topology
counts are checked before tessellation, including linear segment growth for cones
and cylinders. Step-local BVHs are dropped after each input has been consumed;
solver checkpoints retain voxel state rather than geometry. Allocation-observing
tests verify early rejection under a 1 MiB crater allowance.

**Remaining work:** rigid-body crater updates remain discrete surfaces without
continuous deformation boundary velocities. Animated imported-source parity,
complete importer allocation accounting and measurement of retained rigid-world
allocations need further work. The full crater workload remains incomplete until those requirements
and production validation are finished.

Rust and Schematron share **CRT1** (version), **CRT2** (single surface owner),
**CRT3** (ordered timing), **CRT4** (finite direction/profile/envelope) and
**CRT5** (rigid-body compatibility), with **CRT6** to **CRT8** for a crater from an
impact, **CRT9** for `capture` and **CRT10** and **CRT11** for `mantle` and `bulking`. `tests/corpus/valid/crater.scene.xml`, `crater-impact.scene.xml` and
`tests/corpus/invalid/crt1.scene.xml` through `crt9-capture-without-source.scene.xml` independently
exercise the XSD/Schematron and Rust validators.

#### Crater from an impact (`crater@source`)

A crater whose size and timing are the consequence of an impact rather than authored.
`source` names the dynamic rigid-body object that makes it; the owner is the surface the
`crater` belongs to. Nothing in the document says when or how big: the crater begins at
the first step in which the source, approaching the owner, pushes on it with more than
twice its own weight in one step (so a body resting on the surface never starts one), is
centred on the impulse-weighted contact point with its axis along the normal of the owner's surface
there (a mesh's corner normals interpolated across the triangle the point is on, so that finer or coarser
triangles of the same surface give the same axis; a sphere's radius; a box's face; the contact's normal only
chooses the side and is used as it is for shapes that give none), and
grows in composition time over the law's formation time. Radius, depth, rim, start, end,
centre and axis are derived, so giving any of them is **CRT6**; `targetMaterial` is
required with a source and the other target attributes belong only to one (**CRT7**); the
source must be another object3D with a dynamic rigidBody (**CRT8**). `curve`,
`influenceDepth` and `maxMemoryMiB` keep their meaning. The impact is part of the rigid
world's state: the same after any seek, with or without the frame memory, in a fresh world
and from a baked SRPHYS04 cache, which carries the contacts it is found in.

`crater@capture` (boolean, default false, only with `source`: **CRT9**). The rigid contact kills the normal
velocity of the body that makes the crater and nothing takes the rest: the impact scene's rock (2 m radius,
90 478 kg, 100 m/s at 60 degrees) keeps 17 % of its kinetic energy (the free rock's in the same test) as a sliding that friction turns into
rolling (35 m/s, five sevenths of its 50 m/s along the ground, omega r = v, by the formula) and leaves the crater, which opens over half a
second after it has crossed the rim. With `capture` the body is arrested from the impact, for a window of
`2 d / U` plus one step (the time that stops a body at the impact speed `U` in the law's crater depth `d`; slower
bodies stop sooner) and only while its centre is inside the crater's rim radius in the owner's frame: its
velocity relative to the owner loses `U^2 / (2 d)` a second along its direction, never more than stops it in
the step, and its spin loses the same share. All of its kinetic energy can leave it and none can be added. After
the window it is the ground's: it sinks with the floor of the pit and rocks on it (nothing resists rolling, so a body that sinks to the floor of the bowl swings on it
about 0.3 m either way and settles very slowly: a limit of the rigid model). This is a model of the engine, the
mean force of a penetration of depth `d` (kinetic energy over depth), not a published law; the projectile neither
breaks up nor buries itself. The deceleration is a function of the impact the world noticed and the body's
state, so any order of requests and a fresh evaluator give the same bits, and without it the world is what
it was. Measured on the impact scene's rock arriving at 100 m/s (test `crater_capture`, commit 90ce0f1, 2026-10-05,
load1 6 to 10; the results do not depend on the load): rim 7.16, 6.66, 5.04 m and depth 3.00, 2.79, 2.12 m
for 90, 60 and 30 degrees; at 4 s it is 0.04, 0.49 and 1.22 m from the impact point, sunk into the pit (y 0.97,
0.68, -0.39 m, the 2 m radius resting on flat ground is y = -2), at 0.7, 0.4 and 1.3 m/s, and its kinetic energy is
3e4, 8e3 and 1e5 J against 4.5e8 at the impact (the free rock at 60 and 30 degrees keeps 7.8e7 and 2.4e8 J). The
mechanical energy never grows after the impact by more than a ten-thousandth of the impact's, and the free rock's
energy does not grow either. In the ocean scene the water takes momentum from the rock (`full`: the water
relaxes toward the rock's velocity in the hydrostatic response and receives its form drag in the filtered
one), so the rock reaches the bed slowed. At 6 s, with and without `capture`, it is 0.6 and 0.7 m from the
crater's centre in the hydrostatic response (the highest wave after 4 s 2.136 and 2.132 m) and 0.1 and 8.5 m in the
filtered one (2.097 and 2.083 m): with the depth filter the free rock rolls out of its crater and `capture` keeps it
in (test `impact_scenes` on 3 m cells, commit e22458b on fa63e5d, 2026-10-05, deterministic).

`crater@mantle` (boolean, default false, only with `source`: **CRT10**) and `crater@bulking` (1 to 1.3, only with `mantle="true"`:
**CRT10**, **CRT11**). The ground of a crater that grows from an impact takes out less than the law excavates and gives back none of what
the law throws out: for the authored rock in soft rock the kernel's bowl, whose bump reaches the rim crest, excavates 129.7 m3, 1.285 of the law's
volume of 100.9 m3, and the rim puts back 32.9 m3, 0.326 of it, so the ground loses 0.959 of the volume and the 0.8 of it that the law says was thrown
out (80.7 m3) is nowhere in the ground (`sr_sim::cratering::crater`, `sr_3d::crater`; measured by a disposable program on 2026-10-06). With `mantle` the
volumes are a contract. The bowl excavates exactly the law's volume `V` with the depth and the radius of the law: its bump is `(1 - x^2)^p` with
`p = pi R^2 d / V - 1` (`R` the rim crest's radius, `d` the law's depth; `p` is 2 for the crater as it was, and the bowl is made deeper if `p` would
be under 2). The ejecta come down outside the rim as a mantle `t0 (R / r)^3` thick (McGetchin et al. 1973, Collins et al. 2005, from the law's own
text: the inverse cube), brought to zero over the rim's width by a smooth ramp, cut at twenty crest radii and scaled to hold exactly the law's ejecta
volume, `0.8 V`; it is part of the ground, the same deformation as the bowl and the rim, and grows with the crater as they do (every part of the
map, its crest and its height, is scaled by the progress). The rim and the mantle together put back `bulking` times `V` (broken rock takes more room than the
rock did): with `bulking` the rim has the volume `bulking V - 0.8 V` (0.2 V at 1), and without it the rim keeps the height of the law, which is
a measured relation of the law and not cut, and the bulking is what that asks for, 1.126 for the authored rock in soft rock (0.326 V of rim and
0.8 V of mantle), inside the physical range of the debris and derived from the law and not chosen. The integral of the change of height over the
ground is then `(bulking - 1) V`, to 1e-6 of `V` (test `crater` of `sr-3d`). The transient stage of the crater, its collapse into the final one, is not
modelled: the law's radius is the final, apparent radius, and multiplying it by 1.2 to 1.3 would count the collapse twice. Off, which is the default,
the ground is as it was, to the bit.

The ejecta particles that come to rest on the ground are taken out where they lie (`particles3D` runs `Spec::settle`, 0.5 m a second relative to the surface, a choice and not a measurement) so
that what they are made of is not counted twice, once as particles and once in the mantle; they are told to the ocean's coupling as ground, and only the water's take from it. For the authored
rock 379 of 1000 ejecta have been taken out by 5.5 s, and 88 are in the air at 1.6 s in both runs. The mantle that the law's own volume gives, and not the footprint of the particles' landing
points, is what the ground gets; the comparison is a measurement and not a part of the model: for the authored rock, on level ground, with no drag and 20 000 particles, 84 to 86 % of the mass of the ejecta
the engine launches (Housen and Holsapple) comes down inside the crest radius, back in the crater, and the rest lies between one and about three and a half crest radii, steeper than the inverse cube
(log-log slope of the thickness -4.1 to -7.0 against -3, for launch angles 30, 45 and 60 degrees, `the_ballistic_landing_points_of_the_ejecta_make_a_mantle_that_is_measured_against_the_inverse_cube`
in `sr-sim`); the analytic mantle, with all of its 0.8 V outside the rim and out to twenty crest radii, is therefore the law's statement of where the ejecta are in the final crater and not what these
particles do, which is not corrected here.

Granular debris is a height field of deposit over the ground (`sr_sim::granular::Bed`) that relaxes to an angle of repose in a fixed order over sixteen neighbours, without making or losing
volume. `crater@repose` (10 to 60 degrees, only with `source`, never with `mantle="true"`: **CRT10**, **CRT12**) connects it to the scene: the emitter of the crater's ejecta takes out each
particle that comes to rest on the ground (0.5 m a second relative to it, as with the mantle), tells where it lay in the plane of the crater and how much of the target it stood for (its mass over the
target's density), and the volumes are poured, step by step, onto a bed of 144 by 144 cells (twelve to the crest radius, six crest radii to each side) over the grown crater, which relaxes after each step.
The bed after the steps that a frame's particles have computed lies on the ground in that frame, as a height added in the crater's deformation to what the bowl and the rim give. The bed is a fold of
a log of what settled, so it is the same whatever order the frames are asked in (tested to the bit), and is kept at checkpoints like the particles'. For the authored rock thrown as 1000 ejecta, 73 621 kg of the 169 634 kg
have settled by 5.5 s and the deposit is 35.0577 m3 of the 35.0577 that their mass stands for. Limits: the particles and the rigid world land on the ground without the deposit; what reads the crater
after the particles in a frame (the render, and any simulation that runs after them) sees it; material that settles beyond six crest radii is put on the edge of the bed; an emitter deposits on one crater; and
the ground the deposit relaxes on is the final crater, also for what settles during its growth. A volume poured at one point of level
ground piles into a cone whose flank, in each sector of 15 degrees, is within 2 degrees of the declared angle when the pile is ten cells of radius or more (34.1 to 35.6 degrees for 35, and 29.2 to 30.5, 23.7 to 25.5,
38.3 to 40.6 and 34.5 to 35.6 for 30, 25, 40 and 35 at 10, 12, 12 and 20 cells), and up to about 5 degrees off for piles of 5 to 8 cells (26.2 to 30.0 degrees for 30 at 5 cells and 26.9 to 30.8 at 8). Under water the mantle changes the sea sweeps, whose tests assert order and not numbers.

The size is Holsapple's pi-group scaling law (Annu. Rev. Earth Planet. Sci. 21:333-373,
1993, doi 10.1146/annurev.ea.21.050193.002001, Eq. 18). With `pi_V = rho V / m`,
`pi2 = g a / U^2` (no factor of 3.22) and `pi3 = Y / (rho U^2)`,

    pi_V = K1 { pi2 (rho/delta)^((6nu - 2 - mu)/(3 mu))
                + [K2 pi3 (rho/delta)^((6nu - 2)/(3 mu))]^((2 + mu)/2) }^(-3 mu/(2 + mu))

where `m`, `a` and `delta` are the source's mass, equivalent radius and density (mass over
the volume its shape encloses), `U` is its speed along that normal before the step
(the normal component of the relative velocity, so the crater shrinks as the approach
turns glancing), `rho`, `Y` the target's density and cratering strength, `g` gravity and
`nu = 0.33`. The strength term's exponent is `(2 + mu)/2`; the 1993 table prints
`(2 + mu)/mu`, which Holsapple later corrected. Constants, in SI units (the source gives
`Y` in dyne/cm2 and `rho` in g/cm3), are those of the author's calculator note,
"Theory and equations for Craters from Impacts and Explosions"
(lpi.usra.edu/lunar/tools/lunarcratercalc/theory.pdf), which holds both regimes for every
material in one table:

| `targetMaterial` | K1 | K2 | mu | Y (Pa) | rho (kg/m3) | Kr / Kd |
|---|---|---|---|---|---|---|
| `water` | 0.98 | 0 | 0.55 | 0 | 1000 | 0.8 / 0.75 |
| `drySand` | 0.132 | 0 | 0.41 | 0 | 1700 | 1.4 / 0.35 |
| `drySoil` | 0.132 | 0.26 | 0.41 | 2e5 | 1700 | 1.1 / 0.6 |
| `wetSoil` | 0.095 | 0.35 | 0.55 | 5e5 | 2100 | 1.1 / 0.6 |
| `softRock` | 0.095 | 0.215 | 0.55 | 1e6 | 2100 | 1.1 / 0.6 |
| `hardRock` | 0.095 | 0.257 | 0.55 | 1e7 | 3200 | 1.1 / 0.6 |
| `regolith` | 0.132 | 0.26 | 0.41 | 1e4 | 1500 | 1.1 / 0.6 |
| `ice` | 0.095 | 0.351 | 0.55 | 1.5e4 | 930 | 1.1 / 0.6 |

The excavated volume is `V = pi_V m / rho`; the crater's radius at the original surface is
`R = Kr V^(1/3)` and its depth `Kd V^(1/3)` (calculator note); the rim crest is at `1.3 R`
and `0.036` of the rim diameter high (Housen et al. 1983 profiles; Pike 1977, as quoted
there); the rim spans from `R` to its crest. In the crater element's kernel, whose profile
is centred on the crest, `radius` is the crest radius and `rimWidth` is `0.3 R`. The
formation time is `0.8 sqrt(V^(1/3) / g)` (Schmidt and Housen 1987, via the note; other
sources give 0.5 to 1, so it is good to a factor of two). The ejected volume, `0.8 V`, and
the thickness of an inverse-cube blanket at the crest that holds it are computed and
exposed for the ejecta, but nothing is deposited on the terrain.

Units: `physics@pixelsPerMeter` converts scene units to metres for everything the law reads
and gives. `targetDensity` is kilograms per cubic metre, `strength` pascals, `gravity`
metres per second squared; gravity defaults to the magnitude of the physics gravity and is
an error when that is zero and none is given. Housen and Holsapple (2011, Icarus 211:856-875,
doi 10.1016/j.icarus.2010.09.017) say the dependences on strength and porosity are "only
poorly constrained", so the table's strength and density can be overridden; they should
be read as calibration inputs, not measurements.

Limits. One crater per element: the first qualifying contact of the
source with the owner defines it, and later contacts neither start another nor change it.
The crater is circular, with no elongation or downrange shift for an oblique impact; real
ones elongate only at grazing angles (Bottke et al. 2000, doi 10.1006/icar.1999.6323;
Gault and Wedekind 1978), and the asymmetry of an oblique impact shows in
the ejecta (below). Below roughly 15 to 30 degrees from the surface real impacts ricochet; the law
is still applied to the normal component, which keeps the size monotone in the angle, and
no ricochet is modelled. For large, fast craters angle matters less than the normal
component says above 45 degrees (Davison and Collins 2022, doi 10.1029/2022GL101117). A
water layer above the owner is not treated: the crater is the seabed's. The `ice` and
`regolith` rows are the least certain: the author's later table (arXiv:2203.07476) gives
other constants for every material, for example `hardRock` K1 0.06 against 0.095 here,
and an ice strength three orders of magnitude higher, so the two tables are not mixed.
Housen and Holsapple (2011) use `nu = 0.4` where this law uses 0.33, and no verified
mapping connects their constants to these; ejecta built from the later paper take only the
shape of their distribution from it. A 40 m iron body at 20 km/s and 45 degrees into hard
rock gives an apparent crater 0.8 km across here, against a transient crater of 1.3 km in
the Earth Impact Effects calculator (Collins, Melosh and Marcus 2005,
doi 10.1111/j.1945-5100.2005.tb00157.x), a difference within what the two laws disagree by.

Smoke from an impact. A `pyroSource` or `pyroImpulse` with `crater` naming such a crater is the
smoke the impact causes, and nothing is authored about it: its shape, place, timing, density,
temperature and expansion are derived (PYC1), the crater must exist and grow from an impact
(PYC4), and the engine's parameters belong only to such a source (PYC2). The source is a
sphere of the crater's radius at the impact point, active from the impact over the crater's
formation time (an impulse: at the impact), both starting at the first smoke step that begins after the impact, which is when the impact is known. The dust is `dustFraction` of the volume thrown
out of the crater (`0.8 V`), injected as a volume fraction of solids spread over the cells whose centres the
sphere covers and that are not solid (a ground collider takes none of it; a sphere smaller than a voxel is widened to cover one; an impact outside the volume puts
nothing in it), so that the dust injected is the dust and not what the grid happens to cover, it depends
on no unit of mass and `medium@extinction` is, in these sources, extinction per unit volume fraction. The
heat is `heatFraction` of `(1/2) m U^2 sin(theta)^(3/2)` for the body's mass, speed and angle
from the surface; the exponent is the scaling of shock energy with the angle in the 3D
hydrocode runs of Pierazzo and Melosh (2000, doi 10.1146/annurev.earth.28.1.141), while the
crater uses the normal component. It warms the dust by `heat / (dust mass x specificHeat)`
(dust mass: its volume times the target density), at most `maxTemperature` kelvin: a declared
physical cap (vaporisation), not a fallback, below the solver's limit of 50000 K. The solver
derives the expansion from the heating, as an ideal gas at constant pressure, `(dT/dt)/T` in
every cell heated, so there is no authored `expansion`; a push (`velocityX` to `velocityZ` of an impulse,
`velocityRateX` to `velocityRateZ` of a source) may be authored and is added to the cloud, in the volume's axes; a sealed domain cannot sustain it, as
for any expansion. The literature gives ranges, not values, for the share of an impact's
energy that goes into a plume (internal energy of target and body 0.70 to 0.91 of it at 5 to
45 km/s in strong rock, O'Keefe and Ahrens 1977; ejecta kinetic energy 0.07 to 0.5 of it), and
none for the part that rises in smoke, so `heatFraction` 0.1, `dustFraction` 0.01 and
`specificHeat` 1000 J/(kg K) are the engine's, with no published value, to be calibrated. In
physical units a slow impact barely heats anything, and even a 20 km/s impact of a body of
1500 kg heats its dust by only a few hundred kelvin at these defaults, because the dust grows
with the crater and the crater grows more slowly than the energy: a scene needs physical
impact speeds, or a smaller `dustFraction`, for a fireball. This is the engine saying what the
numbers say. What the model represents is dust heated by the impact, spread through the crater's
volume; what it does not represent is the vapour and melt produced near the point of impact and the
shock wave, which are what a real fireball is made of and belong to later work, so the defaults are not
to be tuned to make fire. Because the density of these sources is a volume fraction of solids, the
medium's `extinction` for them is an extinction per unit volume fraction: for grains of diameter `d`
(in scene units) the geometric-optics value is about `1.5 / d` (3/2 over the grain diameter), so a
smoke of 2 mm grains in a scene in metres has an extinction near 750 per unit fraction, and the
author sets `medium@extinction` to about that for the smoke to be seen.

Ejecta from an impact. A `burst` of a `particles3D` with `crater` naming such a crater is what the
impact throws out, and nothing is authored about when, where, how fast or how heavy: `count`
particles are born at the instants of their launches, each at its launch distance from the impact
point in the tangent plane of the contact (placed on the surface as described below), with its own
velocity and mass, and the instant, `repeat`
and `interval` are not given (P3D7); the crater must exist and grow from an impact (P3D8); `angle`
and `angleSpread`, the launch angle above the tangent plane and the half-width of its uniform
spread in degrees (default 45 and 15), belong to such a burst (P3D9) and stay together within
0 to 90 (P3D10). The speed of the ejecta launched from distance `x` follows Housen and Holsapple
(2011), `v/U = C1 [(x/a)(rho/delta)^nu]^(-1/mu) (1 - x/(n2 R))^p` between `1.2 a` and `n2 R`, `a` the
body's radius, `U` its speed, `rho` and `delta` the densities of the target and the body, `R` the radius
of the crater and `nu = 0.4`, with the constants `mu, C1, n2, p` of water, dry sand (also dry soil), hard
rock and weakly cemented basalt (also wet soil and soft rock); regolith and ice have no row and
are an error. The mass launched from within `x` grows as `x^3 - (1.2 a)^3`, and the particles
together hold 80% of the crater's mass (`0.8 rho V`); each stands for the same share, at a launch
distance that is the corresponding quantile, so the count changes the resolution and not the total.
The paper could not be consulted: the equations and constants are those of the specification this
was written from and are **to be checked against it**. Where the paper is silent the engine decides:
the launch angle and its spread, the time of launch (`T (x/R)^3` after the impact, `T` the
formation time, so the fastest material leaves first), and the lopsidedness of an oblique impact,
whose density of mass over the azimuth from downrange is `1 + b cos(az)` with
`b = clamp((45 - theta)/20, 0, 1)` for the angle `theta` of the velocity with the surface, resting on
the published thresholds of 45 and 25 degrees (Herrick and Forsberg-Taylor 2003,
doi 10.1111/j.1945-5100.2003.tb00001.x) and no published formula. Each particle is a pure
function of its index and the seed, so the particles do not depend on the thread count or on a
seek. The crater's place and axis are the owner's at the impact (a moving owner does not carry the
launches along), the speed is the whole relative speed of the body, and the positions and
velocities are in scene units through `physics@pixelsPerMeter`. Where a particle starts: the contact point lies
inside the owner's surface by as far as the body went on before its contact was found, and by the time most are
launched the crater has grown, its rim risen over places that were level, so a particle born at the height of the
original plane is under the surface and the surface pushes it down through the ground. Each particle therefore
starts from the nearest point of the owner's surface to the contact point (`sr_sim::surface::nearest`, for
spheres, boxes and meshes) moved to its launch distance in the tangent plane, taken through the crater's map at
the progress of its own instant, and clear of the surface along the owner's geometric normal by the radius it
collides with (`collisionRadius` times one plus `scaleVariance`, plus `collisionTolerance`). With the rocks' own
collision radius of 0.17 m none of the 4000 particles of the land scene ends below the ground (commit 68e9dd2,
2026-10-05, test crater_ejecta_ground; the counts before are in the ledger); with 0.02 m, smaller than the distance between the chords of the 1 m facets
of the ground and the true bowl and rim, about 1500 still end below it.; gravity, drag and colliders are
those of the emitter, and ejecta born before `emissionStart` or at or after `emissionEnd` are not
made. More particles than `maxParticles` is an error, not a truncation.

Not read in a primary source: Holsapple and Housen (2007), Schmidt and Housen (1987),
Housen, Schmidt and Holsapple (1983), Pike (1977) and Gault and Wedekind (1978) are known
through Holsapple's documents and the papers that cite them. The calculator note's table
and Holsapple (1993) were read directly.

#### Coupled solvers (rigid world and ocean)

An ocean whose `colliders` list a rigid body forms a group with the rigid world; no attribute
says so. Each member can be replayed alone, so the members do not ask each other again (that
would restore the other's checkpoints in turn): the ocean writes the outcome of each of its
canonical steps, once, into an append-only exchange log of small records, and the rigid world
reads from it the load for each of its steps. Writing a step again must reproduce its record to
the last bit, or the write is a divergence error; the log has an internal byte budget (16 MiB)
whose overflow is an error; nothing is dropped, because a record dropped could not be replayed.

The rigid world's step that starts while the ocean's canonical step `m` is the last completed
reads the outcome of step `m - 1`: a delay of one whole ocean step. It is what makes the
exchange causal, because to advance a step the ocean samples the bodies at its start and at the
next canonical instant (to measure their velocity), so it reads the rigid world a full step
ahead of the outcomes it has. A group therefore applies its ocean before the rigid world in a
frame; the ocean's steps pull the rigid world ahead in time order, and the rigid world never
needs an outcome that does not exist. A load that is not there is an error that names the body,
the step and how far the ocean has got, never a stand-in: the last known value would make the
result depend on the order of calls. Smoke and 3D particles may read the rigid world up to one
of their own steps past a frame, so after answering a frame the group steps the ocean on to the
outcomes those readers can ask for (the longest `dt` among the smoke volumes and particle
systems that collide with the group's bodies); a group therefore simulates a little beyond the
instant asked for (in the cases tried, smoke with a `dt` of 0.2 s read the rigid world no further than the ocean had already reached, so the extension is a tested safeguard that no scene has needed). The rigid world takes a `Load3` (force and torque, scene axes and units) per
dynamic body per step through `Driver3::load`, which defaults to none: a world that is never
loaded is unchanged, and a group whose coupling gives no load is bit-identical to no group.

Every member of a group runs on one clock, and it may be any clock that stretches time the same
way everywhere: a `group`'s `timeScale` and `timeOffset`, nested or not (a clock the engine finds as
`scale * t + offset` of the composition's), or the composition's own. The ocean, every body of the 3D
world (the world is one, so a body the ocean does not carry counts too) and every smoke volume and
particle system that collides with a body of the world must run on that same clock, and one that does not
is an error that names it and the ocean whose clock it differs from; a remap, a loop, a freeze or a clip's
rate on the way is not a uniform clock and is refused. The ocean's local time is the
group's time less its `start`, because the exchange is indexed by it, and the rigid world runs on the group's time
(`physics@start`, the steps of `fixedStep`, the force fields' windows and the ocean's `dt` are in it): a scene in a
group of `timeScale` 0.5 at composition time `T` is, bit for bit, the scene without the group at `T / 2`
(and at `2 T` for a scale of 2; the tests compare the ball, the water cells and the particles, for scales of
0.3, 0.5, 0.8, 1.5 and 2, for nested groups and for a late-starting group, in any order of requests and
from a fresh evaluator). The world starts at the composition time at which the group's clock reads its
start; a clock that puts that before the composition begins (a negative `timeOffset` against a world that
starts at zero) is an error, since the scene cannot be asked about before it. Not covered: smoke from a crater
and particles that fall into an ocean keep requiring the composition clock, as does a 2D rigid world, which
stays on the composition's time whatever the group does. The rigid world's frame memory, checkpoints and the log together
make a backward request cheap: it is answered from the frame memory, or by restoring a
checkpoint and replaying with the logged loads, and the two agree bit for bit.

Internal edges of meshes (`physics@fixInternalEdges`, default false). The contact of a body with a mesh of
triangles is reported against the triangle edge or corner it meets, and the normal there tilts with the
tessellation. With the attribute true the world builds its meshes with the parry flag that takes adjacent
triangles into account (`FIX_INTERNAL_EDGES_TWO_SIDED`, the mesh taken as two-sided: the one-sided flag discards the
contacts of a mesh wound the other way, which loses the impacts on a sphere's mesh), and a flat mesh gives its own
normal in every tessellation. The attribute changes the motion of every 3D body that lands or slides on a mesh,
which is why it is off, and a physics cache baked with it carries it in its identity (a world without it hashes as
it did). Crater impacts take their axis and speed from the owner's surface either way. The impact scenes set it;
their sweeps are the same orderings with it on and off. Measured: grounds of 8, 13 and 40 cells with moved
corners and alternating diagonals (test `internal_edges` of sr-sim, commit 90ce0f1, 2026-10-05, a deterministic
result): with the attribute the normal is (0, 1, 0) to 1e-9 and the body is not kicked sideways; without it the
normal tilts by up to 0.149. On the 1 m ground of the land scene a rock arriving at 60 degrees got, without it, a
contact normal (-0.103, -0.045, -0.994), a normal speed of 91.2 m/s for the authored 86.6, and was pushed 7.35 m
across its line of travel in 4 s (0.42 m with the edges fixed): a single run with a disposable program on
2026-10-04, before commit b8eaa4b, not a test and not repeated. The normal speed of the land scene with the
attribute is 86.63 m/s for the authored 86.60 on grounds of 8 to 160 segments (test `crater_normal`, same commit
and date).

Buoyancy and the full coupling (`ocean@bodyCoupling="buoyancy"` and `"full"`). The group above gets its
first physical coupling. Each rigid body in the ocean's `colliders` is loaded, every rigid step, with the
weight of the water it displaces (the density of the ocean, `ocean@density`, 1000 kg/m3 when it gives none, and the ocean's gravity) upward through the centroid of
the submerged volume, which turns a tilted body, and with a quadratic form drag on its vertical motion,
`-(1/2) rho C_d A |v| v`, `C_d` = `bodyDrag` (default 1.0, an engine parameter and not from the impact
literature) and `A` the area the submerged part presents from above, limited to what stops the body within a
step. The submerged volume is that of the body's shape (analytic for a sphere, a closed mesh for a box,
cylinder, cone, capsule, mesh or decomposition) below the water's free surface under the body: the plane of
the per-body samples below, from the canonical step before the last one the ocean has completed, or the
rest level, `waterLevel`, in the ocean's own axes while there is no such step or the body holds no wet
column. The load is held for the whole step and evaluated where the body will be halfway through it, because
a position-dependent force held from the start of a step adds energy (the motion grows) and from the middle
adds none. The waterline is a spring of stiffness `rho g A_wl` and the explicit step is stable only if its
frequency times the step is below 1.8: a body too light for `physics/@fixedStep` is an error that names
it, not a motion that blows up. Such a document cannot be baked into a physics cache, since the loads come
from the water.

Reading the surface under the body gives the radiation damping that a rest level lacks: the body's motion
raises and lowers the water it floats on, and the water takes the energy away. With `bedResponse="hydrostatic"`, a ball of 2094 kg and 1 m
radius, dropped 1.5 m into a closed 16 m ocean 20 m deep (test `ocean_buoyancy`, which pins the response; the figures
are those of commit 76618aa, 2026-10-04, and the test passes at 90ce0f1, 2026-10-05; not repeated), is within 3 cm of its draft for good from 8.75 s
reading the surface and from 24 s at the rest level, and 60 s later is 1.1 cm below the draft the weight
needs (1.2 cm above at the rest level); with no drag at all it settles from a swing of 1.2 m to under 10 cm
by 15 s, where at the rest level it kept bobbing. No linear damping term is added: the algebraic
convergence of the floating ball is the radiation damping, and the surface plane under a body is not
the undisturbed surface while the body is displacing water (the water it displaces stands above the rest
level, so the plane is higher than the swell), which the draft at rest does not feel because the surface
is flat again then.

`full` also gives the body back the horizontal momentum it gave the water. The momentum per unit density
that the body gave in a canonical step, `p`, becomes a force on the body's centre, in the ocean's horizontal
axes mapped into the world, `-rho p / (s^3 dt)` (`s` scene units per metre, `dt` the canonical step), held
for exactly the rigid steps that read that canonical step: a canonical step that is a whole number of
rigid steps is read by that number of them, counted in integers, since a window of one rigid step more or
less is a few per cent of the momentum. It is read one canonical step late, by the rule of the group, so
the momentum of the body and the water is conserved to what is in flight: in a closed basin of 160 m (`bedResponse="hydrostatic"`), before
the waves reach the walls, a ball of 16 755 kg at 3 m/s and a canonical step of 0.1, 0.05 and 0.025 s keeps
total momentum to within 13 %, 10 % and 4.8 % of its own at the worst of three instants (test `ocean_full`,
commit 90ce0f1, 2026-10-05, a deterministic result: 13.2 %, 10.5 % and 4.8 %). Roll is not damped, there is no
added mass, and the force acts at the centre.

The pressure of the water on a body, in the exchange. The per-body sample's `pressure` is the bed source of the
scheme itself, credited to the body by the ocean solver (the paragraph on `BodySample::pressure` above), and the
exchange log records it with the momentum the body gave the water, to the bit on a replay. It is not applied to
the body: the force on it is the push above. In the closed basin of the test above (the ball at 3 m/s, canonical
step 0.05 s, first order; the momentum of the water along x against the sum of the records of the exchange up to
the canonical steps that have ended by 1, 2 and 3 s; test `the_water_has_the_momentum_the_exchange_says_the_body_gave_it_and_the_pressure_is_what_is_left`,
commit 1ba58cd, 2026-10-05, deterministic): the water has 3.8 %, 3.1 % and 11.0 % more than the body gave it with
`bedResponse="hydrostatic"` and 10.7 %, 12.3 % and 14.6 % more with `"depthFiltered"`; with the credit it has
0.00 %, 0.00 % and 0.00 % more in the first and 0.01 %, 0.04 % and 0.09 % in the second, the residuals of the
solver's own balance in the paragraph above.

Per-body water samples. With bodies in an ocean the solver tags every occupied column with the body
that holds most of it (its position in the ocean's `colliders` list, which counts the surfaces
of craters; the lowest position wins a tie) and, together with the exchange of each completed canonical
step, offers the driver for every body that holds a column at the end of the step or gave the water
momentum during it: the horizontal momentum, per unit water density, that this body gave the water
in the step (the part of the total exchange that its columns gave; the sum over bodies is the total
to rounding, since a column's momentum is credited whole to its owner); and, from the state at the end
of the step, the least-squares plane through the free surface of the wet columns of the body's
footprint (its ordinate at their centre and its two slopes), the depth-weighted mean horizontal water
velocity under the footprint, and the mean bed ordinate under it with the body's thickness counted
as bed. Under a body the water that it displaces stands above the rest level, since a column keeps
its depth while the body raises its bed, so the plane there is not the undisturbed surface. A direction
in which the footprint has no extent (one row of columns, one column) has no slope. A replay of a step
offers the same samples bit for bit, restored from a checkpoint or not. The rigid world reads them, from
the group's log, for the surface under each body and the reaction on it. Without bodies,
or without tags, nothing changes, and tagging does not alter the water.

#### Fracture (native scene, rendering and cache integration implemented)

Version 1.3 defines an owned `<fracture>` declaration. The generated typed model,
XSD, Schematron, Rust rules, evaluator, rigid world and both renderers are connected.
An active fracture replaces the object's source draw with world-space rigid pieces;
new cut faces use `interiorMaterial`, while exterior faces retain source materials,
texture coordinates, imported texture maps and globe map drapes. Fragment motion
invalidates isolated-group caches. This establishes native scene execution; it is
not the final UHD impact-film acceptance gate.

| Attribute | Type; default | Contract |
|---|---|---|
| `at` | nonnegative decimal; `0` | Composition seconds; first eligible physics boundary at or after this time |
| `pieces` | integer 1–4096; `8` | Exact requested count, subject to geometric feasibility |
| `seed` | unsigned 64-bit integer; `0` | Read from the typed field, preserving all bits |
| `interiorMaterial` | material ID; required | Independent material for new cut surfaces |
| `interiorUvScale` | positive decimal; `1` | Planar texture repeats per input geometry scene unit |
| `impulseX/Y/Z` | finite doubles; `0` | Total directed impulse in world-scene axes, kg·scene-unit/second |
| `radialImpulse` | nonnegative decimal; `0` | Total scalar radial impulse, kg·scene-unit/second |
| `maxMemoryMiB` | integer 1–4096; `256` | Conservative component geometry/reconstruction allowance |

The evaluator distributes each total impulse by piece mass
fraction. Radial directions run from the source mass centroid to each piece
centroid, rotated into the source's world orientation; a coincident centroid has
zero radial contribution. These authored impulses may change total momentum.
The source's one `<rigidBody>` provides total mass and collision parameters.
**FRX1** gates version 1.3; **FRX2** requires one fracture and one rigid body on
an object3D other than a plane, map or volume; **FRX3** resolves the interior
material; **FRX4** rejects nonfinite numeric values; **FRX5** to **FRX7** belong to a
fracture from an impact (below). Runtime solid validation is
still required for the allowed primitive kinds and imported geometry. The five
`fracture`/`frx1`–`frx4` corpus fixtures agree with the independent schema oracle.
The NaN fixture additionally expects the existing Rust-only `W01` warning.

`sr_3d::fracture::fracture` partitions a closed triangle solid using seeded
plane cuts and constrained cap triangulation. It welds exact duplicate positions,
checks edge closure/orientation and intersecting source surfaces, and preserves
concavities and oriented inner cavities. Disconnected solid components become
separate physical pieces. The requested count is exact (1–4096); requesting
fewer pieces than the disconnected source components is an error. Both inward
and outward global winding work, but incorrectly oriented nested shells fail.
Each piece contains centroid-relative vertices, its original-space centroid,
positive volume, and the source mass distributed by volume. Exterior faces retain
source-triangle IDs for material/UV reconstruction; generated cap faces carry
an explicit interior marker. Numerical failures return errors without partial
output, including world-space volume or mass underflow.

`sr_3d::fracture::surface` reconstructs per-source exterior render batches and a
separate interior batch. Barycentric interpolation preserves primary and alternate
UV sets, separate map UVs and vertex colors inside each source triangle. Normals
are normalized and tangents are orthogonalized; triangle/material seams are not
averaged together. Source material IDs and variants survive. Cut faces receive
outward flat normals, white vertex color, and a canonical planar UV basis in the
input coordinate frame, shared by opposite caps with opposite tangent handedness.
Callers must freeze morph/skin animation and transform source positions, normals
and tangents into the kernel frame before invoking this API. Invalid provenance,
attribute arrays, degenerate faces, out-of-triangle reconstruction and unrepresentable
f32 output are errors. Source indexing and conservative temporary/output admission
are checked before output allocation; caller-owned inputs are outside the allowance.

The geometry API admits at most one million input vertices/triangles, uses a
normalized f64 frame, and defaults to a 256 MiB conservative component allocation
allowance and 100 million work units. Caller-owned input and total process RSS
are outside this estimate. These are cinematic geometric cuts; stress, fracture
toughness and impact-energy failure are not predicted.

`World3::with_fractures` registers source/fragment body indices before simulation.
Each body belongs to at most one partition; duplicate and chained ownership are
rejected. Fragment bodies must be dynamic, with positive finite masses that sum
to the source mass. Fragment offsets use source-local scene units; each impulse
uses world-scene axes in kg·scene-unit/second and acts at its fragment's centre
of mass. The aggregate piece mass properties define the source's mass and inertia.
For decomposed mesh pieces, inertia is integrated from the supplied closed
triangle surface rather than the approximate convex collision hulls. The caller
must supply a geometric partition; this registration API does not repeat the
geometry kernel's intersection tests.

Activation uses composition time and occurs at the first fixed-step boundary
whose timestamp reaches the authored time and whose source is enabled. It never
uses an epsilon to fire a future event early. A hidden source waits for its next
enabled boundary. Pieces replace the source collider, inherit its actual pose,
angular velocity and linear velocity at each fragment centre of mass, and receive
their impulses once. Source joints detach. Fragment visibility windows remain
independent; a hidden piece retains its release pose/motion until enabled. The
normal authored-body activation path cannot overwrite inherited fragment motion.
Event flags, retired sources and detached joints participate in checkpoint replay.
Release motion is validated for all due events before applying any of them.
`Frame3` reports body poses, centre-of-mass/angular velocities, participation and
fired-event flags for evaluator/render integration.

Geometry tests cover mass/volume/centroid conservation, concave holes, cavities,
disconnected inputs, source overlaps, deterministic seeds, many-piece closure,
scale changes and resource/numerical errors. Rigid tests cover timed replacement,
kinematic inheritance, linear/angular momentum, impulses, hidden windows, source
collision removal, joint detachment and backward replay. An independent analytic
tetrahedron checks the complete mesh inertia tensor and scene-unit conversion.
Surface tests independently recover affine exterior UV/color fields, exercise
multiple source materials and alternate UV sets, verify matching opposite-cap
UVs/outward normals, and reject invalid attributes/provenance/budgets. Schema tests
cover typed defaults, full seed precision, ownership, reference resolution, numeric
bounds and nonfinite values. XML tests cover activation, delayed visibility,
backward replay, mass-scaled impulses and baked/live equivalence. Required-GPU
checks exercise raster and path-traced exterior/interior appearance, imported
textures, globe map drapes and isolated-group replay.

Source preparation samples the first eligible fixed boundary at or after `at`
(respecting the source's ancestor clocks/windows), with a one-million-boundary
search allowance. Supported solid geometry includes boxes, spheres, cylinders,
cones, capsules, tori, path/text extrusions, clay, globe relief and imported triangle models.
Imported node animation, skinning, morphs and material variants are sampled at
release; typed `seed` retains all 64 bits. Crater deformation is applied before
cutting. Ordinary rigid object scale retains the existing physics-start contract.
Generated trigonometric seams are canonicalized at one part per million of source
extent and collapsed pole triangles removed; imported positions are not proximity
welded. Invalid/open geometry and exceeded component allowances are frame/bake
errors. These allowances do not measure total solver/importer/GPU peak memory.

A static or kinematic mesh-sequence source may own a fracture. Its collider samples
the numbered source geometry before release; its fragments freeze the selected
geometry at release and become dynamic. **MSQ4** continues to reject dynamic
mesh-sequence source bodies and direct pyro/particle references to sequence objects.

Physics bakes use **SRPHYS04**. Its little-endian header contains the eight-byte
magic, f64 step/start, u64 2D-body and soft-body counts, each soft point count,
u64 3D-body count (including fragments), u64 fracture-event count, u64 frame count,
a 32-byte SHA-256 identity of the document's physics, and a u64 contact count. Each
row holds the 2D poses, the soft lattices, seven f64 per 3D body (position and
quaternion), one f64 participation flag per 3D body and one f64 fired flag per
fracture event (flags must be exactly zero or one), and six f64 per 3D body for its
linear velocity (scene units per second) and angular velocity (degrees per second).
After the rows come the contacts the 3D rigid bodies resolved, in step order, each a
fixed 96-byte record: u64 step, two i32 body indices (the first is never a boundary,
`-1` is a boundary slab), point, normal, normal impulse and the relative velocity of
the second body before the step. A contact names a step the file holds, bodies it has,
and finite numbers. Only impacts are recorded: a pair of bodies whose contact points together push with
no more than twice the weight impulse of all dynamic bodies in one step is dropped at
the source, and a pair that is recorded keeps all its points. A pair that nothing solved
because both bodies were asleep or fixed is dropped too.

The identity covers the 2D and 3D world definitions (bodies, shapes, joints, gravity,
step, fractures; meshes by their numbers) and, for every baked step, what the document
feeds the world: which bodies take part, the poses of those that follow animation, the
force fields and the revision of every deforming surface. A version 4 cache whose
identity is not the document's, whose pinned `cacheSha256` does not match, or that
cannot be read is a document error: nothing is simulated in its place. Counts must match the compiled
scene. Geometry is reconstructed from immutable scene assets; the cache stores state,
not meshes. SRPHYS01 to SRPHYS03 remain readable and keep their behaviour: they carry
no identity, only the counts and the optional file hash are checked, and a mismatch
reports a problem and simulates. `Evaluator::physics_trace` returns the velocities and
contacts, from the verified cache or from a simulation, and the two agree exactly.
Baking samples exact physics boundaries without an added timestamp epsilon.

Particle and pyro collider references expand a released object into its enabled
pieces and stop using the retired source surface. The closed, centroid-relative
partition meshes already include object scale; world-space piece poses therefore
replace the original object's transform. Particle sweeps use the displacement and
quaternion difference between sampled poses. Smoke obstacles transform each piece
into domain coordinates and prescribe its translation/rotation boundary velocity,
including reflected or rotated domains. Both paths work with live and SRPHYS04
physics and deterministic backward seeks. Collider state is sampled on each
consumer's existing fixed-step clock; a release inside a consumer step is observed
at its next sampled boundary, so authors should align physics and consumer steps
when release timing matters.

Each consumer caches immutable fragment acceleration structures by geometry key.
All pieces and referenced objects share the unused portion of that consumer's
`meshMemoryMiB` allowance, after the original surfaces/source meshes are reserved
for backward playback. Absent/changed fracture snapshots discard stale caches.
The particle charge is `48*vertices + 512*triangles + 4096` per piece; the smoke
charge is `256*vertices + 768*triangles + 4096`. Construction errors propagate;
a failed future evaluation does not invalidate earlier replay states. These are
conservative geometry charges, not whole-process memory measurements. Integration
tests cover source removal, displaced-piece contact/occupancy and velocity,
live/baked equivalence, reverse replay, domain rotation/reflection and budget errors.
No additional XML attributes or validation-rule changes are needed for this wiring.

Text uses the renderer's shared `sr-text` shaping and outline extraction,
including multi-line layout, font family selection and glyph counters, followed by
centered extrusion with the object's evaluated height/depth/bevel. Local font
assets are resolved against their owning document; unreadable fonts fail source
preparation. An authored square-ring font fixture gives a deterministic empty-hole
oracle without depending on installed fonts. Font-file lengths, text length and
emitted outline points have admission checks; system-font discovery, font decoder
allocation and temporary path flattening are not a measured total-memory bound.

Clay freezes evaluated blob shapes/transforms, smooth unions/subtractions,
fingerprints and boil at the release frame. Renderer and fracture preparation
share the blob/finish sampling path, and authored seeds retain all 64 bits. The
bounded surface-nets API validates finite inputs, supports resolution through 256,
checks conservative scalar/index-grid plus worst-case output capacity before
allocation, and caps field work at 100 million blob evaluations (including the
normal-sampling allowance). Empty/open/invalid solids still fail fracture kernel
validation. The mesher emits crossed edges on each first grid plane; omitting
those faces previously left holes when the surface approached the grid boundary.
Tests verify closure across sphere sizes, bounded admission, release-time animated
blob sampling and raster/path-traced silhouette/hole preservation with replay.
Trigonometric seam canonicalization applies only to round generated primitives;
text, path and clay vertices keep their generated coordinates.

Ordinary text, path and clay solids also supply particle and smoke colliders,
including source removal and moving-piece replacement after fracture. Automatic
static/kinematic rigid bodies use triangle surfaces rather than filled box/convex
proxies, preserving holes before release; dynamic automatic bodies use convex
decomposition. Signed object scale and winding are retained in this path. Physics
discovery includes conditionally hidden bodies, while ordinary sampled frames gate
their participation, so a condition false at startup does not permanently omit a
body that later activates.

No new XML attributes are needed. Rust and Schematron expand P3D5/PYRO7 to include
text, extrude and clay. P3D6/PYRO8 reject animated procedural shape parameters,
animated blob inputs and nonzero fingerprint boil under the current static-proxy
contract. Rigid pose animation remains accepted. Positive numeric values with
leading plus signs/whitespace agree in both validators. Permanent corpus fixtures
`valid/solid-colliders.scene.xml`, `invalid/solid-colliders-boil.scene.xml` and
`invalid/solid-colliders-blob.scene.xml` cover the revised rules.

Remaining integration work includes broader animated-source collider parity,
source-window search and nonuniform parent-transform adversarial coverage. Final
native UHD impact sequence appearance, throughput and whole-process memory remain
unverified.

#### Fracture from an impact (`fracture@source`)

A fracture whose time and push are the consequence of an impact rather than authored. `source` names the
dynamic rigid-body object that breaks it; the owner is the body that breaks. Nothing in the document says when:
the body breaks at the first impact of the source on it, found by the rigid world as for a crater from an impact
(the same `ImpactWatch`, recorded in the world's state and in an SRPHYS04 cache, so a seek, a fresh world and a
baked cache give the same bits). The fracture fires on the step after the impact is noticed: the pieces
replace the body at the boundary that ends the step of the impact, with the velocity the body had after the
contact, never before it. `at`, `radialImpulse` and `impulseX`, `impulseY`, `impulseZ` are derived, so giving any of them
is **FRX6**; the source must be another object3D with a dynamic rigidBody (**FRX5**); `minImpulse` and
`energyFraction` belong only to a fracture with a source (**FRX7**). The rest of the declaration (`pieces`, `seed`,
`interiorMaterial`, `interiorUvScale`, `maxMemoryMiB`) keeps its meaning, and the pieces are the seeded partition of the
document.

| Attribute | Type; default | Contract |
|---|---|---|
| `source` | object3D ID; absent | The dynamic rigid body whose impact breaks the owner (FRX5) |
| `minImpulse` | positive decimal; twice the source's weight in one step | Total normal impulse of the pair in one step, kg·scene-unit/second, below which nothing breaks (FRX7) |
| `energyFraction` | decimal 0–1; engine value `0.3` | Part of the impact's relative kinetic energy that pushes the pieces apart (FRX7); no XSD default, so that a document that gives it without `source` is refused |

The default threshold is the crater's: a body resting on its target pushes with its weight, so it breaks nothing,
and an impact has to push with more than twice that in one step. A pair that rests, touches
gently or never meets breaks nothing and the owner stays whole for the whole composition.

The push is a modelled quantity, not a measured one. The relative kinetic energy of the impact is
`E_rel = 1/2 mu v_n^2`, with `mu = m_s m_o / (m_s + m_o)` the reduced mass of source and owner and `v_n` the closing
speed along the contact normal as the world noticed it. The fraction `f = energyFraction` of it, `E = f E_rel`, becomes
the kinetic energy the pieces gain relative to the owner's centre of mass. Each piece gets a speed along the unit line from the
owner's mass centre to its own, the same `s` for every piece; the mass-weighted mean of those velocities is removed so that the
push adds no linear momentum, and the angular momentum about the centre of mass is not changed by it. With `d_i`
the unit line of piece `i` and `dbar` the mass-weighted mean of the lines, `s = sqrt(2 E / sum_i m_i |d_i - dbar|^2)`.
The value `0.3` has no published source: it is the engine's, declared here so that a document without
it is defined, and a document that wants another says it. A piece whose centre is on the centre of mass has a zero line and
is given only the mean's removal like the others. The pieces keep the velocity of the body after the contact at their
own centres, so the momentum the contact solver left the owner passes through the fracture, and a spinning owner's pieces
go on turning with it.

Oracles, in the tests of `sr-sim` and `sr-eval` (`fracture_contact`, `impact_block`): the mass of the pieces is the mass of the owner;
total linear and angular momentum of the pieces equal those of the intact owner in the same world, at the instant
of the impact, to 1e-9; the kinetic energy gained is `E` (90.000000 against 90.000000 in the reference world, to a millionth
of it); nothing breaks below the threshold or without an impact; the pieces are the same in any order of
requests, from a fresh world, and from a baked cache. In the block scene
(`examples/cinematic-impact/impact-block.scene.xml`, the 90 478 kg rock at 100 m/s and 60 degrees against a
583 200 kg granite block on a slab, 12 pieces) the block lies still until the rock reaches it, 1.49 s in, and the
mean distance of its pieces from where the block stood, at 3 s (1.5 s after the contact), is 6.3, 10.5 and 14.5 m for rocks arriving at
60, 100 and 150 m/s, and 7.4, 10.5 and 11.4 m for rocks of 30, 90 and 270 tonnes (test `impact_block`, commit 2cf1fa2,
2026-10-06, deterministic). These are orders of magnitude and directions, not predictions of a real impact.

Limits, stated so that they are not mistaken for physics. The partition does not depend on where the impact was: the same
seeded cuts come out wherever the rock lands, and only the push (which depends on the speed and the masses, not on the point)
and the spin of the owner tell them apart. A piece does not break again (no second generation: the world rejects chained ownership, and the pieces are not
objects of the document). The source is not part of the partition and goes on
with the velocity the contact gave it; at the boundary the pieces appear where the owner was and a source that has penetrated
the owner by more than a step's travel may overlap pieces and be pushed out by the solver. The push adds `E` to the kinetic energy
that the contact has already treated: the total energy after the fracture is not more than before the impact only if `f` is at
most the share the contact dissipated (`1 - e^2` for a restitution `e` of the pair), which the engine does not check. The fracture
is the cinematic cut of this section, not a stress or toughness model.

#### Mesh sequences (implemented; importer hardening pending)

Version 1.3 adds `<meshSequence>` under `<assets>`. An `object3D` with
`primitive="mesh"` may reference it. Static mesh material overrides, skin/clip
sampling and the raster/path-traced rendering paths apply to the selected model.

| Attribute | Type; default | Contract |
|---|---|---|
| `id` | ID; required | Lexically scoped asset identity |
| `src` | numbered URI; required | `%d`, `%0Nd` or `####`, at most 64 digits; local/resolved at render time |
| `format` | optional enum | `gltf`, `glb`, `obj`, `ply`, `usd`, `usda`, `usdc`, `usdz`, `fbx`; absent selects by extension |
| `first`, `last` | signed 32-bit integers; required | Inclusive ordered labels; at most 1,000,000 frames |
| `fps` | positive decimal or rational; required | Cache frames per source second |
| `interpolation` | `hold` or `linear`; `hold` | Temporal sampling below |
| `missingFrame` | `error`, `hold`, `transparent`; `error` | Only absent files are missing; decode errors propagate |
| `maxMemoryMiB` | integer 1–4096; `256` | Each root input file and selected decoded models/blend; not importer peak RSS |

Time is `t = object.local_time * animationSpeed + animationOffset` with the
existing evaluated properties (defaults 1 and 0). Let
`q = clamp(t * fps, 0, last - first)`, `i = first + floor(q)`.
Hold selects `i`; linear samples `i` and `i+1` with fraction `q-floor(q)`.
Integral positions and endpoints load one frame. Negative time clamps to the
first label; time beyond the range clamps to the last. Included documents use
their own base directories and asset namespaces.

Hold permits topology changes. Linear requires identical node parent/primitive/
skin assignments, triangle indices, vertex counts, tangent handedness, UV/morph
layouts, material assignments/variants, basis, and material/texture/skin/clip
definitions. Display names are ignored. Positions, UVs, colors, weights and
morph offsets interpolate linearly; node rotations use quaternion slerp.
Normals and orthogonalized tangents are normalized. Incompatible layouts or
unresolved directions are errors, without silently falling back to hold.

Missing-frame hold searches backwards within the declared range and errors
without a predecessor. Transparent hold produces no geometry. Linear sampling
with one absent endpoint fades the available model by its temporal weight;
two absent endpoints produce no geometry. Corrupt or unsupported files always
report errors, including under hold and transparent policies.

Each compiled Program owns a FIFO cache of at most 64 frames / 256 MiB;
active snapshots may retain evicted frames. This allowance is separate from
`maxMemoryMiB`, which checks the selected model and, for distinct linear
endpoints, two copies of the first model plus the second before blending.
Each imported model must also fit individually. Files and dependencies are
immutable during a compiled Program's lifetime. Recompilation starts an empty
cache with new GPU revisions, including after external material edits.
Obsolete per-object GPU frame resources are evicted; vertex/index buffers and
texture dimensions are checked against device limits.

Models reject invalid triangle/node/texture/skin references, nonfinite vertex
attributes or transforms, cyclic hierarchies and more than 512 hierarchy levels,
independent of node order. Shared mesh-importer decoder and external-dependency
allocation bounds remain pending: root-file and retained-model checks do not
establish a peak-RSS limit or guarantee safety of arbitrary importer inputs.

XSD defines types/defaults. Schematron and Rust share **MSQ1** (version 1.3),
**MSQ2** (ordered bounded labels and numbered pattern), **MSQ3** (reject one
`sha256` for numbered files), and **MSQ4** (changing render geometry cannot
silently serve as static collision geometry). A mesh-sequence object can own
a static or kinematic rigid body when it also owns a fracture; the evaluator
updates that source collider and freezes fragment geometry at release. Dynamic
source bodies and direct pyro/particle collider references still require an
explicit proxy. Other mesh consumers still require ordinary mesh assets.

Asset verification expands the declared files and honors missing-frame policies.
The glTF importer rejects sparse accessor counts beyond the parent accessor and
sparse indices that repeat, descend or exceed the accessor's element range,
before geometry decoding. All three unsigned index widths are covered by invalid
input tests and a valid sparse-geometry oracle. These are glTF input invariants,
not new XML attributes or a complete importer allocation guarantee.

Incremental/watch dependency discovery includes numbered mesh/volume files and
mesh dependencies exposed by the shared importer. Complete dependency discovery
for all formats remains part of importer hardening. The valid fixture is
`tests/corpus/valid/mesh-sequence.scene.xml`, using synthetic triangle/square
frames `tests/corpus/media/mesh-frame-{0,1}.obj`.

### UHD path tracing

**No schema attribute is needed to enable tiling.** The existing camera's
`renderer`, `pathSamples`, `maxBounces` and `denoise` retain their meanings.
Internal tiles preserve global pixel/sample seeds and the original camera
projection. Filter halos cover every denoising pass; tile edges must not become
image edges. Peak image working-buffer allocation is bounded independently of
frame area. Geometry/texture limits remain separately validated and reported.

**Render statistics and the probe.** The statistics of a render say where a
frame's time goes (commit 7e6fcb3, 2026-10-04): `sim_rigid_seconds`,
`sim_ocean_seconds`, `sim_smoke_seconds` and `sim_particles_seconds`,
`draw_prep_seconds`, `volume_prep_seconds` and, for the path tracer,
`pt_assemble_seconds`, `pt_bvh_seconds` and `pt_pack_seconds`. With `--stats` the
passes are timed by GPU timestamp queries, the path tracer's under `pathtrace trace`
and `pathtrace denoise` in `gpu.passes` (absent on an adapter without them, and the
timing costs a little itself). All of it is additive and changes no pixel (test
`crates/sr-gpu/tests/stage_times.rs`). `tools/probe_render.py` (commit ca85e5e,
2026-10-04) renders single frames of a scene, optionally at another size (a copy of
the scene is rendered; the scene's own file is never edited), through the shared GPU
queue, and writes a JSON report: the stage seconds, the GPU milliseconds by pass,
the wall seconds of the render process alone (queue wait excluded), its peak resident
memory and the sha256 of the frame, which tells two builds' pixels apart. A failed
render or a missing statistics line is an error, never a partial result; the report
logic has unit tests (`tools/tests/test_probe_render.py`). What the probe reports is
a measurement on one machine at one load, not a promise (ledger: "Render stage
timings, render probe and reflected-volume tests").

### Light through water and glass (path tracer)

Materials with `transmission` above 0 refract in the path tracer. Three
behaviours apply to them, none needing a schema attribute:

- **Dome behind glass.** A path that crosses a refracting surface and then
  reaches nothing, or reflects off it up to the sky, sees the 2D layers behind
  the 3D pass and, where those layers leave the pixel open, the visible dome
  (layer colour plus one minus its alpha times the dome). A dome that is not
  visible stays hidden.
- **Absorption.** `attenuationColor` and `attenuationDistance`, which the
  rasteriser also uses, apply to path-traced paths: a path that refracts into
  a surface carries the Beer–Lambert coefficient `-ln(colour) / distance` per
  channel until it leaves, so after one attenuation distance of water the light
  left is the colour itself. A colour without a distance, or a white colour, does
  not absorb. Absorption is tracked only in scenes that have a transmissive
  material; other scenes keep their pipeline, pixels and speed.
- **Analytic lights below a refracting surface.** The shadow ray of a surface
  seen from inside the denser medium (a path that refracted in before reaching
  it) does not stop at the interface. It is refracted there, found by trace and
  refined once, so that it leaves toward the light, and the contribution carries
  the blockers on both sides, the interface's tint, transmission and Fresnel
  transmittance, the factor `cos(theta_air) / eta^2` that goes with the solid
  angle the interface changes (radiance is not scaled by `1 / eta^2` at
  refraction in this renderer, so this factor is what makes the sum agree with
  brute force) and the absorption along the water path. Shadows cast by glass on
  surfaces in air are black. The dome reaches submerged surfaces by sampled paths
  and needs no such term.

At commit c6195de scenes without a transmissive material rendered the same bytes
at the same speed; the specular sampling below (commit 06d6596) changes every
path-traced frame by noise, not by a bias. The ocean's default spray
(transmission .6) is a transmissive material, so frames with whitewater are lit by
this path.

#### Specular sampling at grazing views

The specular lobe of a surface draws its half vector among the GGX normals visible
from the view (Heitz 2018), with the density `G1(v) D(h) / (4 n.v)` of the light
direction, and the share of samples that go to the specular lobe is capped at
0.999 (it was 0.95, with half vectors drawn from `D(h) cos`). Visible normals
because a grazing view sent many drawn normals' reflections below the surface,
where the path ends, and a dark glossy surface at the horizon (the `farSea` plane
of the ocean scenes) showed dark specks at 8 samples per pixel; the cap because a
dark dielectric's diffuse lobe is a thousandth of its reflectance there and was
given one sample in twenty. Sampling visible normals and the cap together: in the
ocean acceptance scene, in a band of 160 x 50 pixels along the horizon, the pixels
more than 40 levels under the median were 517 at 8 samples without a denoiser and
217 with the scene's, against 15 at 64 samples; they are 5 and 1 (0 at 64 samples),
and the band's mean at 8 samples, 228.39, agrees with 228.87 at 64. The lobe stays
unbiased: a quadrature of the BRDF times the cosine over the hemisphere, with no
sampling, agrees with the picture of a plane under a uniform dome within 0.4 % for
roughness 0.06, 0.25 and 0.5 at seven view angles from the horizon down, and
dropping the masking term from the density fails it (test
`crates/sr-gpu/tests/horizon_specks.rs`; ledger: "Specular sampling of the visible
normals").

Measured against the build before it (commit c7dba89), NVIDIA adapter, 2026-10-06,
the seven frames of the path-traced identity set and the hero frame at 1280 x 720,
t = 3.0:

| frame | pixels differing of 921600 | largest difference (levels) | mean difference (levels) | image mean before, after | trace seconds before, after | sha256 after (first 16 digits) |
|---|---|---|---|---|---|---|
| plume, both lights | 430552 | 201 | 0.88 | 168.47, 168.41 | 4.62, 4.52 | c5756924eb0fe609 |
| plume, dome only | 442240 | 203 | 1.01 | 164.57, 164.49 | 3.76, 3.69 | 0acc6a3d7b7dc71d |
| plume, sun only | 172555 | 1 | 0.19 | 71.222, 71.223 | 1.00, 0.96 | 524f6d0950558c71 |
| dry floor, both lights | 732979 | 22 | 1.05 | 182.550, 182.554 | 0.12, 0.12 | be3f8c9a1617811c |
| dry floor, sun only | 77875 | 23 | 0.16 | 144.672, 144.674 | 0.12, 0.12 | 3e3d13ac3bfedb59 |
| dry floor, dome only | 839336 | 22 | 1.58 | 116.779, 116.787 | 0.09, 0.09 | 36c2fec7b1634588 |
| glass ball in air | 101364 | 53 | 0.31 | 138.230, 138.233 | 0.34, 0.33 | 1235e3373a4501e4 |
| hero | 447584 | 201 | 1.03 | 166.23, 166.17 | 10.22, 10.14 | 24c6ab56703017611dd410948af73798116f35dfbdc0a232639cecd5e5a9b72e |

Every mean moves by at most 0.04 %. The sha256 of the hero frame at 1280 x 720,
t = 3.0 is, from commit 06d6596 (2026-10-06), `24c6ab56703017611dd410948af73798116f35dfbdc0a232639cecd5e5a9b72e`;
it was `953a7b382df31fd6c2bf49e646c9abb9dec4978807ff95e6df4b92328e5eaea1`.

Agreement with brute force (the light replaced by an emissive copy that paths
find, 1024 samples, flat water 4 units over a diffuse floor, 320×180; commit
c6195de, NVIDIA adapter, 2026-10-05; the renders are deterministic, so the figures
do not depend on load), as the open floor's brightness under water over its
brightness dry: sun 0.604 against 0.613; point light 0.728 and sphere light 0.728
against 0.770 (the light's radiance is taken at the straight distance, so a near
light is a few percent dim). Closed form of a vertical sun over absorbing water:
within 1 % per channel. A submerged box's shadow is 10.0 units long where the
refracted direction gives 9.9 and the straight one 15.1. Under steep waves
(amplitude 1.2, wavelength 10) the floor is never brighter than the dry floor and
stays within 5 % of the flat sea's mean, with no flare (tests in
`crates/sr-gpu/tests/water_light.rs`; ledger: "Light through water and glass in
the path tracer").

Dome light needs no term of its own below the surface: with 4 bounces a floor
under water agrees with 24 bounces within 3 % (test
`dome_light_through_water_does_not_depend_on_the_bounce_limit`, commit c6195de,
2026-10-05, NVIDIA).

On a software adapter the brute-force comparisons run only when asked (`SR_BRUTE_FORCE=1`; commit 487cf35, 2026-10-06,
llvmpipe: the two comparisons take 191 s and 428 s, the whole test file 913 s); when asked they render half-size frames
(the same view, the patch scaled with them) at 1024 samples, with the tolerance the
test states or four times the noise of the references' means when that is larger
(commit d9aeb32, 2026-10-05, llvmpipe at a load average of 15: the three tests take
370 s for the three tests, 597 s for the whole test binary instead of 2018 s). The half-size reference of the point and sphere
lights reads about 5 % above the full-size one (0.810 against 0.770), a bias of the
half-size frame that the 15 % margin absorbs: the measured errors are 2.1 % (sun)
and 10.0 % and 10.1 % (point and sphere); `SR_BRUTE_FORCE=full` forces the full size
on any adapter and nothing reduces them on a GPU.

Known limits. A camera that starts under the water is placed inside the medium
by a probe ray straight up (the scene's up is -y): the first transmissive surface
it meets, seen from the inside, sets the medium, and a floor or wall the camera
sees is lit by the sun through that surface (test
`a_camera_under_the_water_sees_a_wall_lit_as_the_oracle_says`, `water_oracle.rs`).
Where the probe finds no surface, or one that is not the boundary of the water the
camera is in (a sloped bank, a lid, an overturning wave), the camera is treated as
in air and the sun does not reach what it sees, as it does not reach a surface
that a path reached without refracting into its medium. The shadow ray follows the first transmissive interface;
a second interface along it (an overturning wave, layered media) is not
followed, and where the refinement finds no way out toward the light (a steep
wave, grazing light) the light contributes nothing there: those samples are
dark, never bright. A textured transmissive surface uses its uniform base
colour for the tint in the shadow ray. A smoke medium and refracting surfaces
in one pass are lit independently: the shadow ray does not cross the medium.
Caustics (light focused by the water surface) are not produced; flat water is
the exact single-refraction case, a wavy sea an approximation. The sun that
crosses a wavy surface is resolved by one refracted shadow ray per sample; the
focusing of the waves is not modelled. Under steep waves a floor under water
receives 0.581 of the light of the same floor dry by the analytic shadow ray,
against 0.631 by brute force with an emissive sphere of 1024 samples (a gap of
0.05, 8 %), and a point reached by two facets takes one facet per sample. Closing
the gap needs the Jacobian of the refraction from the surface to the floor, or
light tracing, and waits for an acceptance shot that shows the lit floor under
waves. A transmissive
plane that covers the whole sea (the `farSea` plane of `impact-ocean.scene.xml`, when its
water material is transmissive)
tints and hides everything below it as well.

Two software-adapter tests are unstable on both commits measured. On 2026-10-05
(llvmpipe, load average 7 in the interleaved set, 3 to 6 in the others; commits
60a458c and b883ec5; raw logs in the ledger), `effect_costs`
`layers_whose_content_changes_are_still_sampled_one_by_one` failed 4 of 10 runs on
60a458c and 2 of 10 on b883ec5 (PSNR 30.7 to 39.1 dB against 40), and 1 of 15 and 0
of 15 with one llvmpipe thread (`LP_NUM_THREADS=1`); `composite`
`polygons_and_stars_lie_on_the_inscribed_ellipse` failed 2 of 50 and 4 of 50 runs with
the default threads and 0 of 80 with one llvmpipe thread. The tests measure image quality and pixel
values, not time; the difference between the two commits is not significant, and the
conclusion holds for these two commits only.

### Schwarzschild black hole (`blackHole`, `accretionDisk`, `camera@geodesics`)

A scene with a non-rotating black hole in vacuum, a thin disk of gas around it and a camera that follows the paths of
light through its spacetime. Version 1.3. This version draws the hole, the disk and the sky and nothing else.

**Units.** G = c = 1 and lengths are scene units, so the mass `M` (`blackHole@mass`) is a length and the horizon is the
sphere `r_s = 2M` in the Schwarzschild radial coordinate `r`, centred on the hole's `x y z`. Times are lengths: a
second of the scene is `timeScale` units of that time for the disk (`accretionDisk@timeScale`, default 1).

**Elements.** `blackHole` has `mass` and a place. `accretionDisk` names its hole (`blackHole`), has `innerRadius` (6M when
absent), `outerRadius`, `temperatureScale` (kelvin), a `seed`, an `angularPattern` (`none`, `clumps`, `spiral`), its
`contrast`, `intensity` and `timeScale`, and the tilt of its axis by `rotationX`, `rotationY` and `rotation` (about z)
in degrees, composed as an object3D composes them, Rz Ry Rx. It has no place of its own: it is centred on the hole.
`camera@geodesics` (default false) makes the camera trace null geodesics; its position is the position of a static
observer at the radius `d` = its distance to the hole, which is outside the photon sphere. `fov`, `roll`, `target`, `yaw`
and `pitch` are those of any camera, and `pathSamples` is the number of antialiasing samples of a pixel.

**Rules.** BH1: the version is 1.3. BH2: one `blackHole` per scene. BH3: an `accretionDisk` names a `blackHole`. BH4: the
inner radius is at least 6M, the radius of the innermost stable circular orbit, and the outer one is beyond the inner.
BH5: a camera with `geodesics="true"` needs a hole. BH6: with it the scene has no `object3D`, `particles3D`,
`particleEmitter`, `ocean`, `fluid`, `flock`, `slime`, `erosion`, `pyro` or `medium`: it is an error and not a silence
(2D layers, text, shapes, effects and adjustments are allowed: the image goes through the 2D chain). BH7: the camera is
farther than 3M from the hole (only its authored `x y z` is checked). BH8: one such camera per scene. W03: a hole or a disk
and no geodesic camera: they are not drawn. W04: `denoise="true"` on a geodesic camera, which does not denoise. W05:
lights in the scene, which are not used. The corpus holds one document for each rule and warning (tests/corpus,
`bh1-version` to `bh8-two-cameras`, `w03-no-lens` to `w05-lights`).

**Null geodesics.** The metric is `ds^2 = -(1 - 2M/r) dt^2 + dr^2 / (1 - 2M/r) + r^2 dOmega^2`. A light ray moves in a
plane through the hole. With `u = 1/r`, `phi` the angle in that plane and `b = L/E` the impact parameter of the ray (its
angular momentum over its energy), Binet's equation for light is `u'' + u = 3 M u^2` and its first integral is
`(du/dphi)^2 = 1/b^2 - u^2 + 2 M u^3`. The photon sphere is `r = 3M` (`u = 1/(3M)`), and the critical impact parameter
`b_c = sqrt(27) M = 3 sqrt(3) M`: a ray with `b < b_c` falls into the hole, one with `b > b_c` is deflected and escapes.
The ray of a static observer at radius `d` that makes the angle `psi` with the direction to the hole has
`b = d sin(psi) / sqrt(1 - 2M/d)`, so the shadow of the hole has the angular radius `psi_sh = asin(b_c sqrt(1 - 2M/d) / d)`
(for `d` far from the hole, `b_c / d`). The total deflection of a ray that escapes is
`alpha(b) = 2 integral from 0 to u_0 of du / sqrt(1/b^2 - u^2 + 2 M u^3) - pi`, `u_0` the smallest positive root of the
radicand: `4M/b + (15 pi/4)(M/b)^2 + (128/3)(M/b)^3 + ...` for large `b`, and
`-ln(b/b_c - 1) + ln(216 (7 - 4 sqrt(3))) - pi` as `b` approaches `b_c`.

**The disk.** The gas is on circular geodesics in the plane through the hole perpendicular to the axis of the disk, which is
(0, -1, 0), the scene's up, turned by the disk's rotations. A circular orbit of radius `r` is stable for `r >= 6M`, its
angular velocity is `Omega = sqrt(M / r^3)` (about the axis, anticlockwise seen from the tip of the axis, times `timeScale`
for the scene's time) and the time dilation of the gas relative to a static observer at infinity is
`u^t = 1 / sqrt(1 - 3M/r)`. A photon that reaches the camera from a point of the disk carries `lambda = L_z / E`, its angular
momentum about the axis over its energy, and the ratio of the energy the camera receives to the one the gas emits is
`g = sqrt(1 - 3M/r) / (1 - Omega lambda)`, with `lambda = b (n . z)`, `n` the unit normal of the plane of the photon's orbit
oriented by its motion from the gas to the camera and `z` the axis. For weak fields `g = 1 / (1 + v . d)` with `d` the
direction of the camera ray going out from the camera and `v = Omega r (z x r^)`: the side that moves toward the camera has
`g > 1` and is bluer and brighter. The renderer traces from the camera and so reverses the orientation of the path; the
formula in terms of `lambda` is the one that holds. In Luminet's form (Luminet 1979, Astron. Astrophys. 75, 228, not read in preparing
this text) `1 + z = (1 - 3M/r)^(-1/2) (1 + Omega b sin(theta_0) sin(alpha))` with `theta_0` the inclination of the
observer against the axis and `alpha` the angle of the pixel, whose sign is that of `-lambda` there. The observed bolometric
intensity is `g^4` times the emitted one, and the colour is a black body at `g T`, which already carries the `g^3` of
the spectral intensity.

The temperature of the gas is the profile of a thin disk with no torque at its inner edge,
`T(r) = temperatureScale * f(r) / f(49/36 r_in)`, `f(r) = (r_in/r)^(3/4) (1 - (r_in/r)^(1/2))^(1/4)` (Shakura and Sunyaev 1973,
Astron. Astrophys. 24, 337, not read either, in its Newtonian form with the inner edge at `r_in`, `f(49/36 r_in) = 0.48787`): `temperatureScale` is the
temperature at the maximum, `r = 49/36 r_in`, before the shift of light. The relativistic factors of Novikov and Thorne are not
applied. The disk is opaque: a ray ends at the first point of the disk it meets, after at most the number of crossings the
renderer follows. The azimuthal pattern is a seeded function of the angle and of `Omega(r) * time`, so that it turns
differentially; `none` gives a smooth disk. The sky at infinity is what the rays that escape see.

**Acceptance.** A renderer of this scene is accepted by the physical quantities below, not by a reference image. The numbers
were computed on 2026-10-06 with a Gauss-Legendre quadrature of 400 points of the integral above, with `M = 1`; they were not
measured on the renderer.
- The shadow has the radius `b_c = 5.19615 M` for a distant observer: `psi_sh` is 0.48336 rad at `d = 10 M`, 0.10200 rad
  at `50 M` and 0.0051910 rad at `1000 M`.
- The deflection of a ray is `0.590396` rad at `b = 10 M`, `0.236136` at `20 M`, `0.0850835` at `50 M`, `0.0412225` at
  `100 M` and `0.00401182` at `1000 M`; the weak-field series with the three terms above agrees to 4.4e-5 at `b = 100 M` and
  to 4.3e-8 at `1000 M`, and the strong-deflection form to 4e-6 at `b = b_c (1 + 1e-6)`, where it is 13.4153 rad.
- The ratio of energies is `g = sqrt(1 - 3M/r)`, `0.70711` at `r = 6M`, for a ray with `lambda = 0` (a disk seen face on) and
  `g -> 1` for large `r`; a disk seen at an angle is brighter and bluer on the side that approaches, where `lambda` has
  the sign that makes `1 - Omega lambda` smaller than 1.
- The photon ring, the light that goes round the hole before it reaches the camera, is a family of images whose size
  approaches `b_c` and whose width falls by the factor `e^pi` between one and the next.
A renderer that does not meet them for the cases it can reach, within the error of its integration, is wrong; the
integrator step and the number of steps are the renderer's and are written in its own section.

**Reference implementation (`sr_sim::gr`).** The engine carries this physics as a CPU reference in double precision,
`crates/sr-sim/src/gr.rs`, that the renderer is compared with and that no render calls. Lengths are the scene's and `G = c = 1`.

*The integration.* Binet's equation is integrated in the angle `phi` with the classical fourth-order Runge-Kutta method on
`(u, w = du/dphi)`, `w' = -u + 3 M u^2`, with a fixed step of 0.02 rad (`STEP`) and at most 4096 steps (`MAX_STEPS`), after which
a ray that has neither been captured nor escaped is taken to be captured (it is circling the photon sphere). The ray starts at
the observer's radius `r_o` with `u = 1/r_o` and `w = +-sqrt(max(1/b^2 - u^2 + 2 M u^3, 0))`, positive for a ray that goes in. It
is captured when `u >= 1/(2M)` and has escaped when `u <= 0`, at the angle `phi_inf = phi + h u / (u - u_new)` of a linear
interpolation across the last step. The planes of the disk are crossed at `phi0 + k pi`, `k < 4`, and the step that would pass one
is shortened to land on it. The arithmetic is written in one order, so that a shader can do the same sums: the acceleration is
`-u + 3 * M * u * u` taken left to right, the stages come in the order `k1` to `k4`, the half step `0.5 * h` is taken once and the
result is `x + (h / 6) * (k1 + 2 k2 + 2 k3 + k4)`. The same code runs in single precision (`gr::f32`).

*The camera and the ray of a pixel* (`gr::image`, the convention that the shader copies). The camera has the unit vectors `fwd`,
`right` and `down` (`right x down = fwd`), a focal length `f` in pixels and the size `w x h`; the ray of pixel `(x, y)` is
`d = normalize(fwd f + right (x + 0.5 - w/2) + down (y + 0.5 - h/2))`. With `n` the unit vector from the hole to the observer, `cos a
= -d . n`, `perp = d + n cos a` and `sin a = |perp|`, the plane of the ray has the basis `e1 = n` and `e2 = perp / sin a` (any unit
vector orthogonal to `n` when `sin a < 1e-6`), `phi` grows from `e1` toward `e2` along the path traced from the camera outward, the
impact parameter is `b = max(r_o sin a / sqrt(1 - 2M/r_o), 1e-5)` and the ray goes in when `cos a > 0`. The plane of the disk, with the
unit axes `dx`, `dy` (`dx x dy = dz`) and `dz` the axis of spin, is first met by the plane of the ray at `phi0` in `(0, pi]`:
`phi0 = atan2(-e1 . dz, e2 . dz)`, plus `pi` if it is negative and plus `pi` again if it is below 1e-6; when `(e1 . dz)^2 + (e2 . dz)^2 <
1e-12` the planes coincide and there is none. A pixel takes the first of the four crossings whose radius is within the disk, and then
the azimuth `psi = atan2(p . dy, p . dx)` of `p = cos(phi_k) e1 + sin(phi_k) e2`, and the redshift
`g = sqrt(1 - 3M/r) / (1 + Omega b h)` with `Omega = sqrt(M / r^3)` and `h = (e1 x e2) . dz`: it is the `g` of the paragraph on the disk, for
`lambda = -b h`, because `e1 x e2` is the normal of the plane of the orbit oriented against the photon's motion. A ray that escapes ends
in the direction `cos(phi_inf) e1 + sin(phi_inf) e2`. A difference of convention between a renderer and this reference shows as a
difference of image and not as physics, which is why it is written here.

*The closed forms* (`gr::oracle`), none run per pixel: the shadow `b_c = sqrt(27) M`, the horizon `2M`, the photon sphere `3M`, the
last stable orbit `6M`, `Omega = sqrt(M/r^3)`, the smallest positive root `u_0` of `1/b^2 - u^2 + 2 M u^3` by bisection on `(0, 1/(3M))`, the
exact deflection `2 integral_0^{u_0} du / sqrt(1/b^2 - u^2 + 2 M u^3) - pi` evaluated by Romberg quadrature of the form with `u = u_0 (1 - s^2)`, in
which the radicand is `(u_0 - u) Q(u)`, `Q(u) = (1 - 2 M u_0)(u + u_0) - 2 M u^2`, and the integrand `2 sqrt(u_0) / sqrt(Q)` is smooth on `[0, 1]`,
the weak-field series `4x + (15 pi/4) x^2 + (128/3) x^3 + (3465 pi/64) x^4`, `x = M/b`, Luminet's redshift
`g = sqrt(1 - 3M/r) / (1 + Omega b sin(i) sin(alpha))` and the temperature of Shakura and Sunyaev `r^(-3/4) (1 - sqrt(r_in/r))^(1/4)` times a scale, zero
at and inside `r_in`, with its maximum at `49/36 r_in`. The references are Luminet 1979 (Astron. Astrophys. 75, 228), Shakura and Sunyaev 1973
(Astron. Astrophys. 24, 337), Darwin 1959 for the exact deflection as an elliptic integral and Chandrasekhar 1983 (The Mathematical Theory of Black
Holes) for the same; the series coefficients are those of Keeton and Petters 2005 (Phys. Rev. D 72, 104006). None of them was read in preparing this text:
they are cited from memory, the closed forms that need them are derived above, and what is checked is the quadrature of the integral and the
integration against each other, which do not share code.

*What was measured* (tests `gr` and `gr_image` of `sr-sim`, commits 5e33e9d, bc4e7d1 and c7fcc13, 2026-10-06; the results are deterministic and do not
depend on the load). The impact parameter that separates capture from escape, found by bisection for a ray from `1e9 M`, is within a relative
`9.7e-8`, `5.7e-9`, `3.5e-10` and `2.1e-11` of `sqrt(27) M` at the steps 0.08, 0.04, 0.02 and 0.01. The integrated deflection for a ray from `1e9 M` with
the step `2e-4` is `0.590395778`, `0.236135975` and `0.041222440` at `b = 10`, `20` and `100 M` against `0.590395788`, `0.236135995` and `0.041222540` by quadrature (the
`1e-7` at `100 M` is the part of the ray before `1e9 M`). At the step of the shader the angle at infinity is within `1.3e-7` of the quadrature for the impact
parameters 5.3, 5.5, 6, 8, 10, 20 and 100 M (`7.2e-8` at 10 M), and the error at 10 M falls from `9.1e-7` to `7.2e-8` to `6.1e-9` as the step goes 0.04, 0.02, 0.01.
The weak-field series leaves a residual of about `700 (M/b)^5`. Single against double precision, for four rays: at most `9.4e-7` relative in the angle at
infinity and `1.4e-5` in the radius of a crossing (test `single_precision_follows_double_precision`). The image: on 256 x 160 pixels, with an equatorial camera at
`40 M` and a focal length of 100 pixels, the shadow has the radius `12.915` px by area against `12.764` px from `b_c` (limit 0.5 px); the two sides of an
inclined image have the same classes and radii, and `1/g + 1/g'` averages to `1/sqrt(1 - 3M/r)` to `1e-9`; in the column of the plane through the axis `g =
sqrt(1 - 3M/r)` to `1e-12`; a 1280 x 720 image takes about 2.6 s of one core. The comparison of the shader with this reference, pixel by pixel, is the
test of the renderer (`sr-gpu`, commits e4b1f85 for the tests and 99f4d2e for the shader, on the NVIDIA adapter, 2026-10-06, as its author reported it and not
rerun in preparing this text). On 96 x 60 pixels the shader in single precision and this image agree on the class of 5760 of 5760 pixels (106 captured, 5084 of
background, 570 of disk); the worst relative differences are 1.2e-5 in the angle of escape, 8.1e-6 in the radius of a point of the disk, 3.1e-6 in `g` and 1.3e-6 rad in
the azimuth. Against `gr::f32::trace`, for the rays more than 5% from the critical curve, 2528 of 3186 radii of crossings agree to 1e-6 or better and the worst is 1.2e-5
up to `56 M` (6.8e-5 beyond it, near the escape) and 1.4e-6 in the angle of escape: the GPU compiler contracts operations, so the same order of operations does not
give the same single-precision bits. The shadow of an observer at `1e4 M` is 39.99 px for the 40 expected.

**Limits.** The hole is Schwarzschild: no rotation, no charge, no frame dragging. The disk is analytic, geometrically thin,
opaque and in steady circular motion: it has no vertical structure, no self-irradiation, no radial flow and no
relativistic emissivity profile beyond the one above, and it is not made of particles. The scene has no other 3D object
and no medium (BH6), so there is no light from or through anything but the disk. One hole and one geodesic camera. The
observer is static at a finite radius `d`; a camera that moves or an observer in free fall is not modelled. The reference integrates a
ray in its own plane with a fixed step of 0.02 rad, so a ray that circles the photon sphere for more than 82 rad is taken to be captured, the exact
deflection loses accuracy for `b` within a few per cent of `b_c` (the integral diverges there), `g` exists for `r > 3M` only, and the time of an
image turns the pattern of the disk and nothing else: `r`, `psi` and `g` do not depend on it. None of the
formulas of this section was checked against the papers it cites: they are derived in the text, the Schwarzschild
quantities (Binet's equation, `b_c`, the weak-field deflection `4M/b`, `g`) are standard, and the numerical figures above
are the check of the integrals. The inner edge `r_in = 6M` and the temperature of the disk are the engine's choices and not a
fit to any observation.

### Voxel assets and objects (`voxelAsset`, `primitive="voxels"`)

A model of cells, each with a palette index 1 to 255, is declared once as an asset and used by any number of objects. It is the
body of cells of `sr_3d::occupancy` (sparse bricks of side eight, exact moments, connected components) that the rigid world already
reads, so what an object looks like and what it collides as are the same cells.

```xml
<assets>
  <voxelAsset id="castle" src="castle.vox" sha256="DIGEST" license="MIT" maxCells="2000000"/>
  <mesh id="rock" src="rock.glb"/>
  <voxelAsset id="rock-cells" fromMesh="rock" cellSize="5"/>
</assets>
<composition>
  <object3D id="keep" primitive="voxels" voxels="castle" cellSize="2" palette="file" surface="blocks"/>
</composition>
```

In the example the mesh `rock` is a glb of a rock a metre across, 100 scene units, so cells of 5 units are 20 to a side and about
8,000 cells (cells of a quarter of a unit would be 400 to a side, 64 million, far over the default `maxCells`).

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

**A mesh cut into cells (`fromMesh`).** The mesh is taken in the frame it is drawn in: the vertices as the renderer places them (the
basis of the import times the node's transform), in scene units (an asset in metres is 100 to the metre) and scene axes (y and z turned
about x), and `cellSize` is in scene units; a cube of one metre in a glb, cut at 10, is 1000 cells, x 0 to 9, y -10 to -1 and z -10 to -1
(`sr_eval::voxel::from_model`, test `a_mesh_asset_is_cut_in_the_frame_it_is_drawn_in...`). The lattice is aligned to multiples of `cellSize` in those coordinates, and a
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
   is located at floor(size.xyz / 2)", `ogt_vox.h`, line 125), by `p = R q + t`, where `R` comes from the rotation byte of the `nTRN` and a voxel is a unit box (the pivot is a corner of the
   grid, every face is on an integer coordinate, the pivot is subtracted from the geometry and the transform then applied to it:
   `ogt_vox.h`, "EXPLANATION OF MODEL PIVOTS", lines 123 to 170), so that a negated axis sends the cell `q` to `-q - 1`, not
   `-q` (a model of 3 x 3 x 3, the cell (0, 0, 0), the byte 105 and the translation (5, 6, 7) give the cell (4, 6, 7), worked
   out by hand in the test and not by the script that writes the fixtures):
   bits 0 and 1 are the column of the nonzero entry of the first row, bits 2 and 3 those of the second (the third is the column that
   is left), bits 4, 5 and 6 the signs of the three rows (section (c) of the extension file of the same repository, whose example
   `R = [[0, 1, 0], [0, 0, -1], [-1, 0, 0]]` is the byte 105 and is the fixture `spec-rotation`). Where the cells of two models fall
   on one place the later one in the graph wins.

*What is evidence and what is proof.* Fact 4 is proved by the description, by `ogt_vox.h` and by the fixtures that `tools/make_vox.py`
writes (an independent script that shares no code with the reader and works with the centres of the boxes as exact fractions: 24 rotations,
nesting, several models), and by cells worked out by hand in a test, not by a real file: none of the real files has a rotation (`axes.vox`
has translations only), so the convention of the negated axis is the reference's and is not checked against a file that MagicaVoxel wrote. The layout of `axes.vox` of the `dot_vox` crate, with its cube on the plane z = 0 and about x = y = 0, fits
the centre `floor(size / 2)` and is the evidence of it, not a proof. The real files are in `crates/sr-3d/tests/fixtures/vox/real`, with
the licence texts and the sources (`SOURCES.md`); the test that reads all thirteen sample files is run with `VOX_SAMPLES` set and
reads nothing, rather than failing, when the files are not there.

**The loader (`sr_eval::voxel_asset::load`).** The asset key resolves as a mesh asset's does (a local file; a remote scheme is an error that
says so), and a program knows the voxel assets that its objects of primitive `voxels` name (and the mesh asset of a `fromMesh`). The bytes
are read bounded by `maxMemoryMiB` (default 128), hashed, and the declared `sha256` is checked on the bytes that were read before
anything is parsed, so a file that is not the one the document names is refused as that and not as a malformed `.vox` (a `voxelAsset`
with `fromMesh` has no digest of its own: the digest that the mesh asset declares is checked against its file). `maxCells` (default 4,194,304) is the limit of the grid; over either
limit is an error that names the number. The cells are moved so that the minimum corner of the box of the occupied cells is the origin
(the object's origin, as the XSD says), and `VoxelModel::origin_cells` is the minimum key before the move, in the lattice of the scene
after the scene graph: the cells of the file are the cells of the model plus it, and the pivot of a model of the file, `floor(size / 2)`,
can be worked out again from it. A model has the materials of the file as numbers by palette index (`sr_3d::voxel::material`: `_type`
and the properties `ogt_vox.h` reads, dimensionless, the values of MagicaVoxel's sliders, and the keys it does not read as spelt), a
fingerprint of them for a cache of surfaces, the fingerprint of the cells, the colours' origin (the file's or the default palette; none
for a cache or a mesh), the size of a cell that the asset says, and where the bytes came from (digest, length, modification time). An
asset is read once for a program and key ("parse once"): a later call finds the file as it was, the same length and modification time (for
every file the model was made from), and returns the model without reading it; if either changed the bytes are read and hashed and the
model is reused only if the hash is the same (a rewrite of the same bytes is not a change, a change of one voxel with the same length is),
and a file changed with the same length AND the same time is taken for the same, the price of not hashing up to `maxMemoryMiB` on each
call. The source of a `fromMesh` is the mesh and every file the importer reads for it (a `.bin` beside a `.gltf`, the materials and
textures of an `.obj`), its digest covers all of them, and it is taken again after the mesh is cut: a file that changed in between is an
error and not a model that its digest does not describe. The mesh asset is the one of the document that has the voxel asset (an asset of
an included document is named by its namespace and its id, and so is its mesh). A box of cells wider than 2^30 along an axis is an error that
names its span (an occupancy has 2^30 keys on an axis once its corner is at the origin). The peak memory of a load is up to about three
times `maxMemoryMiB` (the bytes, the grid, and the copy that is moved when the corner is not already at the origin). `_ri` wins over
`_ior` where a material has both (`refractive_index`), and a negative zero and a zero are one number in the fingerprint of the materials,
which `tools/vox_materials_hash.py` works out again from the description of the hash. Tests (`crates/sr-eval/tests/voxels/loader.rs`): a cube of two cells
a side from a file, through the loader and the world's collider, has the mass `8 rho s^3` and the inertia `m L^2 / 6` (L = 2 s) about
each axis through its centre, with no products, to 1e-12; one model under two scene graphs that translate it differently has the same
cells and fingerprint and origins that differ by the translations in the scene's axes, the origin worked out by hand; a wrong digest, a
file that is not a model with a wrong digest, the limits, a remote source and a missing file are errors that say what they are; the
cache; the materials; a glb cube of a metre cut at 10 (the frame it is drawn in); a cache with the scale of its grid.

**Limitation: an SRVOL file has no checksum of its own.** The `sha256` of a `voxelAsset` is the evaluator's to check on the bytes it
reads (the loader of the next step); SRVOL version 1 has no field for the provenance of the file a cache was made from, and gets one,
as a version 2 or an optional chunk, when something needs to say where the cells came from.

**Bounds before allocation.** The reader checks, before it builds anything: the size of the file (default 1 GiB), the models
(65,536), the nodes of the scene graph (1,048,576), its depth (64) with a cycle check, and the count of cells that a graph places
(one model as many times as it is used, capped before it is built), then the limits of the grid. A translation or a placed cell
more than 2^24 cells from the origin is an error (so that the translations of a graph at its deepest cannot wrap), and every way of
filling the grid, from a file, a cache or a mesh, refuses a cell outside the keys of an occupancy, `[-2^30, 2^30)`, by name. A model with a side over 256 (the
coordinates of a cell are bytes, and MagicaVoxel's own models are at most 256 on a side) is refused, and every number is named in the
message. The scene graph is checked whole before anything is placed: exactly one root that reaches every node (a cycle, a second root
or a node off the tree is an error that names it), and the places that it makes and the cells in them are counted node by node, once,
so that a node under two parents cannot ask for billions of places from a file of a few hundred bytes (`max_placements`, default 2^20,
the cost of the walk whatever the models hold, a model with no voxel included; the places are then made one at a time and none is kept).
The cells that the places hold are bounded as well, by the caller's limit and by the importer's own, 2^26 (the most that `maxCells` allows,
because a default `Limits` has no bound), the bytes of the grid by the caller's and by 2^30, and the grid is built in place under them.
The rows of a mesh's box are cut a chunk at a time, so that the memory of a cut is a chunk and not the box.

| Rule | Says |
|---|---|
| VOX1 | `voxelAsset` and primitive `voxels` need `version="1.3"` |
| VOX2 | exactly one of `src` and `fromMesh`; `format`, `model`, `voxelGrid` only with a file of that format; `fromMesh` and `cellSize` together |
| VOX3 | `fromMesh` names a mesh asset |
| VOX4 | an object of primitive `voxels` names a `voxelAsset` in `voxels` |
| VOX5 | `voxels`, `cellSize`, `palette`, `surface` belong to primitive `voxels` |
| VOX6 | `palette` is `file` or at most 255 material IDs |
| VOX7 | a voxels object has no mesh, volume, terrain, map, text or path, and no medium or pyro child |
| VOX8 | `rigidBody@shape="voxels"` belongs to an object of primitive `voxels` |
| VOX9 | `density`, `maxFragments`, `fragmentMinCells`, `fragmentOverflow` and `anchor` belong to a rigid body whose collider is the cells |
| VOX10 | a body of cells has a `density` and no `mass` |
| VOX11 | `maxFragments`, `fragmentMinCells` and `fragmentOverflow` belong to a body that a crater or a fracture can break |
| VOX12 | `anchor` belongs to a body that has a crater |
| VOX13 | a body of cells that a crater or a fracture breaks is scaled the same on every axis |
| VOX14 | a body of cells has a crater or a fracture, not both |
| VOX15 | an object of cells that a crater or a fracture breaks has a `rigidBody` whose collider is the cells (`shape` `voxels` or `auto`, or none) |

The Schematron and `sr-model`'s `rules.rs` agree on all 428 documents of the corpus (and the independent `lxml` oracle of
`tools/build_corpus.py` with them): a valid document of each source and an invalid one for each rule.

**Bodies of cells in the scene (`rigidBody`, `crater`, `fracture`, `burst` on an object of primitive `voxels`).** The schema says what the physics and the render of
an object of cells can be asked for; the rules say what is refused. (This is the schema and the rules; the evaluator that makes bodies, cuts and fractures of them from
a document is the next step, and the physics it will call is already in the engine.)

*The frame.* The object's origin is the minimum corner of the box of its occupied cells, the axes are the scene's with y down, and the cell `[i, j, k]` fills
`[i, i + 1) x [j, j + 1) x [k, k + 1)` cells of `cellSize` object units; `sr_3d::voxel::cell_to_object(key, cellSize)` is the centre `(key + 1/2) * cellSize`,
`object_to_cell(p, cellSize)` the cell that holds a point (the floor of `p / cellSize`, where a point within 4 units in the last place of a face is on it and so in the cell above, since 0.3 / 0.1 is 2.9999999999999996 and 0.3 is a face of cells of 0.1; none outside the keys of an occupancy), and `file_key(key, origin_cells)` the key in the
file's own lattice. The body's centre of mass is not the origin; the world works it out from the cells. Metres are scene units over `pixelsPerMeter`.

*The body (VOX8 to VOX15).* `rigidBody@shape="voxels"` (or `auto`, which is the cells for an object of primitive voxels) makes the cells the collider: the mass is the number of
cells times the volume of one (`cellSize` times the object's scale, over `pixelsPerMeter`, cubed) times `density` (kg/m^3, required: the materials of an asset carry none; `mass` is
refused). `maxFragments` (1 to 4096, 64 in the engine), `fragmentMinCells` (loose parts of fewer cells are dust, 1) and `fragmentOverflow` (`error`, which names both numbers, or
`dust`, which makes the smallest parts dust) are the slots that the pieces of a cut take, and cost bodies of the world, so they are refused where there is nothing to cut
(no crater and no fracture on the owner, VOX11). `anchor` says which part a crater's cut leaves as the body: `base`, every part that touches the base layer (the
cells of the greatest y key: the lowest layer, the way ground is held by what is under it), which is the value when it is not given, or `largest`; with a crater only (VOX12), and the owner of a
crater is static or kinematic (CRT5), so a dynamic body has no anchor to give. A body that a crater or a fracture breaks is scaled the same on
every axis (VOX13: the cells are cubes for the cut; the rule reads the scale of the object itself, so the scale of an ancestor group, and an animated scale, are not seen by it and the evaluator has to check the world scale of the owner at run time), has the one or the other (VOX14), and has the cells for its collider (VOX15: with a box or a mesh for the collider there is no
body of cells to cut, and a document that said so would mean nothing; this does not depend on the scale, which is why it is a rule of its own and not VOX13's).

*The crater (CRT5, CRT13 to CRT17).* A crater in an object of cells is cut at the impact, once, by the law's crater with the conserving kernel (bulking 1: four fifths thrown, a
fifth heaped as the rim, in cells): so `mantle`, `bulking` and `repose`, which are ideas of an analytic surface, are refused (CRT13), and so are `start`, `end` and `curve`, which
describe a growth that a cut does not have (an attribute with no effect is a falsehood in a document): `curve` by CRT14, and `start` and `end` by CRT6, which refuses them for a crater that grows
from a source, and CRT15 gives the crater of cells one. It grows from a source (CRT15), and its rigid body may be the cells
(CRT5 gains `voxels`). `capture` is the world's and stays. The ejecta are a `particles3D` whose `burst@crater` names the crater, and for a crater of cells its particles are the
cells that the cut throws, each with its place, velocity and palette colour: the burst has no `count` (CRT16: the number is the cut's) and no `angle` or `angleSpread` (CRT17: the cells leave with the velocities the cut gives them, and a launch angle would be an
attribute with no effect), and a burst that is not of such a crater has a `count`, and may have the angles, as before.

*The fracture (FRX3, FRX8 to FRX12).* The partition is `voronoi` (the engine's: `pieces` seeds drawn from `seed`, at most 4096), `planes` (up to 63 planes of `nx ny nz offset`
in object units: a cell is on the positive side if the normal dotted with its centre `(key + 1/2) * cellSize` is at least the offset; the normal is rounded to 2^-32 of its largest component, is not all zeros and every number is finite: FRX11)
or `labels="material"` (the palette index of a cell is its label, so a model breaks along its materials); a part that is not connected is split into its components. `pieces` and `seed`
belong to voronoi (FRX10). The pieces have the material of their cells: there is no cut surface to paint, so `interiorMaterial` and `interiorUvScale` are refused for cells (FRX8; the
exposed face of a piece is drawn from the palette like any other, and a surface that is to look different is a palette index, or a later `surface` value) and `interiorMaterial` stays
required for a mesh (FRX3). The slots are those of the owner's `rigidBody`.

*The fracture by stress (`fracture@mode="stress"`, FRX13 to FRX15).* `mode` is `impact` (the engine's value and that of every fracture that has no `mode`: at a time, or by the impact
of `source`) or `stress`, which has a required `strength` in pascals (FRX13: `strength` belongs to the stress and the stress needs it), belongs to a dynamic body of cells (FRX14: a
mesh has no joints to break, a static body has nothing to hold it up and a kinematic one nothing that loads it) and is fired by the load on the body and by nothing else, so it has no
`source`, `at`, `minImpulse`, `energyFraction`, `radialImpulse` or `impulse` (FRX15). The partition is the fracture's own (`voronoi`, `planes`, `labels`): its pieces are the parts that
break off, and the joints between them are what has the strength. The evaluator does not make it yet: it refuses it by name (E24, "fracture mode stress is not evaluated yet by this build"), as a fracture by impact with no source would come apart at the first frame; `sr_eval::voxels::stress`
makes the body that the world takes, and the physics is in the engine:

* *One rigid body until a joint breaks* (`World3::with_stress`, `sr_sim::stress`). A weld in the solver between pieces that are bodies was the first thing tried, and it is refused: at
  240 steps a second a block of 2 m of 64 welded boxes hit at 100 m/s keeps 2.1 times the kinetic energy that it had (8 pieces 0.90, 256 pieces 0.87 of it), and a block of 256 costs 6.1 ms a step
  (`tests/rigid/stress_spike.rs`, the record); at 960 steps a second they lose energy, and cost four times as much. So the pieces are not bodies and the joints are not constraints: there is
  no elastic energy, no stress wave and no creep, and what is read in a step is what the body did in it.
* *The load on a side of a cut* (`sr_sim::stress::balance`). The motion of a rigid body gives the momentum of any set of its pieces exactly from the mass sums of the set, so the force and the
  moment that the rest of the body puts on a side are the change over the step of the momentum and the spin of the side, less the impulses of the loads on it: the weight and the fields (a uniform
  acceleration), the contacts of the step (the normal and friction impulses at the points the solver gave, taken through the pose of the cell's subshape, on the piece they are on) and the one
  joint of the world that may hold the body (what the balance of the whole body leaves, force and moment). The balance is about the centre of mass of the body and in the motion relative to it,
  so a step in which the body moves its own length gives what it gives at rest (a balance about a point fixed in the world left 16 N and 65 N m on a body in free fall at 4 m/s, and the error
  grows with the speed). What is left over with no joint (damping, a velocity that was set, a torque of the driver) is a rigid acceleration, which makes no stress.
* *The cut of a joint* (`sr_sim::stress::plan`). A joint that is the only way between two parts of the body (a bridge: a tree has nothing else) is cut by itself, and the load on it is exact.
  A joint in a cycle is statically indeterminate, and the rule is the beam's: the plane of the joint (through its centre, normal to its normal), the side the pieces whose centres are on its
  `a` side, the cut the joints across that plane, and the load of the side shared over them as over one rigid section, by area for a pull and by distance from the centre of the section for a
  bending moment (a ring of four pieces with joints of 1 and 3 units of area, pulled apart, has the same stress in both joints, the pull over the sum of the areas, where a tree would put it
  all on one). The cut of least area was the other candidate, and it isolates the weakest piece of a block (its load is its own weight, and says nothing of what the block carries across).
* *The stress* (`sr_sim::stress::cut_stresses`): `N / A` for the pull (tension positive), the mean shear of the force across the section over the area, the bending stress `g . r` with `g = J^-1 (n x M)`
  from the exact second moments of the faces of the cut (`sr_3d::pieces::sections`: the sums are exact integers taken from the first face and converted once) and the greatest value over the corners of
  the joint's box, and the twist as a shear by the polar moment (the formula of a round section, which a square exceeds by about an eighth). A joint breaks when the principal tension of them,
  `s / 2 + sqrt(s^2 / 4 + t^2)` with `s` the normal and bending stress added and `t` the shear and the twist added (their directions are not worked out, so this is a bound), reaches the strength
  (Rankine, a brittle joint); a push alone gives zero.
* *The order.* Every joint is read on the state at the end of a step, and those at or over the strength break together at the start of the next one, whatever order they were looked at in. The pieces that
  the broken joints leave apart are cut from the body by the machinery of the voxel split: the part that the joint of the world holds stays the body, or the largest (the first of equals); the others take
  the slots of the pool, in the order of their lowest piece, with the mass properties of their cells and the velocity of their own centres on the body they came out of, so that momentum, angular
  momentum and energy are those the body had (a beam that spins in flight breaks at its two middle joints together and keeps all three to 1e-9 over 240 steps). A part of fewer cells than `fragmentMinCells`
  is dust, and more parts than `maxFragments` is an error or the smallest are dust, as in a cut. A part that is itself pieces joined together is a body that can break again, from the same pool: the
  four pieces that fall off a cantilever land and break at every joint at once.
* *What was measured.* A cantilever of five cubes welded to the world with a weight on its tip has in every joint the principal tension of the beam to 5e-4, with the exact section modulus; the load that
  makes the root joint's principal tension the strength, found from the quadratic that includes the shear (it is the strength W / L to the second order of the depth over the length), breaks nothing in two
  seconds at 0.995 of it and breaks the root and no other joint at 1.005; a column that hangs breaks at its root at 0.99 of the weight under it over its area and not at 1.01, and a column that stands
  breaks nothing; the same bits in a fresh world, by jumps and after a seek back; a body registered as one piece, or a body that does not break, is the body it was to the bit; 640 joints of 256 pieces read
  add about 0.05 ms to a step when the block is awake (the first reading, which makes the cuts, 1.5 ms), against the 5 ms asked.
* *The friction and the frame.* The normal impulse of a contact is the step's total, point by point; the friction is not: the world's friction model (the simplified one of Rapier 0.36) solves one friction constraint for each manifold and gives the vector of the last sub-step. It is read once for each manifold, at the middle of its loaded points, and taken to the step's total by the number of sub-steps when the body has a joint of the world, and, when it has none, by the part of what the balance of the whole body leaves along it (a bar that slides at 3 m/s with a friction of 0.5 has the friction of the step 8.00 times the last sub-step's, and the moment about its centre of mass is left under a thousandth of the friction's). A load acts at the point of the body that the contact is on, not at the middle of the two surfaces (they come apart by what the contact slides: a fifth of the friction's moment), and a point of a patch that lies on the edge of two pieces belongs to the pieces under the patch. The load of a step is that of its middle, and is taken to the body's frame at the rotation halfway between the start and the end of the step (a frame of the end turned a spinning beam's force half a step against it: a pull of 2.5 read 3.25). The twist of a manifold (a torque about its normal) is not read. Bodies of a family that break in the same step claim the slots of the pool in turn.
* *Limits.* The body is rigid until it breaks: no stress wave (a load is carried by the whole body at once), no energy kept in the joints (a piece that is bent springs back for nothing), and an impact is
  one step of load. The stress of a body at rest on its contacts is that of the way the solver spread the pressure over them: the solver holds a block of 2 m by 2 m by 1 m on a floor at the four
  corners of its foot, a quarter of its weight at each (98.1 N s a step against 392.3), so the block is a deep beam of span 2 m on two supports, whose bending moment at the middle is W L / 8 = 23.5 kN m and whose
  principal tension there is 6 M / (b h^2) = 3.53e4 Pa (read: within 3%, a test). The true tension of a block that lies on its whole face is zero (it is in compression only): what is read is an artefact of the way the solver supports it, which grows with rho g L (a block twice as long reads twice as much) and is not the stress of its weight. The compression at its base, rho g h = 4.7e4 Pa, is another stress, and the 3.5e4 is not the weight over a section: a
  strength under 3.5e4 Pa breaks the block where it lies, a real material has megapascals. A joint in a cycle is the plane's cut, not the true load path. At most one
  joint of the world holds the body (the load of two is not determined by the balance of the body) and at most 1024 pieces (the cuts are worked out for every joint: 12.8 ms at the worst for 256 pieces under
  load, once and after every break). A joint of the world stays on the body that was the parent even when the piece it was anchored in is not the one that stays (the part that the joint holds stays). The dust's
  momentum leaves with it and is not recorded. A twist is read by the polar moment, which understates the largest shear of a square by 13.3% and of a thin rectangle of width b and thickness t by about b / 2t; the stress of a joint is read at the corners of the box that holds its faces, which are its corners for a rectangle and a bound for a joint that is not (an L, a staircase), and the second moments of such a joint are those of its faces added up. The contact points of a body of cells are taken through the pose of the cell's subshape,
  which the log of contacts did not do before the correction `fix/contact-points-of-cells`.

**The pieces of a body of cells (`sr_3d::pieces`).** `partition(occupancy, rule, max_pieces)` cuts a body into pieces and finds the joints
between them: by seeds (`Voronoi`, drawn from `(seed, index)` by splitmix64, or `VoronoiAt`, given; at most 4096 and within 2^40 of the origin in
doubled coordinates, an error that names the number or the seed otherwise), by up to 63 planes, or by labels the caller gives. This module has
two users that were written apart and share it, the fracture of a body of cells into rigid pieces and the fracture by stress that breaks the joints
by their area, so its fields are private and what it returns cannot be edited into something that breaks its rules. The cells of a piece are keys
of the occupancy that was cut (its own lattice, cells and not metres); a body with no cell has no pieces and is an error. Every cell is in exactly
one piece; a part of the rule that is not connected by faces is split into its components; pieces are numbered by their first cell in the scan (z, y,
x) and list their cells in it, so the result does not depend on the order of the input or on threads, and nothing depends on the order of a hash.
A piece has the exact moments of its cells (`Piece::moments`), `PieceGraph::piece_of(cell)` says which piece a cell is in, and Voronoi is exact
integers: squared distance in `i128` over the doubled coordinates `u = 2 key + 1`, a tie to the seed of the lowest index; a plane puts a cell on its
positive side if `normal . u >= offset`, compared and not subtracted, so that no offset overflows. A joint (`Edge`, `a < b`, sorted by `(a, b)`) has
the faces two pieces share BY AXIS (`faces: [u32; 3]`, so that the area is right where the cells are not cubes: `Edge::area(size)` is the faces of
each axis times the product of the two other sizes, and `Edge::centroid(size)` the mean of the centres of the faces weighted by their areas), the
sum of the doubled coordinates of their centres by axis and the sum of their unit normals from `a` to `b` (which is not an area: the faces on the
two sides of a piece wrapped round another cancel in it), all in integers. More pieces than `max_pieces` is an error that names the number, never
a truncation. What it does not do: merge or cut pieces again, give the centre of the pieces of a joint (it is in their moments), or know a material.
Tests (`crates/sr-3d/tests/geometry/pieces.rs`): a bar cut by a plane has one joint whose four faces, face sum and normal are worked out by hand;
the tie of two seeds; a U of cells whose Voronoi part is two arm tops (split into components), a constant label, alternating labels, two cells that
touch only by an edge or a corner (no joint); a ring wrapped round two cells, whose normals cancel along y and whose area and centre are worked out
by hand; the cells of a ball with a bite go to the nearest seed by a brute-force argmin over the seeds and no two pieces of one part touch, and the
joints equal a brute force over all the pairs of cells, for 1, 3, 7 and 20 seeds; a single cell; cells at the last keys of an occupancy, seeds, offsets
and normals at the extremes; the seeds are the reference sequence of splitmix64.

**Rigid bodies of cells, what a crater or a fracture does to them, and what a frame says of them (`sr_3d::occupancy`, `sr_sim::physics3d`, `sr_eval::voxels`).**
The acceptance ledger entry "Voxel physics: the track so far, what is proven and what is stated as untested" holds the figures, and the entries before it each step's.
Occupancy and mass: a body of cells is an `Occupancy` whose mass, centre of mass and inertia are worked out from exact integer moments, the same bits for the same cells in any order, and the world gives them to the solver with their principal frame (the eigen solver of the physics library swaps the axes of a diagonal tensor with a repeated smaller moment, and the exact path is also used for the pieces of every fracture).
Cuts and slots: a body that breaks gives its pieces to slots that the world holds for them, with the motion of the point of the body each was, and a frame is the same to the bit however it is asked (fresh, after a later one, again, replayed from a checkpoint).
Crater in cells: the law's crater in soft rock takes 6 460 cells out of a slab with a pillar (6 352 on flat ground, 98.34 percent of the law's volume), throws 80 percent of them in counts and heaps 20 percent on a rim that is the law's level set, as one cut of the body.
Fracture: a body of cells divided by `partition` into bodies of cells with the cut's rules for pieces that are too small or too many; the source weighs all its cells until it breaks, the fragments and the dust sum to it, and the momentum and the angular momentum that the dust takes away are recorded as lost (the fragments and the lost equal the source's to 1e-9).
Cache and frames: the mass properties, components and body of a grid are kept by its content under a budget of bytes (a hit compares the bricks, so a fingerprint collision is never a wrong answer), and a frame says how many cuts each body has had, with the cells of each revision and the bricks that differ between two.
A document reaches all of it (`rigidBody shape="voxels"` with `density`, a `crater` or a `fracture` on the object, a `burst` of the cells a cut throws, `maxFragments`, `fragmentMinCells`, `fragmentOverflow` and `anchor`); a physics cache is refused for a world with bodies of cells, and the renderer does not read what a frame says of them yet, so an object that is cut is still drawn from its asset. The axis of a crater in cells is the normal of the surface of the cells round the impact, and ground that slopes at 10, 20 and 30 degrees is cut with the ball along its normal to the law's volume to 6 percent; oblique impacts on a slope, curved ground and steeper slopes are untested, and so is an axis tilted out of the plane that the ground falls in; the limits are in that ledger entry.

## SRVOL cache version 1

This engine interchange/cache format is independent of the scene XML version.
It complements OpenVDB ingestion; it is not an OpenVDB-compatible encoding.
The implementation lives in `crates/sr-volume`.

All numbers are little-endian; arrays have no padding. A file is exactly one
frame. EOF is required after its last grid; trailing bytes are an error.

| Field | Encoding | Constraint |
|---|---|---|
| Magic | 8 bytes: `53 52 56 4f 4c 00 0d 0a` | Exact match |
| Version | u32 | 1 |
| Grid count | u32 | 0–64, additionally bounded by reader budget |
| Each grid's name length/name | u16 then ASCII bytes | 1–64 bytes; unique, lexicographically sorted |
| Background | f32 | Finite |
| Index-to-world matrix | 16 f64, column-major | Finite, affine, invertible with finite inverse |
| Brick count | u32 | Total across all grids bounded before allocation |
| Each brick's key | 3 i32 | Brick coordinates; each in [-268435456,268435455] |
| Each brick's samples | 512 f32 | Finite, x-fastest, then y, then z |

Bricks have side eight. Negative voxel coordinates use Euclidean division and
remainders. Brick keys are unique and lexicographically sorted; background-only
bricks are omitted. Integer index coordinates identify voxel centres. Trilinear
sampling includes background samples from absent bricks. Each grid has its own
transform. Generic fields can be signed; the density/temperature semantic checks
belong to the consuming volume declaration.

Default reader ceilings are 256 MiB encoded bytes, 100,000 total bricks and
64 grids. Callers may impose smaller budgets. These are resource ceilings, not
permission to allocate from untrusted counts without checking the input. The
decoder does not execute code or resolve paths stored inside a cache. Channel
names and matrices are data, never filenames. Asset URIs inherit the existing
document asset access policy.

## Exact XSD attribute inventory

This inventory records the executable XSD spelling, lexical type, requiredness
and default for each cinematic element, and, for the types that existed before this
proposal (`object3DType`, `cameraType`), only the attributes it adds or whose values it
extends; it does not claim to list every attribute of those types. “Optional; absent” means that XSD
supplies no value; the behavioral sections above specify contextual defaults
and semantic requirements. Named types refer to the shipped XSD definitions.
Inline restrictions list their base and facets. Runtime and Schematron checks
still apply, including finite values, ownership, version gates and references.

### `volumeAssetType`

| Attribute | XSD type or inline restriction | Presence/default |
|---|---|---|
| `id` | xs:ID | Required |
| `src` | volumeSourceType | Required |
| `sha256` | sha256Type | Optional; absent |
| `format` | xs:string; enumeration=srvol, enumeration=srvseq, enumeration=openvdb | Default `srvol` |
| `densityGrid` | volumeChannelType | Default `density` |
| `temperatureGrid` | volumeChannelType | Optional; absent |
| `velocityGridX` | volumeChannelType | Optional; absent |
| `velocityGridY` | volumeChannelType | Optional; absent |
| `velocityGridZ` | volumeChannelType | Optional; absent |
| `first` | volumeFrameIndexType | Optional; absent |
| `last` | volumeFrameIndexType | Optional; absent |
| `fps` | fpsType | Optional; absent |
| `interpolation` | volumeInterpolationType | Default `hold` |
| `missingFrame` | volumeMissingFrameType | Default `error` |
| `boundsMinX` | xs:double | Optional; absent |
| `boundsMinY` | xs:double | Optional; absent |
| `boundsMinZ` | xs:double | Optional; absent |
| `boundsMaxX` | xs:double | Optional; absent |
| `boundsMaxY` | xs:double | Optional; absent |
| `boundsMaxZ` | xs:double | Optional; absent |

### `meshSequenceAssetType`

Also includes `assetProvenance`, inventoried below.

| Attribute | XSD type or inline restriction | Presence/default |
|---|---|---|
| `id` | xs:ID | Required |
| `src` | volumeSourceType | Required |
| `format` | xs:string; enumeration=gltf, enumeration=glb, enumeration=obj, enumeration=ply, enumeration=usd, enumeration=usda, enumeration=usdc, enumeration=usdz, enumeration=fbx | Optional; absent |
| `first` | volumeFrameIndexType | Required |
| `last` | volumeFrameIndexType | Required |
| `fps` | fpsType | Required |
| `interpolation` | meshSequenceInterpolationType | Default `hold` |
| `missingFrame` | volumeMissingFrameType | Default `error` |
| `maxMemoryMiB` | xs:positiveInteger; maxInclusive=4096 | Default `256` |

### `voxelAssetType`

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

### `mediumType`

| Attribute | XSD type or inline restriction | Presence/default |
|---|---|---|
| `densityScale` | nonNegativeDecimal | Default `1` |
| `extinction` | nonNegativeDecimal | Default `1` |
| `albedo` | colorType | Default `#FFFFFFFF` |
| `anisotropy` | xs:double; minInclusive=-0.99, maxInclusive=0.99 | Default `0` |
| `emissionColor` | colorType | Default `#000000FF` |
| `emissionScale` | nonNegativeDecimal | Default `0` |
| `blackbody` | xs:boolean | Default `false` |
| `temperatureScale` | positiveDecimal | Default `1` |
| `stepSize` | positiveDecimal | Default `1` |
| `maxSteps` | xs:positiveInteger; maxInclusive=65536 | Default `2048` |
| `lighting` | xs:string; enumeration `exact`, `grid` | Default `exact` |
| `lightGridCell` | xs:positiveInteger; maxInclusive=64 | Default `1` |
| `lightGridDomeDirections` | xs:positiveInteger; minInclusive=8, maxInclusive=512 | Default `64` |
| `lightGridMemoryMiB` | xs:positiveInteger; maxInclusive=4096 | Default `128` |

### `pyroType`

| Attribute | XSD type or inline restriction | Presence/default |
|---|---|---|
| `colliders` | xs:IDREFS | Optional; absent |
| `colliderThickness` | positiveDecimal | Optional; absent |
| `forceFields` | xs:IDREFS | Optional; absent |
| `useForceFields` | xs:boolean | Default `true` |
| `width` | positiveDecimal | Required |
| `height` | positiveDecimal | Required |
| `depth` | positiveDecimal | Required |
| `voxelSize` | positiveDecimal | Required |
| `dt` | positiveDecimal | Default `.016666666666666666` |
| `ambientTemperature` | positiveDecimal; maxInclusive=50000 | Default `300` |
| `dissipation` | nonNegativeDecimal | Default `0` |
| `cooling` | nonNegativeDecimal | Default `0` |
| `buoyancy` | nonNegativeDecimal | Default `0` |
| `vorticity` | nonNegativeDecimal | Default `0` |
| `turbulence` | nonNegativeDecimal | Default `0` |
| `seed` | xs:unsignedLong | Default `0` |
| `pressureTolerance` | positiveDecimal | Default `0.000001` |
| `boundary` | xs:string; enumeration=open, enumeration=closed | Default `closed` |
| `follow` | xs:boolean | Optional; absent is false; true needs `boundary="open"` (PYRO9) |
| `followMargin` | xs:positiveInteger | Optional; only with `follow` (PYRO10), leaves a cell between the faces (PYRO11); the engine uses 12 |
| `followLoss` | unitDecimal | Optional; only with `follow` (PYRO10); the engine uses 0 |
| `pressureIterations` | xs:positiveInteger; maxInclusive=10000 | Default `200` |
| `solver` | xs:string; enumeration=jacobi, enumeration=multigrid | Default `jacobi` |
| `advection` | xs:string; enumeration=semilagrangian, enumeration=maccormack | Default `semilagrangian` |
| `maxMemoryMiB` | xs:positiveInteger; maxInclusive=4096 | Default `256` |
| `checkpointMemoryMiB` | xs:positiveInteger; maxInclusive=4096 | Default `256` |
| `meshMemoryMiB` | xs:positiveInteger; maxInclusive=4096 | Default `128` |

### `pyroSourceType`

Also includes `pyroShape`, inventoried below.

| Attribute | XSD type or inline restriction | Presence/default |
|---|---|---|
| `start` | xs:double | Default `0` |
| `end` | xs:double | Optional; absent |
| `densityRate` | nonNegativeDecimal | Default `0` |
| `temperatureRate` | nonNegativeDecimal | Default `0` |
| `velocityRateX` | xs:double | Default `0` |
| `velocityRateY` | xs:double | Default `0` |
| `velocityRateZ` | xs:double | Default `0` |
| `expansion` | xs:double | Default `0` |
| `crater` | xs:IDREF | Optional; a crater that grows from an impact (PYC1 to PYC4) |
| `heatFraction` | nonNegativeDecimal; maxInclusive=1 | Optional, with `crater`; engine default `0.1` |
| `dustFraction` | positiveDecimal; maxInclusive=1 | Optional, with `crater`; engine default `0.01` |
| `specificHeat` | positiveDecimal | Optional, with `crater`; default `1000` J/(kg K) |
| `maxTemperature` | positiveDecimal; maxInclusive=50000 | Optional, with `crater`; default `5000` K |

### `pyroImpulseType`

Also includes `pyroShape`, inventoried below.

| Attribute | XSD type or inline restriction | Presence/default |
|---|---|---|
| `time` | nonNegativeDecimal | Required unless `crater` is given (PYC3) |
| `density` | nonNegativeDecimal | Default `0` |
| `temperature` | nonNegativeDecimal | Default `0` |
| `velocityX` | xs:double | Default `0` |
| `velocityY` | xs:double | Default `0` |
| `velocityZ` | xs:double | Default `0` |
| `expansion` | xs:double | Default `0` |
| `crater` | xs:IDREF | Optional; a crater that grows from an impact (PYC1 to PYC4) |
| `heatFraction` | nonNegativeDecimal; maxInclusive=1 | Optional, with `crater`; engine default `0.1` |
| `dustFraction` | positiveDecimal; maxInclusive=1 | Optional, with `crater`; engine default `0.01` |
| `specificHeat` | positiveDecimal | Optional, with `crater`; default `1000` J/(kg K) |
| `maxTemperature` | positiveDecimal; maxInclusive=50000 | Optional, with `crater`; default `5000` K |

### `pyroBlastType`

| Attribute | XSD type or inline restriction | Presence/default |
|---|---|---|
| `time` | nonNegativeDecimal | Required |
| `energy` | nonNegativeDecimal | Required; joules |
| `x` | xs:double | Default `0` |
| `y` | xs:double | Default `0` |
| `z` | xs:double | Default `0` |
| `ambientDensity` | positiveDecimal | Default `1.2`; kg/m^3 |
| `ambientPressure` | positiveDecimal | Default `101325`; Pa |
| `gamma` | positiveDecimal; minInclusive=1.1, maxInclusive=3 | Default `1.4` |

### `particles3DType`

| Attribute | XSD type or inline restriction | Presence/default |
|---|---|---|
| `id` | xs:ID | Required |
| `name` | xs:string | Optional; absent |
| `x` | xs:double | Default `0` |
| `y` | xs:double | Default `0` |
| `z` | xs:double | Default `0` |
| `rotation` | xs:double | Default `0` |
| `rotationX` | xs:double | Default `0` |
| `rotationY` | xs:double | Default `0` |
| `start` | xs:double | Default `0` |
| `end` | xs:double | Optional; absent |
| `condition` | expressionString | Optional; absent |
| `parent` | xs:IDREF | Optional; absent |
| `scaleX` | xs:double | Default `1` |
| `scaleY` | xs:double | Default `1` |
| `scaleZ` | xs:double | Default `1` |
| `visible` | xs:boolean | Default `true` |
| `opacity` | unitDecimal | Default `1` |
| `motionBlur` | triStateType | Default `inherit` |
| `castShadow` | xs:boolean | Default `true` |
| `receiveShadow` | xs:boolean | Default `true` |
| `useForceFields` | xs:boolean | Default `true` |
| `rate` | nonNegativeDecimal | Default `10` |
| `emissionStart` | nonNegativeDecimal | Default `0` |
| `lifetimeVariance` | nonNegativeDecimal | Default `0` |
| `speed` | nonNegativeDecimal | Default `0` |
| `speedVariance` | nonNegativeDecimal | Default `0` |
| `drag` | nonNegativeDecimal | Default `0` |
| `collisionRadius` | nonNegativeDecimal | Default `0.5` |
| `friction` | nonNegativeDecimal | Default `0` |
| `trail` | nonNegativeDecimal | Default `1` |
| `emissionEnd` | nonNegativeDecimal | Optional; absent |
| `dt` | positiveDecimal | Default `.016666666666666666` |
| `lifetime` | positiveDecimal | Default `2` |
| `emitterWidth` | positiveDecimal | Default `1` |
| `emitterHeight` | positiveDecimal | Default `1` |
| `emitterDepth` | positiveDecimal | Default `1` |
| `emitterRadius` | positiveDecimal | Default `1` |
| `size` | positiveDecimal | Default `1` |
| `collisionTolerance` | positiveDecimal | Default `0.001` |
| `velocityX` | xs:double | Default `0` |
| `velocityY` | xs:double | Default `0` |
| `velocityZ` | xs:double | Default `0` |
| `gravityX` | xs:double | Default `0` |
| `gravityY` | xs:double | Default `0` |
| `gravityZ` | xs:double | Default `0` |
| `rotation0X` | xs:double | Default `0` |
| `rotation0Y` | xs:double | Default `0` |
| `rotation0Z` | xs:double | Default `0` |
| `rotationVarianceX` | nonNegativeDecimal | Default `0` |
| `rotationVarianceY` | nonNegativeDecimal | Default `0` |
| `rotationVarianceZ` | nonNegativeDecimal | Default `0` |
| `angularVelocityX` | xs:double | Default `0` |
| `angularVelocityY` | xs:double | Default `0` |
| `angularVelocityZ` | xs:double | Default `0` |
| `angularVelocityVarianceX` | nonNegativeDecimal | Default `0` |
| `angularVelocityVarianceY` | nonNegativeDecimal | Default `0` |
| `angularVelocityVarianceZ` | nonNegativeDecimal | Default `0` |
| `inheritedVelocityX` | xs:double | Default `0` |
| `inheritedVelocityY` | xs:double | Default `0` |
| `inheritedVelocityZ` | xs:double | Default `0` |
| `directionX` | xs:double | Default `0` |
| `directionY` | xs:double | Default `-1` |
| `directionZ` | xs:double | Default `0` |
| `spread` | nonNegativeDecimal; maxInclusive=360 | Default `0` |
| `scaleVariance` | nonNegativeDecimal; maxExclusive=1 | Default `0` |
| `bounce` | unitDecimal | Default `0.5` |
| `seed` | xs:unsignedLong | Default `0` |
| `maxParticles` | xs:positiveInteger; maxInclusive=1000000 | Default `10000` |
| `maxEvents` | xs:positiveInteger; maxInclusive=1000000 | Default `16384` |
| `maxMemoryMiB` | xs:positiveInteger; maxInclusive=4096 | Default `256` |
| `checkpointMemoryMiB` | xs:positiveInteger; maxInclusive=4096 | Default `64` |
| `meshMemoryMiB` | xs:positiveInteger; maxInclusive=4096 | Default `128` |
| `maxWork` | xs:positiveInteger; maxInclusive=1000000000 | Default `100000000` |
| `segments` | xs:positiveInteger; maxInclusive=256 | Default `12` |
| `emitterMesh` | xs:IDREF | Optional; absent |
| `mesh` | xs:IDREF | Optional; absent |
| `material` | xs:IDREF | Optional; absent |
| `sprite` | xs:IDREF | Optional; absent |
| `forceFields` | xs:IDREFS | Optional; absent |
| `colliders` | xs:IDREFS | Optional; absent |
| `gas` | xs:IDREF | Optional; absent; the object3D whose native pyro volume drags the particles (P3D11) |
| `emitterShape` | xs:string; enumeration=point, enumeration=box, enumeration=sphere, enumeration=mesh | Default `point` |
| `shape` | xs:string; enumeration=sphere, enumeration=billboard, enumeration=streak, enumeration=mesh | Default `sphere` |
| `sizeEnd` | nonNegativeDecimal | Optional; absent |
| `color` | colorType | Default `#FFFFFFFF` |
| `colorEnd` | colorType | Optional; absent |
| `opacityEnd` | unitDecimal | Optional; absent |
| `sizeCurve` | curveType | Default `linear` |
| `colorCurve` | curveType | Default `linear` |

### `burst3DType`

| Attribute | XSD type or inline restriction | Presence/default |
|---|---|---|
| `time` | xs:double | Required unless `crater` is given (P3D7) |
| `count` | xs:positiveInteger | Required, except on a burst of the crater of an object of cells, where it is refused (CRT16); `angle` and `angleSpread` are refused there too (CRT17) |
| `repeat` | xs:nonNegativeInteger | Default `0` |
| `interval` | positiveDecimal | Default `1` |
| `crater` | xs:IDREF | Optional; a crater that grows from an impact (P3D7 to P3D10) |
| `angle` | nonNegativeDecimal; maxInclusive=90 | Optional, with `crater`; engine default `45` degrees |
| `angleSpread` | nonNegativeDecimal; maxInclusive=90 | Optional, with `crater`; engine default `15` degrees |

### `oceanType`

| Attribute | XSD type or inline restriction | Presence/default |
|---|---|---|
| `id` | xs:ID | Required |
| `name` | xs:string | Optional; absent |
| `x` | xs:double | Default `0` |
| `y` | xs:double | Default `0` |
| `z` | xs:double | Default `0` |
| `rotation` | xs:double | Default `0` |
| `rotationX` | xs:double | Default `0` |
| `rotationY` | xs:double | Default `0` |
| `start` | xs:double | Default `0` |
| `end` | xs:double | Optional; absent |
| `condition` | expressionString | Optional; absent |
| `parent` | xs:IDREF | Optional; absent |
| `scaleX` | xs:double | Default `1` |
| `scaleY` | xs:double | Default `1` |
| `scaleZ` | xs:double | Default `1` |
| `visible` | xs:boolean | Default `true` |
| `opacity` | unitDecimal | Default `1` |
| `motionBlur` | triStateType | Default `inherit` |
| `castShadow` | xs:boolean | Default `true` |
| `receiveShadow` | xs:boolean | Default `true` |
| `width` | positiveDecimal | Default `64` |
| `depth` | positiveDecimal | Default `64` |
| `cellSize` | positiveDecimal | Default `1` |
| `waterLevel` | xs:double | Default `0` |
| `bottomDepth` | nonNegativeDecimal | Default `10` |
| `gravity` | positiveDecimal | Default `9.81` |
| `damping` | nonNegativeDecimal | Default `0` |
| `dt` | positiveDecimal | Default `.016666666666666666` |
| `dryTolerance` | positiveDecimal | Default `0.0000000001` |
| `initialVelocityX` | xs:double | Default `0` |
| `initialVelocityZ` | xs:double | Default `0` |
| `bathymetryScale` | xs:double | Default `1` |
| `bathymetryOffset` | xs:double | Default `0` |
| `seed` | xs:unsignedLong | Default `0` |
| `maxMemoryMiB` | xs:positiveInteger; maxInclusive=4096 | Default `256` |
| `checkpointMemoryMiB` | xs:nonNegativeInteger; maxInclusive=4096 | Default `64` |
| `meshMemoryMiB` | xs:positiveInteger; maxInclusive=4096 | Default `128` |
| `surfaceMemoryMiB` | xs:positiveInteger; maxInclusive=4096 | Default `128` |
| `maxWork` | xs:positiveInteger; maxInclusive=1000000000000 | Default `100000000` |
| `boundary` | xs:string; enumeration=closed, enumeration=open, enumeration=periodic | Default `closed` |
| `bathymetryEncoding` | xs:string; enumeration=red, enumeration=terrarium, enumeration=mapbox | Default `red` |
| `order` | xs:string; enumeration=1, enumeration=2 | Default `1` |
| `material` | xs:IDREF | Optional; absent |
| `bathymetry` | xs:IDREF | Optional; absent |
| `colliders` | xs:IDREFS | Optional; absent |
| `bodyCoupling` | xs:string; enumeration=none, enumeration=buoyancy, enumeration=full | Default `none` (the water does nothing to the bodies); `buoyancy` and `full` need `colliders` (OCN8) |
| `density` | positiveDecimal | Default `1000`; density of the water, kilograms per cubic metre |
| `bodyDrag` | nonNegativeDecimal | Optional, with `colliders` (OCN9); form-drag coefficient of the vertical motion of a body in the water (with `bodyCoupling`) and of its horizontal exchange with it (`bedResponse="depthFiltered"`), default `1.0` |
| `bedResponse` | xs:string; enumeration=depthFiltered, enumeration=hydrostatic | Default `depthFiltered`; no effect without `colliders` |
| `splash` | xs:IDREFS | Optional; absent; the particles3D emitters whose particles fall into this ocean, each throwing out the ejecta of a crater (OCN13) |

### `oceanWaveType`

| Attribute | XSD type or inline restriction | Presence/default |
|---|---|---|
| `wavelength` | positiveDecimal | Default `16` |
| `amplitude` | nonNegativeDecimal | Default `1` |
| `direction` | xs:double | Default `0` |
| `phase` | xs:double | Optional; absent |
| `speed` | nonNegativeDecimal | Optional; absent |

### `waterImpulseType`

| Attribute | XSD type or inline restriction | Presence/default |
|---|---|---|
| `time` | nonNegativeDecimal | Default `0` |
| `x` | xs:double | Default `0` |
| `z` | xs:double | Default `0` |
| `radius` | positiveDecimal | Default `1` |
| `amplitude` | xs:double | Default `1` |
| `velocityX` | xs:double | Default `0` |
| `velocityZ` | xs:double | Default `0` |
| `type` | xs:string; enumeration=displace, enumeration=add-water | Default `displace` |
| `source` | xs:IDREF | Optional; a body that enters the water (OCN10 to OCN12); excludes the attributes above |

### `whitewaterType`

| Attribute | XSD type or inline restriction | Presence/default |
|---|---|---|
| `emissionRate` | nonNegativeDecimal | Default `10` |
| `threshold` | nonNegativeDecimal | Default `0.5` |
| `start` | nonNegativeDecimal | Default `0` |
| `lifetime` | positiveDecimal | Default `3` |
| `sprayFraction` | unitDecimal | Default `0.4` |
| `launchSpeed` | nonNegativeDecimal | Default `3` |
| `drag` | nonNegativeDecimal | Default `0.1` |
| `radius` | positiveDecimal | Default `0.05` |
| `seed` | xs:unsignedLong | Default `0` |
| `end` | nonNegativeDecimal | Optional; absent |
| `maxParticles` | xs:positiveInteger; maxInclusive=1000000 | Default `10000` |
| `maxMemoryMiB` | xs:positiveInteger; maxInclusive=4096 | Default `64` |
| `maxWork` | xs:positiveInteger; maxInclusive=1000000000000 | Default `100000000` |
| `checkpointMemoryMiB` | xs:nonNegativeInteger; maxInclusive=4096 | Default `64` |
| `foamMaterial` | xs:IDREF | Optional; absent |
| `sprayMaterial` | xs:IDREF | Optional; absent |
| `foamMode` | enumeration `particles`, `albedo` | Default `particles` |
| `foamRadius` | positiveDecimal | Optional; absent (one cell) |
| `foamAlbedo` | unitDecimal | Default `0.9` |
| `foamRoughness` | unitDecimal | Default `0.8` |

### `craterType`

| Attribute | XSD type or inline restriction | Presence/default |
|---|---|---|
| `id` | xs:ID | Optional; names the crater so that what its impact causes can refer to it |
| `source` | xs:IDREF | Optional; the dynamic rigid body that makes the crater (CRT6 to CRT8) |
| `capture` | xs:boolean | Default `false`; only with `source` (CRT9): the body that makes the crater is arrested by it |
| `mantle` | xs:boolean | Default `false`; only with `source` (CRT10): the ejecta come down as a mantle that is part of the ground, and the volumes add up |
| `bulking` | xs:double; 1 to 1.3 | Optional, only with `mantle="true"` (CRT10, CRT11); what the rim and the mantle put back, in volumes of the bowl; without it, what the law's own rim height asks for |
| `repose` | xs:double; 10 to 60 | Optional, only with `source` and not with `mantle="true"` (CRT10, CRT12); the angle of repose in degrees of the debris: the ejecta particles that come to rest are removed and their volume is poured onto a deposit that relaxes at this angle and lies on the ground |
| `targetMaterial` | xs:string; enumeration=water, drySand, drySoil, wetSoil, softRock, hardRock, regolith, ice | Required with `source`; absent otherwise (CRT7) |
| `targetDensity` | positiveDecimal | Optional with `source`: kg/m3 |
| `strength` | nonNegativeDecimal | Optional with `source`: Pa |
| `gravity` | positiveDecimal | Optional with `source`: m/s2; default the physics gravity |
| `centerX` | xs:double | Default `0` |
| `centerY` | xs:double | Default `0` |
| `centerZ` | xs:double | Default `0` |
| `normalX` | xs:double | Default `0` |
| `normalY` | xs:double | Default `0` |
| `normalZ` | xs:double | Default `-1` |
| `radius` | positiveDecimal | Default `50` |
| `depth` | nonNegativeDecimal | Default `10` |
| `rimHeight` | nonNegativeDecimal | Default `2` |
| `rimWidth` | positiveDecimal | Default `10` |
| `influenceDepth` | positiveDecimal | Optional; absent |
| `start` | nonNegativeDecimal | Default `0` |
| `end` | nonNegativeDecimal | Default `1` |
| `curve` | xs:string; enumeration=linear, enumeration=ease-in, enumeration=ease-out, enumeration=ease-in-out, enumeration=step | Default `ease-in-out` |
| `maxMemoryMiB` | xs:positiveInteger; maxInclusive=4096 | Default `128` |

### `fractureType`

| Attribute | XSD type or inline restriction | Presence/default |
|---|---|---|
| `at` | nonNegativeDecimal | Default `0` |
| `pieces` | xs:positiveInteger; maxInclusive=4096 | Default `8` |
| `seed` | xs:unsignedLong | Default `0` |
| `interiorMaterial` | xs:IDREF | Required of a fracture of a mesh (FRX3); refused on a fracture of cells (FRX8) |
| `interiorUvScale` | positiveDecimal | Default `1` |
| `impulseX` | xs:double | Default `0` |
| `impulseY` | xs:double | Default `0` |
| `impulseZ` | xs:double | Default `0` |
| `radialImpulse` | nonNegativeDecimal | Default `0` |
| `maxMemoryMiB` | xs:positiveInteger; maxInclusive=4096 | Default `256` |
| `source` | xs:IDREF | Optional; the dynamic rigid body whose impact breaks the owner (FRX5 to FRX7) |
| `minImpulse` | positiveDecimal | Optional; only with `source` (FRX7); absent: twice the source's weight in one step |
| `energyFraction` | unitDecimal | Optional, no XSD default; only with `source` (FRX7); the engine uses `0.3` |
| `partition` | xs:string; enumeration=voronoi, enumeration=planes, enumeration=labels | Optional; only on the fracture of an object of cells (FRX9); engine default `voronoi` |
| `planes` | xs:string | Optional; with `partition="planes"` (FRX11): up to 63 planes of four numbers `nx ny nz offset` in object units |
| `labels` | xs:string; enumeration=material | Optional; with `partition="labels"` (FRX12) |
| `mode` | xs:string; enumeration=impact, enumeration=stress | Optional, no XSD default; `impact` is the engine's; `stress` needs `strength` (FRX13), a dynamic body of cells (FRX14) and none of `source`, `at`, `minImpulse`, `energyFraction`, `radialImpulse`, `impulseX`, `impulseY`, `impulseZ` (FRX15) |
| `strength` | positiveDecimal | Required with `mode="stress"` and only with it (FRX13): the principal tension, in pascals, that breaks a joint |

### `rigidBody3DType` bindings of a body of cells

| Attribute | XSD type or inline restriction | Presence/default |
|---|---|---|
| `shape` | adds enumeration=voxels | `auto` is the cells for an object of primitive voxels (VOX8) |
| `density` | positiveDecimal | Required when the collider is the cells, and `mass` is then not given (VOX9, VOX10); kg/m^3 |
| `maxFragments` | xs:positiveInteger; maxInclusive=4096 | Optional, no XSD default; only with a crater or a fracture on the owner (VOX11); the engine uses `64` |
| `fragmentMinCells` | xs:positiveInteger | Optional, no XSD default; as `maxFragments` (VOX11); the engine uses `1` |
| `fragmentOverflow` | xs:string; enumeration=error, enumeration=dust | Optional, no XSD default; as `maxFragments` (VOX11); the engine uses `error` |
| `anchor` | xs:string; enumeration=largest, enumeration=base | Optional; only with a crater on the owner (VOX12), which is static or kinematic (CRT5), so the engine's value when it is not given is `base` |

### `assetProvenance`

| Attribute | XSD type or inline restriction | Presence/default |
|---|---|---|
| `sha256` | sha256Type | Optional; absent |
| `license` | xs:string | Optional; absent |
| `credit` | xs:string | Optional; absent |
| `proxy` | xs:anyURI | Optional; absent |

### `pyroShape`

| Attribute | XSD type or inline restriction | Presence/default |
|---|---|---|
| `shape` | xs:string; enumeration=sphere, enumeration=box, enumeration=mesh | Default `sphere` |
| `radius` | positiveDecimal | Default `1` |
| `width` | positiveDecimal | Optional; absent |
| `height` | positiveDecimal | Optional; absent |
| `depth` | positiveDecimal | Optional; absent |
| `x` | xs:double | Default `0` |
| `y` | xs:double | Default `0` |
| `z` | xs:double | Default `0` |
| `rotation` | xs:double | Default `0` |
| `rotationX` | xs:double | Default `0` |
| `rotationY` | xs:double | Default `0` |
| `scaleX` | xs:double | Default `1` |
| `scaleY` | xs:double | Default `1` |
| `scaleZ` | xs:double | Default `1` |
| `mesh` | xs:IDREF | Optional; absent |


### `object3DType` cinematic bindings

The following attributes connect the new elements and terrain or path-tracing
configuration to existing objects/cameras; other attributes retain their
existing definitions.

| Attribute | XSD type or inline restriction | Presence/default |
|---|---|---|
| `primitive` | xs:string; enumeration=sphere, enumeration=box, enumeration=plane, enumeration=mesh, enumeration=cylinder, enumeration=cone, enumeration=torus, enumeration=capsule, enumeration=text, enumeration=extrude, enumeration=clay, enumeration=map, enumeration=globe, enumeration=volume, enumeration=voxels | Required |
| `material` | xs:IDREF | Optional; absent |
| `mesh` | xs:IDREF | Optional; absent |
| `volume` | xs:IDREF | Optional; absent |
| `voxels` | xs:IDREF | Optional; a `voxelAsset`, required with primitive `voxels` (VOX4, VOX5) |
| `cellSize` | positiveDecimal | Optional, no XSD default; only with primitive `voxels` (VOX5); absent: the asset's, else the cache's, else `1` |
| `palette` | xs:string | Optional; `file` or at most 255 material IDs (VOX5, VOX6); absent: `file` if the file has colours, else the object's `material` |
| `surface` | xs:string; enumeration=blocks | Optional, no XSD default; only with primitive `voxels` (VOX5); the engine uses `blocks` |
| `terrain` | xs:IDREF | Optional; absent |
| `planetRadius` | positiveDecimal | Default `6378137` |
| `terrainTileSize` | xs:positiveInteger; maxInclusive=4096 | Default `256` |
| `terrainZoom` | xs:nonNegativeInteger; maxInclusive=22 | Optional; absent |
| `terrainMissing` | xs:string; enumeration=error, enumeration=zero | Default `error` |
| `terrainMemoryMiB` | xs:positiveInteger; maxInclusive=4096 | Default `128` |
| `terrainEncoding` | xs:string; enumeration=terrarium, enumeration=mapbox | Default `terrarium` |
| `exaggeration` | nonNegativeDecimal | Default `1` |
| `textureSize` | xs:positiveInteger; minInclusive=64, maxInclusive=8192 | Default `2048` |
| `resolution` | xs:positiveInteger; minInclusive=8, maxInclusive=256 | Default `64` |

This table is not the whole of `object3DType`, and the inventory above makes no claim to be. The type also
carries attributes that other changes added and that this proposal neither defines nor depends on: `shadowCatcher`
(commit c737214), `node` and `materialOverride` (43d03ad), `tracking` (54dd99b), `map` and `buildings` (0f63bdc), and on
`materialType` `unevenness` and `unevennessScale` (096f530). They are upstream's and are specified where they were
added; reconciling the XSD with this document means checking the attributes listed here, not those.

### `blackHoleType`

| Attribute | XSD type or inline restriction | Presence/default |
|---|---|---|
| `id` | xs:ID | Required |
| `name` | xs:string | Optional |
| `mass` | positiveDecimal | Required; scene units with G = c = 1 |
| `x` | xs:double | Default `0` |
| `y` | xs:double | Default `0` |
| `z` | xs:double | Default `0` |

### `accretionDiskType`

| Attribute | XSD type or inline restriction | Presence/default |
|---|---|---|
| `id` | xs:ID | Required |
| `name` | xs:string | Optional |
| `blackHole` | xs:IDREF | Required (BH3) |
| `innerRadius` | positiveDecimal | Optional; absent is 6 `mass` (BH4) |
| `outerRadius` | positiveDecimal | Required (BH4) |
| `temperatureScale` | positiveDecimal | Required; kelvin, at the peak of the profile |
| `seed` | xs:unsignedLong | Default `0` |
| `angularPattern` | xs:string; enumeration=none, enumeration=clumps, enumeration=spiral | Default `clumps` |
| `contrast` | unitDecimal | Default `0.5` |
| `intensity` | nonNegativeDecimal | Default `1` |
| `timeScale` | positiveDecimal | Default `1` |
| `rotationX` | xs:double | Default `0` |
| `rotationY` | xs:double | Default `0` |
| `rotation` | xs:double | Default `0`; about z |

### `cameraType` cinematic bindings

The following attributes connect the new elements and terrain or path-tracing
configuration to existing objects/cameras; other attributes retain their
existing definitions.

| Attribute | XSD type or inline restriction | Presence/default |
|---|---|---|
| `renderer` | xs:string; enumeration=raster, enumeration=pathtrace | Default `raster` |
| `pathSamples` | xs:positiveInteger; maxInclusive=65536 | Default `64` |
| `maxBounces` | xs:positiveInteger; maxInclusive=64 | Default `4` |
| `denoise` | xs:boolean | Default `true` |
| `geodesics` | xs:boolean | Default `false`; true traces null geodesics of the scene's `blackHole` (BH5 to BH8) |


## Conformance and acceptance

The runnable native combination example is
[`examples/cinematic-impact/impact.scene.xml`](examples/cinematic-impact/impact.scene.xml).
It authors an oblique approach, submerged crater, shallow-water displacement,
1,500 ejecta particles and a thermal smoke plume at 3840×2160. Its units and
timings are artistic, explicitly identified in scene metadata. It is a workload
and integration example, not a validated scientific Chicxulub reconstruction or
an accepted cinematic-quality film. The evaluator integration test exercises
pre-impact, impact, late settling and reverse replay; production-size execution
is practical with `cargo test --release -p sr-eval --test crater cinematic_impact::`.
The full encoded-sequence gate below remains independently required.

Two further examples are written in physical units (`physics/@pixelsPerMeter="1"`, metres and
seconds, gravity declared) and author no time for any effect:
[`impact-land.scene.xml`](examples/cinematic-impact/impact-land.scene.xml) (a rock of 2 m radius and 2700
kg/m3 arrives at 100 m/s and 60 degrees on soft rock; the crater, its smoke and its ejecta, 4000 particles
that hold 80 % of the crater's mass, are consequences of the contact) and
[`impact-ocean.scene.xml`](examples/cinematic-impact/impact-ocean.scene.xml) (the same rock arrives at a
second-order ocean 20 m deep that carries it with `bodyCoupling="full"` and makes a crater in the
seabed; its `waterImpulse` names the rock and says nothing else: the cavity its entry makes). `cargo test -p sr-eval --test crater impact_scenes::` checks, with no GPU, that
no effect has a time attribute, that nothing happens before the contact, that the crater, the dust, the
heat in the dust and the ejecta (their mass, which is 0.8 of the crater's, and their reach) grow with speed,
mass and angle, that an oblique impact carries the ejecta downrange, that the ocean's water volume is conserved to the last
cell and the dust is what the law gives, and that any order of instants, a fresh evaluator and a replay
from the first checkpoint give the same bits. Limits that the scenes show and the engine does not hide:

- The ejecta of the land scene land on the ground, which is their collider, with a restitution of 0.15, a
  friction of 0.7 and the contact radius of the 0.34 m rocks they stand for, 0.17 m (the friction is about tan 35
  degrees, the angle of repose of coarse rock debris, inside the 0.6 to 0.85 that Byerlee's law gives for rock on
  rock: the engine's choice, not a measurement of this ejecta, and the literature ranges are from memory and not
  checked). They are born clear of the surface the crater has by then, so the heaviest rock of the sweeps
  (270 000 kg, the biggest crater) runs to the end like the others. Measured by `impact_scenes` (commit e22458b
  on fa63e5d, 2026-10-05, deterministic): with the authored rock 3748 of the 4000 are under 0.2 m/s at 5.9 s and
  none is below the ground; with the heaviest rock 1861 of 4000, none below the ground over its 80 m, and 29 that
  flew past its edge and fall on nothing. The ocean scene has no ejecta: they would be launched from the sea bed
  and ignore the water, and with the authored 20 m of water at most 34 of 3000 ejecta of the biggest rock of the
  sweeps reach the surface (commit 3078153, 2026-10-05).
- The ocean scene has no smoke: the crater is under water and the smoke solver has no smoke inside water,
  so a smoke source from that crater would make a cloud on the sea bed.
- In the ocean scene, with `bodyCoupling="full"` and the cavity of the rock's entry (`waterImpulse@source`), the
  sweeps are run with the ocean answering by the depth of the water (`bedResponse="depthFiltered"`, the default) and
  by the hydrostatic pressure (`"hydrostatic"`), each written into the document by the tests, on 3 m cells, and
  `tools/impact_sweeps.sh` prints them. What the tests assert is an order, never a value. The far wave of the sea
  (the highest the water stands above its rest level in the ring 20 to 40 m from where the rock enters, from the
  moment the rock is in it) grows with the speed (60, 100 and 150 m/s at 60 degrees) and with the mass (60 000, 90 478
  and 270 000 kg), and the far wave of a rock that arrives straight down grows with both, in both responses, over the
  whole sweep and over three lighter or slower points (40, 60 and 80 m/s; 20 000, 40 000 and 60 000 kg), chosen
  because the cavity of the first model was limited at the heavier ones; the crater on the seabed grows with speed and
  mass; the water is conserved to the last cell; and the highest surface anywhere grows with the speed at 60 degrees.
  The highest surface anywhere by mass at 60 degrees, by speed and mass of a vertical plunge, and every sweep by
  angle are recorded in ignored tests and not asserted: the first pair of the mass sweep is 0.4 % apart.
  Measured by `impact_scenes` (commit e22458b on fa63e5d, 2026-10-05, load1 13 to 15; the values are deterministic
  and the load does not change them; the amplitudes change with the cavity's kernel, so this is a dated
  measurement and not a criterion), depth filter | hydrostatic, metres. Far wave: by speed 0.91, 1.51 and 2.30 |
  0.89, 1.54 and 2.31; by mass 1.14, 1.51 and 2.71 | 1.18, 1.54 and 2.76; a vertical plunge by speed 0.99, 1.72 and
  2.56 | 1.01, 1.73 and 2.57, by mass 1.34, 1.72 and 2.94 | 1.34, 1.73 and 2.99; the three lighter or slower points
  by speed 0.61, 0.91 and 1.17 | 0.61, 0.89 and 1.22, by mass 0.62, 0.93 and 1.14 | 0.66, 0.91 and 1.18. The depth of
  the cavity by the law for those rocks is 7.5 to 15.8 m, under the 20 m of water. Highest surface anywhere: by speed
  4.17, 4.74 and 7.52 | 4.31, 4.84 and 7.58; by mass 4.72, 4.74 and 8.22 | 4.79, 4.84 and 8.42; a vertical plunge by speed
  4.53, 5.82 and 8.07 | 4.58, 5.85 and 8.02, by mass 4.71, 5.82 and 8.55 | 4.72, 5.85 and 8.60; by angle 30, 60 and 90
  degrees 4.10, 4.74 and 5.82 | 4.11, 4.84 and 5.85. The crater on the seabed is 2.81, 3.42 and 4.09 m in radius by speed
  and 2.15, 3.42 and 7.49 m by mass (depth 1.18, 1.43 and 1.72 m by speed), and 2.77, 3.42 and 3.63 m by angle, the same
  in both responses. The wave of the crater alone (the sea with the bed as its only collider, no cavity) grows with
  speed, mass and angle in both responses: by speed 0.006, 0.012 and 0.021 m with the filter and 0.16, 0.27 and
  0.40 m without. With `capture` the rock rests in its crater and without it the rock lies 8.5 m (filtered) or 0.7 m
  (hydrostatic) from it at 6 s. A body lighter than water floats and makes no
  crater.
- Everything in one ocean. A test over 4 m of water (the seabed raised, the rest of the scene as authored) puts
  the bed cratered by contact and the rock as colliders, `bodyCoupling="full"`, the cavity of the entry and 3000
  ejecta that fall into the same ocean (`splash`), in the depth-filtered response, with the ocean keeping no
  checkpoint. Nothing fails, the water is conserved to a part in 1e9, and the ocean, the rock and the particles are
  the same to the bit at any time asked in any order and from a fresh evaluator. 11 of the 3000 ejecta fall in
  (0.20 m3), because the rest are thrown from the seabed and stay under water (test `impact_scenes`, commit
  b1552ce, 2026-10-05, deterministic). The scheduling that makes this possible is in the paragraph on ejecta falling
  into an ocean; the whole-frame cost of the ocean side of the scene is in the ledger, milestone "Phase budget".
- The dust is as hot at 30 degrees as at 90: the heat falls as sin^1.5 of the angle and the dust volume as
  the speed along the normal to the power 1.7, so the temperature rise barely moves (19.20, 19.07 and 19.08 K
  at 30, 60 and 90 degrees at 100 m/s with the default heat fraction of 0.1); the heat held by the dust grows
  with the angle (5.7, 13.6 and 17.1 cubic metres of dust times kelvin), and the temperature grows with speed
  and with mass (test `impact_scenes`, commit e22458b, 2026-10-05, deterministic). A slow impact in physical
  units is a cold cloud.

A simulation that is authored but cannot run (a resource limit, a solver error) is an
error of the frame, never a note that lets the render succeed without what was asked
for (commit 91e9013, 2026-10-04). The evaluator's frame graph carries `failures`, the
part of its problems that are such simulations (smoke, ocean, 3D particles, fracture,
and the crater and collider limits of a rigid body); the renderer reports each as an
error with its cause, drops the derived messages for a node whose simulation failed
(the old "volume primitive requires @volume" and "ocean evaluation produced no
surface"), and keeps authorised fallbacks as notes. The command line prints every
error and the notes when it exits on one, and delivery reports all of them. `render`
(with and without `--strict`), `render --bench` and `encode` exit 1 with the cause and
write no image for these scenes, and a failure found when the world is built is
reported at every time, also before the simulation's own start. Test:
`crates/scene-render/tests/solver_failures.rs`, seven cases (smoke, the ocean's solver
and its surface, 3D particles, a crater's draw vertices, a rigid crater's collider,
fracture), four commands each.

The accompanying conformance suite must cover all of the following:

- Valid minimal examples for each new element, every enum branch, default
  expansion, bad references, version gates, nonfinite numbers and invalid bounds.
- Independent cache fixtures, every truncated header/payload boundary, invalid
  transforms, duplicate grids/bricks, total resource budgets and deterministic
  serialization. Compare OpenVDB samples/transforms to independent reference data.
- Uniform-density transmittance against Beer–Lambert, transformed media,
  overlapping media in reversed order, inside-volume cameras, mesh occlusion,
  emission, colored scattering and shadowed smoke at changing viewpoints.
- Light through water and glass: a floor under water lit by the sun, a point
  light and a sphere light against brute force, absorption against its closed
  form and by distance, the shadow's refracted position, steep waves without
  gain or flares, glass shadows in air unchanged, an untouched transmissive object
  leaving the picture alone, a camera under the water as documented, tiled equal
  to whole frames, and glass over nothing showing the visible dome.
- Light grids: default exact lighting unchanged by their existence; grid lighting
  against the exact march for each light type, environment (isotropic and
  anisotropic), cameras inside the volume, surfaces inside the domain, overlapping
  media, advected and multi-frame media, tiled against whole frames, determinism,
  memory and node-count errors, and the measured limits above (VOL10 included).
- Render statistics that add no pixel, the probe's report logic, the skip of
  negligible cells (which cells, how the bound moves with extinction, density scale
  and domain, which media are marched in full), the brick directory and its fallback to
  the search, and the unchanged frame of a frozen plume with both.
- A simulation that cannot run failing every render command with its cause and no
  partial image, at every time.
- Pyro divergence reduction, source timing, cooling/dissipation, obstacles,
  forward/backward seeking and cache/live equivalence.
- 3D particles' z motion, distribution, lifetime/cap behavior, delayed birth,
  moving emitters/colliders, field windows and deterministic seeking.
- Ocean rest equilibrium, water conservation, wave propagation speed, positivity,
  shoreline behavior, impulses, spray and replay.
- Globe elevation/seams, crater normals and collision updates, fracture mass
  conservation and activation, mesh-cache topology checks and seek behavior.
- UHD path tracing on 128 MiB storage bindings; exact whole/tiled comparison
  for noisy, denoised, perspective and orthographic frames, including edge tiles.
- A native cinematic impact scene rendered and encoded at 3840×2160 in strict
  mode, with a reviewed multi-frame sequence, recorded adapter/settings and
  measured memory/time. A tiny scene or a still frame cannot prove this gate.
- Existing compatibility tests, formatting, Clippy, MSRV and release build.

## Implementation status and schema review

The acceptance ledger, not this prose, records verified progress. Proposal
acceptance requires the final executable XSD/Schematron changes, matching Rust
validation, conformance fixtures, runnable examples and documentation. Runtime
interfaces not yet integrated remain explicitly pending in that ledger.

Schema-design review currently verifies the additive version policy, explicit
identities/ownership, time and spatial units, finite values, resource limits,
cache format and UHD behavior. The exact attribute inventory above reconciles
the cinematic element fields/defaults and relevant object/camera bindings with
the executable XSD. **Complete semantic-validator coverage and the final
rule scorecard remain pending implementation reconciliation** (the Schematron has 294 assertions with the rules of this section,
counted by parsing the file: `grep -c` of `sch:assert` counts closing tags too; 114 of them are in the
cinematic families OCN 14, P3D 11, CRT 17, PYRO 11, VOL 10, BH 8, FRX 15, VOX 15, PYC 6, MSQ 4 and GEO 3, and the rest are
sr-core's own: the rules R, C, V, MOV, PEN and TXT; sr-core 1.3.0 as vendored has 246 and carries the other cinematic
families, and the 47 that it does not (BH1 to BH8, FRX5 to FRX15, CRT10 to CRT17, PYRO9 to PYRO11, VOX1 to VOX15, PYC5 and PYC6) are this repository's. At commit 349d371,
before sr-core 1.3.0 was vendored, the file had 228, and at fa63e5d 169, 66 in the cinematic families without BH). Inventory
agreement alone does not establish behavior or full acceptance. Existing metadata supplies scene provenance;
the new numerical data carries no new personal-information fields. Channel names
are machine identifiers and are not localized. No prior fields are deprecated.

The integration tests of `sr-sim`, `sr-eval`, `sr-model`, `sr-3d` and `scene-render` are built as a few binaries, one for
each area, and a test file that this document names (for example `crater_capture` or `impact_scenes`) is a module of the
binary of its area: `crates/<crate>/tests/<area>/<file>.rs`, run with `cargo test -p <crate> --test <area> <file>::`. The
tests that observe allocations (`crater_memory`, `terrain_memory`, `pyro_export_memory`, each with its own global
allocator), the timing tests (`perf`) and the long ignored `hero_hires` keep binaries of their own.
