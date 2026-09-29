//! The physical camera: pose, projection, depth of field and shake.
//!
//! Scene space is the frame's pixel space extended into depth: origin at the
//! frame's top-left corner on z = 0, +x right, +y down, +z away from the viewer
//! (scene-render conventions 2.1). The implicit camera (no `<camera>`) matches
//! the compositor's 2.5D projection: it sits on the frame's centre axis at
//! `zoom = (w/2) / tan(fov/2)` with a 60° horizontal field of view, looking
//! along +z with y down, so the z = 0 plane maps 1:1 onto the frame (2.2).
//! A `<camera>`'s x, y and z are absolute scene positions (2.3), or positions in
//! its parent's frame when it is parented (5.4).

use glam::{Mat4, Vec3};

/// Camera inputs, already resolved from the document.
#[derive(Clone, Debug, PartialEq)]
pub struct CameraParams {
    /// Horizontal field of view, degrees.
    pub fov: f32,
    pub orthographic: bool,
    /// Visible height in scene units for orthographic cameras (default: the frame height).
    pub ortho_height: Option<f32>,
    pub near: f32,
    pub far: f32,
    /// Absolute eye position in scene space; `None` is the implicit camera's eye.
    pub position: Option<Vec3>,
    /// Added to the eye (camera shake).
    pub offset: Vec3,
    /// Degrees.
    pub yaw: f32,
    pub pitch: f32,
    pub roll: f32,
    /// World point to look at.
    pub target: Option<Vec3>,
    /// World matrix of the parent frame that `position`, `yaw`, `pitch` and `roll` are in.
    pub frame: Option<Mat4>,
}

impl Default for CameraParams {
    fn default() -> CameraParams {
        CameraParams {
            fov: 60.0,
            orthographic: false,
            ortho_height: None,
            near: 0.1,
            far: 10000.0,
            position: None,
            offset: Vec3::ZERO,
            yaw: 0.0,
            pitch: 0.0,
            roll: 0.0,
            target: None,
            frame: None,
        }
    }
}

/// A resolved camera for one frame.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CameraView {
    /// World → camera (camera space: x right, y down, z forward).
    pub view: Mat4,
    /// Camera → clip, reverse-Z (near maps to depth 1, far to 0).
    pub proj: Mat4,
    pub eye: Vec3,
    /// Horizontal focal length in pixels.
    pub focal_px: f32,
    pub near: f32,
    pub far: f32,
    pub orthographic: bool,
}

impl CameraView {
    pub fn view_proj(&self) -> Mat4 {
        self.proj * self.view
    }
    /// Camera-space depth (distance along the view axis) of a world point.
    pub fn depth_of(&self, p: Vec3) -> f32 {
        self.view.transform_point3(p).z
    }
}

/// Distance of the default camera from the z = 0 plane.
pub fn zoom(width: f32, fov: f32) -> f32 {
    (width * 0.5) / (fov.to_radians() * 0.5).tan()
}

/// The camera looking along +z with y down, rotated by yaw (about y), pitch (about x) and roll (about z).
fn orientation(yaw: f32, pitch: f32, roll: f32) -> Mat4 {
    // Rx(θ) turns +z towards −y (up): positive pitch looks up; positive yaw turns towards +x
    Mat4::from_rotation_y(yaw.to_radians())
        * Mat4::from_rotation_x(pitch.to_radians())
        * Mat4::from_rotation_z(roll.to_radians())
}

/// Resolves a camera for a `w`×`h` frame.
pub fn resolve(p: &CameraParams, w: f32, h: f32) -> CameraView {
    let fov = p.fov.clamp(0.1, 179.0);
    let z = zoom(w, fov);
    let local_eye = p.position.unwrap_or(Vec3::new(w * 0.5, h * 0.5, -z)) + p.offset;
    let (eye, frame_rot) = match p.frame {
        Some(f) => (f.transform_point3(local_eye), Mat4::from_quat(f.to_scale_rotation_translation().1)),
        None => (local_eye, Mat4::IDENTITY),
    };
    let rot = match p.target {
        Some(t) if (t - eye).length_squared() > 1e-8 => {
            // look-at with y down as "up" in camera space
            let f = (t - eye).normalize();
            let up_hint = if f.y.abs() > 0.999 { Vec3::Z } else { Vec3::Y };
            let r = up_hint.cross(f).normalize();
            let r = -r;
            let d = f.cross(r).normalize();
            let d = -d;
            Mat4::from_cols(r.extend(0.0), d.extend(0.0), f.extend(0.0), glam::Vec4::W)
                * Mat4::from_rotation_z(p.roll.to_radians())
        }
        _ => frame_rot * orientation(p.yaw, p.pitch, p.roll),
    };
    let cam_to_world = Mat4::from_translation(eye) * rot;
    let view = cam_to_world.inverse();
    let (n, f) = (p.near.max(1e-4), p.far.max(p.near + 1e-3));
    let proj = if p.orthographic {
        let oh = p.ortho_height.unwrap_or(h).max(1e-3);
        let ow = oh * w / h;
        Mat4::from_cols_array(&[
            2.0 / ow,
            0.0,
            0.0,
            0.0,
            0.0,
            -2.0 / oh,
            0.0,
            0.0,
            0.0,
            0.0,
            -1.0 / (f - n),
            0.0,
            0.0,
            0.0,
            f / (f - n),
            1.0,
        ])
    } else {
        let fx = 2.0 * z / w;
        let fy = 2.0 * z / h;
        // clip = (fx x, −fy y, near·far/(far−near) − near/(far−near)·z, z)
        Mat4::from_cols_array(&[
            fx,
            0.0,
            0.0,
            0.0,
            0.0,
            -fy,
            0.0,
            0.0,
            0.0,
            0.0,
            -n / (f - n),
            1.0,
            0.0,
            0.0,
            n * f / (f - n),
            0.0,
        ])
    };
    CameraView { view, proj, eye, focal_px: z, near: n, far: f, orthographic: p.orthographic }
}

/// Horizontal field of view (degrees) of a lens of `focal_mm` on a sensor `sensor_mm` wide.
pub fn fov_of_lens(focal_mm: f32, sensor_mm: f32) -> f32 {
    2.0 * (sensor_mm / (2.0 * focal_mm)).atan().to_degrees()
}

/// Focal length (mm) for a horizontal field of view on a sensor `sensor_mm` wide.
pub fn lens_of_fov(fov: f32, sensor_mm: f32) -> f32 {
    sensor_mm / (2.0 * (fov.to_radians() * 0.5).tan())
}

/// Depth-of-field constant: the circle of confusion in pixels is
/// `k · |1/focus − 1/depth|` with depths in scene units (100 per metre).
pub fn coc_scale(focal_mm: f32, f_stop: f32, sensor_mm: f32, width_px: f32) -> f32 {
    // c(mm) = f²/N · |1/zf − 1/z| with z in mm; 1 scene unit = 10 mm
    (focal_mm * focal_mm / f_stop.max(0.1)) * (width_px / sensor_mm.max(1e-3)) / 10.0
}

/// Camera shake from its four fractal noise values N₀…N₃ (channels 0 to 3 at
/// frequency · t): (x, y offset in scene units, roll in degrees, zoom factor). The camera
/// moves amplitude · N₀ right and amplitude · N₁ down, rolls rotation · N₂ and zooms by 1 + zoom · N₃.
pub fn shake(amplitude: f32, rotation: f32, zoom_amount: f32, n: [f64; 4]) -> (f32, f32, f32, f32) {
    (n[0] as f32 * amplitude, n[1] as f32 * amplitude, n[2] as f32 * rotation, 1.0 + n[3] as f32 * zoom_amount)
}
