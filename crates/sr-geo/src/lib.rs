//! # sr-geo
//!
//! Geography for scene-render maps.
//!
//! * [`data`] reads GeoJSON, TopoJSON, KML and GPX into features.
//! * [`sphere`] and [`clip`] do spherical geometry: rotation, great-circle
//!   arcs, area, containment, and clipping to the antimeridian or a small
//!   circle with polygons rejoined along the clip edge.
//! * [`project`] projects features to the plane (Mercator, Web Mercator,
//!   equirectangular, Equal Earth, Natural Earth, Albers, Lambert conformal
//!   conic, orthographic, stereographic, azimuthal equal-area and
//!   equidistant), resampling edges so great circles stay curved.
//! * [`view`] is the map camera: centre, zoom and rotation, fitting, and
//!   smooth "fly to" moves (van Wijk and Nuij).
//!
//! The spherical algorithms are ported from d3-geo 3.1 (ISC licence,
//! © Mike Bostock) and checked against it.

pub mod clip;
pub mod data;
pub mod project;
pub mod sphere;
pub mod view;
