//! Rendering of 2D documents: compositing, effects, text, vector art, maps, video, caches and their costs, and the golden frames.

mod common;

#[path = "raster_effects/cache_regressions.rs"]
mod cache_regressions;
#[path = "raster_effects/caches.rs"]
mod caches;
#[path = "raster_effects/clip_blend_doc.rs"]
mod clip_blend_doc;
#[path = "raster_effects/composite.rs"]
mod composite;
#[path = "raster_effects/effect_costs.rs"]
mod effect_costs;
#[path = "raster_effects/effect_reads.rs"]
mod effect_reads;
#[path = "raster_effects/effects.rs"]
mod effects;
#[path = "raster_effects/effects25d.rs"]
mod effects25d;
#[path = "raster_effects/golden.rs"]
mod golden;
#[path = "raster_effects/gpu_time.rs"]
mod gpu_time;
#[path = "raster_effects/maps.rs"]
mod maps;
#[path = "raster_effects/markers.rs"]
mod markers;
#[path = "raster_effects/morphology.rs"]
mod morphology;
#[path = "raster_effects/motion_blur.rs"]
mod motion_blur;
#[path = "raster_effects/output.rs"]
mod output;
#[path = "raster_effects/pass_costs.rs"]
mod pass_costs;
#[path = "raster_effects/quality.rs"]
mod quality;
#[path = "raster_effects/resample.rs"]
mod resample;
#[path = "raster_effects/shaders.rs"]
mod shaders;
#[path = "raster_effects/shutter_angle_node.rs"]
mod shutter_angle_node;
#[path = "raster_effects/srep67_serial.rs"]
mod srep67_serial;
#[path = "raster_effects/stage_times.rs"]
mod stage_times;
#[path = "raster_effects/stencil_effects.rs"]
mod stencil_effects;
#[path = "raster_effects/stills.rs"]
mod stills;
#[path = "raster_effects/stroke_text_doc.rs"]
mod stroke_text_doc;
#[path = "raster_effects/text.rs"]
mod text;
#[path = "raster_effects/vector.rs"]
mod vector;
#[path = "raster_effects/version_1_6.rs"]
mod version_1_6;
#[path = "raster_effects/video.rs"]
mod video;
#[path = "raster_effects/wiggle_path_smooth.rs"]
mod wiggle_path_smooth;
