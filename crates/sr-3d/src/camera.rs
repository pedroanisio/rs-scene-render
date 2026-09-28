//! The physical camera: pose, projection, depth of field and shake.
//!
//! Scene space is the frame's pixel space extended into depth: origin at the
//! frame's top-left corner on z = 0, +x right, +y down, +z away from the viewer
//! (scene-render conventions 2.1). The implicit camera (no `<camera>`) matches
//! the compositor's 2.5D projection: it sits on the frame's centre axis at
//! `zoom = (w/2) / tan(fov/2)` with a 60° horizontal field of view, looking
//! along +z with y down, so the z = 0 plane maps 1:1 onto the frame (2.2).
//! A `<camera>`'s x, y and z are absolute scene positions (2.3).

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
    let eye = p.position.unwrap_or(Vec3::new(w * 0.5, h * 0.5, -z)) + p.offset;
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
        _ => orientation(p.yaw, p.pitch, p.roll),
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

fn hash(n: i64, seed: u64) -> f32 {
    let mut x = (n as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15) ^ seed.wrapping_mul(0xD1B5_4A32_D192_ED03);
    x ^= x >> 31;
    x = x.wrapping_mul(0xBF58_476D_1CE4_E5B9);
    x ^= x >> 29;
    (x >> 40) as f32 / (1u64 << 24) as f32 * 2.0 - 1.0
}

/// 1D gradient-free value noise in [−1, 1], smooth.
fn noise1(t: f64, seed: u64) -> f32 {
    let i = t.floor();
    let f = (t - i) as f32;
    let u = f * f * (3.0 - 2.0 * f);
    let (a, b) = (hash(i as i64, seed), hash(i as i64 + 1, seed));
    a + (b - a) * u
}

fn fbm(t: f64, octaves: u32, seed: u64) -> f32 {
    let (mut v, mut amp, mut freq, mut norm) = (0.0, 1.0, 1.0, 0.0);
    for o in 0..octaves.max(1) {
        v += noise1(t * freq, seed.wrapping_add(o as u64 * 1013)) * amp;
        norm += amp;
        amp *= 0.5;
        freq *= 2.0;
    }
    v / norm
}

/// Camera shake at time `t`: (x, y offset in scene units, roll in degrees, zoom factor).
pub fn shake(
    amplitude: f32,
    frequency: f32,
    rotation: f32,
    zoom_amount: f32,
    octaves: u32,
    seed: u64,
    t: f64,
) -> (f32, f32, f32, f32) {
    let tt = t * frequency as f64;
    (
        fbm(tt, octaves, seed) * amplitude,
        fbm(tt, octaves, seed ^ 0x5bd1_e995) * amplitude,
        fbm(tt, octaves, seed ^ 0x1b87_3593) * rotation,
        1.0 + fbm(tt, octaves, seed ^ 0xcc9e_2d51) * zoom_amount,
    )
}
