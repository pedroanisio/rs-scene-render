# Next oracles of the crater in cells: an oblique impact on a slope, and curved ground (note only, no code)

Netuno, 2026-10-08. For Urano. What is tested now (flat; slopes of 10, 20 and 30 degrees with the ball along the normal; the pillar in the reach) and what the new oracles would
add, with what each would be compared with and where I expect it to fail. Written from the code as it is (`crater_cut_of`, `crater_axis`, `surface_along`, `VoxelOwner::aligned`),
not run.

## 1. An oblique impact on a slope

**What changes.** The ball keeps its speed along the surface's normal (so the law's crater is the same) and adds a tangential component. Three things in the wiring depend on it:

1. *The axis* is the normal of the surface of the cells (`crater_axis`), not the velocity: it should not move with the angle of approach. (A real oblique crater is not circular under about 15 degrees from the surface, and its ejecta are thrown downrange: neither is in the law the engine uses, which reads the normal speed only, nor in `Ejection`, whose directions are radial out of the axis. This is a stated limit, not a defect to find.)
2. *The centre* of the crater is the contact point, inside the ground by the step the ball went on, **along the velocity**, not along the normal: the point is displaced sideways by up to `|v| dt sin(phi)` = 0.36 m x sin(60) = 0.31 m (1.2 cells) from where the ball touched. `surface_along` then moves it along the **axis** to the surface, which keeps the lateral shift. It is small against a crest radius of 6.7 m (5 percent) and is the first thing the oracle could find.
3. *The law's speed* is `-owner_velocity . axis` (`aligned`): the normal speed. It should be the flat ground's at any angle of approach with the same normal speed.

**The document.** The slope of `scene_slope.rs` (20 degrees), the ball with the normal speed 86.6 m/s and a tangential speed `86.6 tan(phi)`: phi = 30 and 60 degrees, the tangent **down the slope** (in the plane the ground falls in) and **across it** (along z: this is the axis tilted out of the slope's plane that the ledger lists as untested, and it is not the axis that is tilted but the velocity; the axis stays in the plane by symmetry; the case that tilts the axis out of the plane is the ground that slopes along both x and z, below). The ball starts on its own line back from the point P of the surface, so that the first contact is at P to within the radius.

**What to assert** (each a number, from the document and not from the code under test):

| Assertion | Compared with | Expected |
|---|---|---|
| the law's volume | the flat ground's (100.92 m3) | 0.5 percent, whatever phi and whatever the tangent |
| the axis | the slope's normal, from the slope's angle | under 3 degrees (the staircase's 0.1, and the noise of an estimate over the crest radius) |
| the centre of the crater (the projected `spec.center`) | P, computed from the geometry of the document | under 0.5 m (two cells): this is where the sideways shift would show |
| the volume destroyed | the law's | the slope's residual (+5.4, +2.0, -1.3 percent at 10, 20, 30 degrees) and the 6.5 percent band, as the normal case |
| every destroyed cell | inside the crest radius plus the rim of the **axis through the centre**, and under the ceiling | exactly, as in the normal case |
| the thrown cells' net tangential momentum | zero | by the model: it documents that the cells carry no downrange bias; the law's own list of ejecta has some |
| every frame | no failure, no problem | through the last frame |

**Where I expect trouble.** (a) the centre, if the contact point's lateral offset at 60 degrees is over 0.5 m (it would say that `surface_along` should project along the velocity, or that the point should be the first-contact point of the sphere and not the manifold's mean); (b) at phi = 60 the ball **glances**: the first contact may be a cell edge of the staircase, not the face, and the impact watch's impulse threshold (`rest_threshold`) may notice the impact one step later, which moves the point; (c) a ball that skids may touch the ground twice in the step of the impact (the second is not a second cut: tested). If (a) or (b) show, the first design to try is the contact point moved back along the ball's velocity by the step's travel and then to the surface along the axis; to be tested, not assumed.

**Cost.** Four documents of 1.1 M cells, 7 s each in the `ci` profile; the slope's builder is there.

## 2. Curved ground

The kernel's removal is geometric and does not look at the ground's shape (it is tested against a brute force over the whole ground for a flat axis and a slanted one), so what curved ground tests is the **wiring's choices**: the axis estimate, the plane's projection, the rim's room, and the pieces. Four grounds, each a 30 m by 30 m by 12 m slab with a height field added and the ball of the same mass and speed coming down:

| Ground | Ball at | Why |
|---|---|---|
| a **hill**: a spherical cap of radius of curvature 20 m and height 4 m, then 10 m and 5 m | its top (axis vertical by symmetry) and 5 m off it (the axis is tilted by the local slope, about 14 degrees at 20 m) | the estimate over the crest radius (6.7 m) smooths a surface whose normal changes by 19 degrees across the ball's neighbourhood: the axis at the offset point is the **mean** normal of the cells within 6.7 m, not the local one |
| a **bowl** (the same cap upside down) | its bottom and off it | concave: the rim's "held up from below" has to find room on the walls, the ceiling and the crest radius reach beyond the bowl |
| a **ridge** (a half cylinder along z) | across it, 3 m off the line | a convex edge in one direction only: the axis is tilted in x and not in z |
| an **edge**: ground that stops (a cliff of 6 m) | 3 m from the edge | a part of the crater's region has no ground: what is destroyed is what exists; the part beyond the edge may come loose as a piece |

**What to assert.**

1. *The cut equals the brute force*: the destroyed set is exactly `{cells : r < crest + rim and a >= S(r) and a <= ceiling}` about the axis and centre that the wiring chose (the kernel's accessors; independent of `excavate`), as the unit test does for the flat and the slanted axis. This is exact, not a tolerance.
2. *The axis* against the analytic normal of the surface **at the impact point** (from the height field): within 5 degrees where the radius of curvature is 40 m, and the test **measures** the error at 20, 10 and 5 m and prints it; I expect the error of the estimate to grow as the curvature radius falls under the crest radius (at 5 m it is the mean over a ball bigger than the hill). If it exceeds 5 degrees at 20 m the estimator's radius is the thing to change: try the radius of a half crest radius, or the ball's own radius, and pick by the measured error against the analytic normal for the four curvatures (the estimate is a function of the radius, and the right radius is the one that reproduces the local normal of a sphere).
3. *The law's volume* reads the speed along that axis: the flat ground's 100.92 m3 within 0.5 percent (as in the slopes), so the **volume destroyed** compared with the law is the check of the kernel on a curved ground: expect the hill to destroy more (the cap above the plane within the reach is removed up to the ceiling: the brute force says how much, as an equality) and the bowl less.
4. *The rim's room*: the heap is complete (added = the uplift count) or the cut fails with the number it found. A failure is a **result** (a stated limit of the rim's algorithm on that curvature, with the number), not a pass or a fail of the test: the oracle records the curvatures that heap and those that do not.
5. *Conservation*: before + added = after + destroyed + pieces, cell by cell, every case; every cell has the palette of the asset or the rim.
6. *Every frame* through the last, as the rule says.

**Where I expect trouble.** The bowl and the edge: the rim's cells must be "held up from below" along the lattice axis nearest to minus the axis, which on a 30-degree wall is the vertical; cells on the far wall of a bowl may not be held up. The slope tests passed at 30 degrees, which is the wall of a bowl of radius 20 m at its edge only.

## 3. What not to do yet

- A ground that slopes along **x and z** at once (a plane tilted about a diagonal): the axis out of both lattice planes, which the ledger lists. It is a plane, so one document (the plane's normal from two angles) with the same assertions as the slope; cheap, and I would do it with the oblique ones.
- Slopes over 30 degrees: the staircase's steps are over half a cell tall for the horizontal run of a cell at 45 degrees; the "face of cells" snap and the rim's "held up" rule change character. A note for after the curved grounds, and it is likely to need a different `down()` than the nearest lattice direction.
- A moving owner (a crater in a body that is itself falling or turning): the dust's velocity and the thrown cells' velocities are in the owner's frame; the burst takes them through the owner's matrix at the impact (not its velocity). Not an oracle yet; a stated limit.

## 4. Order I would build them in

1. oblique, in the plane and across it, at 20 degrees (the builder exists; four documents): the centre and the law.
2. the plane tilted about a diagonal (one document): the axis out of the lattice planes.
3. the hill at its top and 5 m off, curvature radii 20 and 10 m: the axis error against the analytic normal, which decides the estimator's radius.
4. the bowl and the ridge.
5. the edge (the pieces).

Each is red-first by the same test that fails before the change it asks for; the first three are the likeliest to find something (the centre at 60 degrees, the axis on a diagonal, the estimator's radius).
