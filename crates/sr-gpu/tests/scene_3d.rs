//! 3D documents on the rasteriser: scenes, lights, materials, particles, the ocean, craters, fracture, simulation, models and joints.

mod common;

#[path = "scene_3d/crater.rs"]
mod crater;
#[path = "scene_3d/fracture.rs"]
mod fracture;
#[path = "scene_3d/joint_pose_doc.rs"]
mod joint_pose_doc;
#[path = "scene_3d/joint_sockets_doc.rs"]
mod joint_sockets_doc;
#[path = "scene_3d/maps3d.rs"]
mod maps3d;
#[path = "scene_3d/material_unevenness_doc.rs"]
mod material_unevenness_doc;
#[path = "scene_3d/mesh_sequence.rs"]
mod mesh_sequence;
#[path = "scene_3d/model_select.rs"]
mod model_select;
#[path = "scene_3d/morph_by_name.rs"]
mod morph_by_name;
#[path = "scene_3d/ocean.rs"]
mod ocean;
#[path = "scene_3d/particles3d.rs"]
mod particles3d;
#[path = "scene_3d/safe_audit.rs"]
mod safe_audit;
#[path = "scene_3d/scene3d.rs"]
mod scene3d;
#[path = "scene_3d/shadow_catcher_doc.rs"]
mod shadow_catcher_doc;
#[path = "scene_3d/silent_3d.rs"]
mod silent_3d;
#[path = "scene_3d/sim.rs"]
mod sim;
#[path = "scene_3d/text3d_tracking.rs"]
mod text3d_tracking;
#[path = "scene_3d/three.rs"]
mod three;
#[path = "scene_3d/three_shadow_catcher.rs"]
mod three_shadow_catcher;
#[path = "scene_3d/three_unevenness.rs"]
mod three_unevenness;
#[path = "scene_3d/voxel_crater.rs"]
mod voxel_crater;
#[path = "scene_3d/voxels.rs"]
mod voxels;
