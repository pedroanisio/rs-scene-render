//! The path tracer: light through water and glass, foam mixed into the water, the specular lobe, the horizon, instances and tiles, and the geodesic pass.

mod common;

#[path = "path_tracer/foam_albedo.rs"]
mod foam_albedo;
#[path = "path_tracer/geodesic.rs"]
mod geodesic;
#[path = "path_tracer/geodesic_scene.rs"]
mod geodesic_scene;
#[path = "path_tracer/horizon_specks.rs"]
mod horizon_specks;
#[path = "path_tracer/pathtrace_instances.rs"]
mod pathtrace_instances;
#[path = "path_tracer/pathtrace_tiles.rs"]
mod pathtrace_tiles;
#[path = "path_tracer/specular_lobe.rs"]
mod specular_lobe;
#[path = "path_tracer/water_light.rs"]
mod water_light;
#[path = "path_tracer/water_oracle.rs"]
mod water_oracle;
