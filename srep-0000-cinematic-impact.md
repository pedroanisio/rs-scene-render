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

### Pyro simulation

Add an owned `<pyro>` child to `object3D primitive="volume"`. Exactly one volume
source is required: either `@volume` or one `<pyro>`, never both (VOL1). The object
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
| `pressureTolerance` | positive 1/second; 0.000001 | Maximum RMS divergence residual |
| `maxMemoryMiB`, `checkpointMemoryMiB` | integers 1–4096; 256 each | Separate solver/workspace and checkpoint budgets |
| `meshMemoryMiB` | integer 1–4096; 128 | Aggregate conservative charge for distinct mesh regions in this domain |
| `forceFields` | optional IDREFS | Physics force fields acting on this domain; absent selects all (R34 validates references) |
| `useForceFields` | boolean; true | Disable all scene force fields when false |
| `colliders` | optional IDREFS | Up to 4096 distinct rigid collision proxies; absent selects none (PYRO7) |
| `colliderThickness` | optional positive length; twice `voxelSize` | Thickness of referenced planes, in each plane's local units |

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
  the collision-count ceiling. More than 16 contacts in a step, more than 90 degrees of collider
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
| `emitterShape`, `emitterWidth`, `emitterHeight`, `emitterDepth`, `emitterRadius`, `emitterMesh` | point/box/sphere/mesh, default point; dimensions/radius default 1; mesh requires a mesh asset and is a surface source |
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
impact still at 1.5 seconds reports 101,272 physical triangles, 44.977 seconds GPU
frame time, 52.91 seconds wall time and 573,220 KiB peak host RSS on RTX 6000 Ada
Vulkan. The preceding expanded frame used 44.094 seconds GPU time, 55.77 seconds
wall time and 584,708 KiB host RSS. These single observations demonstrate a small
host-memory reduction, not a GPU speedup or sequence-throughput improvement.
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
collider references (at most 4096), and static collider geometry. Their independent
XSD/Schematron fixtures are `valid/particles3d.scene.xml` and `invalid/p3d1` through
`p3d6.scene.xml`. Budgets, finite transformed values, importer failures and device
precision limits remain runtime checks with explicit diagnostics.

### Ocean surfaces and impulses

**Implementation status:** The CPU solver, version-1.3 `<ocean>` node, owned
waves and impulses, image/mesh bathymetry, local-clock replay, native surface
meshes, reflective/transmissive rendering, and native foam/spray tracers are
implemented. The completed UHD film remains unverified.

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
| `maxMemoryMiB`, `checkpointMemoryMiB` | 256, 64 | Solver workspace and separate checkpoint ceiling; zero checkpoints disables retention |
| `meshMemoryMiB`, `surfaceMemoryMiB` | 128, 128 | Bathymetry decoding/sampling and generated surface geometry ceilings |
| `maxWork` | 100000000 | Work allowance per solver seek; also bounds bathymetry sampling and swell evaluation separately |

Memory attributes are at most 4096 MiB; all except checkpoint memory are positive.
Admission budgets cover the named operation, not aggregate GPU use or copies
retained by external API callers. Mesh importer internal dependency allocations
still need hardening; the flattened geometry is checked against its budget.

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
materials, lights, path tracing and shutter samples. OCN1–OCN4 enforce version,
reference kinds, finite/resolved solver input and static solver configuration.
Only pose and opacity are animatable; the six ocean corpus fixtures are checked
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
| `maxWork` | 100000000 | Per-request whitewater work ceiling, at most 1,000,000,000 |
| `foamMaterial`, `sprayMaterial` | absent | Optional scoped material references |

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
seek. Each substep and impulse charges eight units per cell (24 per substep for `order="2"`). At most four
million cells, 16,384 impulses and 4,096 retained checkpoints are admitted.
Resident input capacities are included in the memory ceiling, which is 256 bytes per cell (400 for `order="2"`: two more state copies and sweep row buffers; measured peak 158 and at most 278 bytes per cell at one million cells). Checkpoint memory
is separately bounded; zero disables retention. Failed seeks leave the published
frame and caches unchanged. Fractional samples never become canonical state,
and replay after checkpoint eviction is bit-identical in the conformance tests.

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

Rigid replay now admits optional checkpoints before cloning. The engine defaults
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

Particle collider references to crater objects now use the same object-space
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
**CRT5** (rigid-body compatibility). `tests/corpus/valid/crater.scene.xml` and
`tests/corpus/invalid/crt1.scene.xml` through `crt5.scene.xml` independently
exercise the XSD/Schematron and Rust validators.

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
material; **FRX4** rejects nonfinite numeric values. Runtime solid validation is
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

Fracture bakes use **SRPHYS03**. Its little-endian header contains the eight-byte
magic, f64 step/start, u64 2D-body and soft-body counts, each soft point count,
u64 3D-body count (including fragments), u64 fracture-event count, and u64 frame
count. Each row retains the version-2 2D/soft/3D pose fields, followed by one f64
participation flag per 3D body and one f64 fired flag per event. Flags must be
exactly zero or one. Counts must match the compiled scene. Geometry is reconstructed
from immutable scene assets; the cache stores state, not meshes. Documents without
fracture still write SRPHYS02; SRPHYS01/02 remain readable. Baking samples exact
physics boundaries without an added timestamp epsilon.

Particle and pyro collider references expand a released object into its enabled
pieces and stop using the retired source surface. The closed, centroid-relative
partition meshes already include object scale; world-space piece poses therefore
replace the original object's transform. Particle sweeps use the displacement and
quaternion difference between sampled poses. Smoke obstacles transform each piece
into domain coordinates and prescribe its translation/rotation boundary velocity,
including reflected or rotated domains. Both paths work with live and SRPHYS03
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
validation. The mesher now emits crossed edges on each first grid plane; omitting
those faces previously left holes when the surface approached the grid boundary.
Tests verify closure across sphere sizes, bounded admission, release-time animated
blob sampling and raster/path-traced silhouette/hole preservation with replay.
Trigonometric seam canonicalization applies only to round generated primitives;
text, path and clay vertices keep their generated coordinates.

Ordinary text, path and clay solids now also supply particle and smoke colliders,
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
and default for each cinematic element. “Optional; absent” means that XSD
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
| `pressureIterations` | xs:positiveInteger; maxInclusive=10000 | Default `200` |
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

### `pyroImpulseType`

Also includes `pyroShape`, inventoried below.

| Attribute | XSD type or inline restriction | Presence/default |
|---|---|---|
| `time` | nonNegativeDecimal | Required |
| `density` | nonNegativeDecimal | Default `0` |
| `temperature` | nonNegativeDecimal | Default `0` |
| `velocityX` | xs:double | Default `0` |
| `velocityY` | xs:double | Default `0` |
| `velocityZ` | xs:double | Default `0` |
| `expansion` | xs:double | Default `0` |

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
| `emitterShape` | xs:string; enumeration=point, enumeration=box, enumeration=sphere, enumeration=mesh | Default `point` |
| `shape` | xs:string; enumeration=sphere, enumeration=billboard, enumeration=streak, enumeration=mesh | Default `sphere` |
| `sizeEnd` | nonNegativeDecimal | Optional; absent |
| `color` | colorType | Default `#FFFFFFFF` |
| `colorEnd` | colorType | Optional; absent |
| `opacityEnd` | unitDecimal | Optional; absent |
| `sizeCurve` | curveType | Default `linear` |
| `colorCurve` | curveType | Default `linear` |

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
| `maxWork` | xs:positiveInteger; maxInclusive=1000000000 | Default `100000000` |
| `boundary` | xs:string; enumeration=closed, enumeration=open, enumeration=periodic | Default `closed` |
| `bathymetryEncoding` | xs:string; enumeration=red, enumeration=terrarium, enumeration=mapbox | Default `red` |
| `order` | xs:string; enumeration=1, enumeration=2 | Default `1` |
| `material` | xs:IDREF | Optional; absent |
| `bathymetry` | xs:IDREF | Optional; absent |

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
| `maxWork` | xs:positiveInteger; maxInclusive=1000000000 | Default `100000000` |
| `foamMaterial` | xs:IDREF | Optional; absent |
| `sprayMaterial` | xs:IDREF | Optional; absent |

### `craterType`

| Attribute | XSD type or inline restriction | Presence/default |
|---|---|---|
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
| `interiorMaterial` | xs:IDREF | Required |
| `interiorUvScale` | positiveDecimal | Default `1` |
| `impulseX` | xs:double | Default `0` |
| `impulseY` | xs:double | Default `0` |
| `impulseZ` | xs:double | Default `0` |
| `radialImpulse` | nonNegativeDecimal | Default `0` |
| `maxMemoryMiB` | xs:positiveInteger; maxInclusive=4096 | Default `256` |

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
| `primitive` | xs:string; enumeration=sphere, enumeration=box, enumeration=plane, enumeration=mesh, enumeration=cylinder, enumeration=cone, enumeration=torus, enumeration=capsule, enumeration=text, enumeration=extrude, enumeration=clay, enumeration=map, enumeration=globe, enumeration=volume | Required |
| `material` | xs:IDREF | Optional; absent |
| `mesh` | xs:IDREF | Optional; absent |
| `volume` | xs:IDREF | Optional; absent |
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


## Conformance and acceptance

The runnable native combination example is
[`examples/cinematic-impact/impact.scene.xml`](examples/cinematic-impact/impact.scene.xml).
It authors an oblique approach, submerged crater, shallow-water displacement,
1,500 ejecta particles and a thermal smoke plume at 3840×2160. Its units and
timings are artistic, explicitly identified in scene metadata. It is a workload
and integration example, not a validated scientific Chicxulub reconstruction or
an accepted cinematic-quality film. The evaluator integration test exercises
pre-impact, impact, late settling and reverse replay; production-size execution
is practical with `cargo test --release -p sr-eval --test cinematic_impact`.
The full encoded-sequence gate below remains independently required.

The accompanying conformance suite must cover all of the following:

- Valid minimal examples for each new element, every enum branch, default
  expansion, bad references, version gates, nonfinite numbers and invalid bounds.
- Independent cache fixtures, every truncated header/payload boundary, invalid
  transforms, duplicate grids/bricks, total resource budgets and deterministic
  serialization. Compare OpenVDB samples/transforms to independent reference data.
- Uniform-density transmittance against Beer–Lambert, transformed media,
  overlapping media in reversed order, inside-volume cameras, mesh occlusion,
  emission, colored scattering and shadowed smoke at changing viewpoints.
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
31-rule scorecard remain pending implementation reconciliation.** Inventory
agreement alone does not establish behavior or full acceptance. Existing metadata supplies scene provenance;
the new numerical data carries no new personal-information fields. Channel names
are machine identifiers and are not localized. No prior fields are deprecated.
