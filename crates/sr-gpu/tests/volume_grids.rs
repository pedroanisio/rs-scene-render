//! Media in the path tracer: the volume pipeline, the skip of empty space, the brick directory and the light grids.

mod common;

#[path = "volume_grids/light_grid.rs"]
mod light_grid;
#[path = "volume_grids/volume.rs"]
mod volume;
