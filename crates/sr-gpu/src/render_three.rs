//! 3D objects in the compositor: every `object3D` under one parent renders
//! together as one 3D pass (shared depth, lights and shadows) at the paint
//! position of the first, and composites as a layer. The pass runs when the
//! compositor reaches that position, so transmissive surfaces refract the
//! layers already drawn behind it.

use super::render_fx::element_props;
use super::*;
use crate::three::{Dof, Draw3, Env3, Light3, LightKind, Maps, MeshSrc, Scene3, SplatDraw, ThreeEngine};
use crate::vector::Attrs;
use glam::{Mat4, Vec3};
use sr_3d::camera::{self, CameraParams, CameraView};
use sr_3d::{AlphaMode, Asset, MaterialParams};

enum LoadedMesh {
    Static(Arc<Result<Asset, String>>),
    Sequence(sr_eval::mesh_sequence::Loaded),
}
impl LoadedMesh {
    fn model(&self) -> Option<&sr_3d::Model> {
        match self {
            Self::Static(a) => match &**a {
                Ok(Asset::Model(m)) => Some(m),
                _ => None,
            },
            Self::Sequence(s) => s.frame.model.as_deref(),
        }
    }
    fn splats(&self) -> Option<&sr_3d::Splats> {
        match self {
            Self::Static(a) => match &**a {
                Ok(Asset::Splats(s)) => Some(s),
                _ => None,
            },
            _ => None,
        }
    }
    fn opacity(&self) -> f32 {
        match self {
            Self::Static(_) => 1.,
            Self::Sequence(s) => s.frame.opacity,
        }
    }
}

/// Replaces the frame camera (360 cube faces, stereo eyes).
#[derive(Clone, Copy, Debug)]
pub struct ViewOverride {
    /// Face rotation applied after the viewport camera's own orientation.
    pub face: Mat4,
    /// Eye offset along the camera's right axis (scene units).
    pub eye_shift: f32,
    /// Horizontal field of view (degrees) of the square face.
    pub fov: f32,
    /// Project frame size: the viewport camera's eye sits where it would for this frame.
    pub frame: [f32; 2],
}

/// The 360 reprojection pipeline.
pub struct SpherePipe {
    pipe: wgpu::RenderPipeline,
    bgl: wgpu::BindGroupLayout,
    smp: wgpu::Sampler,
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct SphereU {
    faces: [[[f32; 4]; 4]; 6],
    mode: [f32; 4],
    region: [f32; 4],
}

/// Face k: face space → viewport camera space (front, right, back, left, up, down).
pub fn face_rotations() -> [Mat4; 6] {
    let r = std::f32::consts::FRAC_PI_2;
    [
        Mat4::IDENTITY,
        Mat4::from_rotation_y(r),
        Mat4::from_rotation_y(2.0 * r),
        Mat4::from_rotation_y(-r),
        Mat4::from_rotation_x(r),
        Mat4::from_rotation_x(-r),
    ]
}

/// Camera extras beyond the view.
pub(super) struct CamExtras {
    pub exposure: f32,
    pub dof: Option<Dof>,
    pub lens_k1: f32,
    /// Screen-space ambient occlusion (radius, intensity) and reflections.
    pub ao: Option<[f32; 2]>,
    pub ssr: bool,
    /// Path tracing (camera renderer="pathtrace").
    pub path: Option<crate::pathtrace::PathOpts>,
}

impl CamExtras {
    /// Hash resolved values, including depth of field's animated focus target.
    pub(super) fn hash(&self) -> u64 {
        h(&[
            self.exposure.to_bits() as u64,
            self.lens_k1.to_bits() as u64,
            self.ssr as u64,
            self.dof.map_or(0, |d| {
                h(&[
                    1,
                    d.coc_scale.to_bits() as u64,
                    d.focus.to_bits() as u64,
                    d.max_coc.to_bits() as u64,
                    d.blades as u64,
                ])
            }),
            self.ao.map_or(0, |a| h(&[1, a[0].to_bits() as u64, a[1].to_bits() as u64])),
            self.path.map_or(0, |p| h(&[1, p.samples as u64, p.bounces as u64, p.denoise as u64])),
        ])
    }
}

fn attrs<'a>(n: &'a sr_eval::FrameNode) -> Attrs<'a> {
    Attrs { e: &*n.elem, props: Some(&n.props) }
}

fn primitive_kind(n: &FrameNode) -> String {
    let a = attrs(n);
    if n.kind == "particles3D" {
        match a.str("shape").as_deref() {
            Some("mesh") => "mesh",
            Some("billboard") => "plane",
            Some("streak") => "cylinder",
            _ => "sphere",
        }
        .into()
    } else if n.kind == "ocean" {
        "ocean".into()
    } else {
        a.str("primitive").unwrap_or_else(|| "box".into())
    }
}

fn flag(a: &Attrs, name: &str, d: bool) -> bool {
    a.num(name, d as u8 as f64) != 0.0
}

/// 2D affine embedded in 3D (acts on x and y, keeps z).
fn embed(a: &Affine) -> Mat4 {
    let [aa, b, c, d, e, f] = a.0.map(|v| v as f32);
    Mat4::from_cols_array(&[aa, b, 0.0, 0.0, c, d, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, e, f, 0.0, 1.0])
}

/// A node's own 3D transform: T(x, y, z) · Rz · Ry · Rx · S.
fn local3(a: &Attrs) -> Mat4 {
    let deg = |n: &str| (a.num(n, 0.0) as f32).to_radians();
    Mat4::from_translation(Vec3::new(a.num("x", 0.0) as f32, a.num("y", 0.0) as f32, a.num("z", 0.0) as f32))
        * Mat4::from_rotation_z(deg("rotation"))
        * Mat4::from_rotation_y(deg("rotationY"))
        * Mat4::from_rotation_x(deg("rotationX"))
        * Mat4::from_scale(Vec3::new(
            a.num("scaleX", 1.0) as f32,
            a.num("scaleY", 1.0) as f32,
            a.num("scaleZ", 1.0) as f32,
        ))
}

/// Pose of a camera or light: T(x, y, z) · R_yaw · R_pitch · R_roll.
fn pose3(a: &Attrs) -> Mat4 {
    let deg = |n: &str| (a.num(n, 0.0) as f32).to_radians();
    Mat4::from_translation(Vec3::new(a.num("x", 0.0) as f32, a.num("y", 0.0) as f32, a.num("z", 0.0) as f32))
        * Mat4::from_rotation_y(deg("yaw"))
        * Mat4::from_rotation_x(deg("pitch"))
        * Mat4::from_rotation_z(deg("roll"))
}

/// The 3D parent of an element and its influence: `@parent`, else the target of a
/// `transformConstraint type="parent"`.
fn parent3(e: &dyn Element) -> Option<(String, f32)> {
    if let Some(AttrValue::Str(pid)) = e.get_attr("parent") {
        return Some((pid.to_string(), 1.0));
    }
    sr_model::element::children(e).into_iter().find_map(|c| {
        let ca = Attrs { e: c, props: None };
        (c.element_name() == "transformConstraint" && ca.str("type").as_deref() == Some("parent"))
            .then(|| ca.str("target").map(|t| (t, ca.num("influence", 1.0).clamp(0.0, 1.0) as f32)))
            .flatten()
    })
}

/// `m` blended with the identity by `w` (constraint influence).
fn toward(m: Mat4, w: f32) -> Mat4 {
    if w >= 1.0 {
        return m;
    }
    let (s, r, t) = m.to_scale_rotation_translation();
    Mat4::from_scale_rotation_translation(Vec3::ONE.lerp(s, w), glam::Quat::IDENTITY.slerp(r, w), t * w)
}

/// The document's lights.
pub(super) fn doc_lights(p: &Program) -> &[m::Light] {
    p.scene.lights.as_ref().map(|l| &l.lights[..]).unwrap_or(&[])
}

impl Renderer {
    /// A light's resolved transform dependencies, including targets outside the cached subtree.
    /// Resolve them with the drawing path so chained parents, cameras and simulated 3D poses
    /// contribute their actual world transforms rather than only their authored attributes.
    pub(super) fn light_transform_hash(ctx: &Ctx, id: &str) -> Option<u64> {
        let lights = doc_lights(ctx.p);
        let light = lights.iter().find(|l| l.id == id)?;
        let a = Attrs { e: light, props: element_props(ctx.g, id) };
        let world = Self::pose_world(ctx.g, lights, light, &a, 0);
        let mut words: Vec<u64> = world.to_cols_array().iter().map(|v| v.to_bits() as u64).collect();
        for c in sr_model::element::children(light) {
            if c.element_name() != "transformConstraint" {
                continue;
            }
            let ca = Attrs { e: c, props: None };
            if let Some(target) = ca.str("target").and_then(|t| Self::target_point(ctx.g, lights, &t)) {
                words.extend(target.to_array().map(|v| v.to_bits() as u64));
            }
        }
        Some(h(&words))
    }

    pub(super) fn three_engine(&mut self) -> &mut ThreeEngine {
        let g = &self.gpu;
        self.three.get_or_insert_with(|| {
            Box::new(ThreeEngine::new_with(g.device.clone(), g.queue.clone(), g.gbuffer_depth, g.scalar_target))
        })
    }

    /// World matrix of a 3D object: its parent's full 3D world (an object, camera or light;
    /// a 2D parent contributes its 2D world), else the enclosing 2D world.
    fn world3(g: &FrameGraph, lights: &[m::Light], i: usize, depth: u32) -> Mat4 {
        let n = &g.nodes[i];
        // a rigid body's simulated pose replaces its own transform and parent
        if let Some(m) = &n.pose3 {
            return Mat4::from_cols_array(&m.map(|v| v as f32));
        }
        let own = local3(&attrs(n));
        if let Some(pw) = Self::parent_world(g, lights, &*n.elem, depth) {
            return pw * own;
        }
        match n.parent {
            Some(p) => embed(&g.nodes[p as usize].world) * own,
            None => own,
        }
    }

    /// World matrix of the frame a parented element's own transform is in,
    /// weighted by the parent constraint's influence.
    fn parent_world(g: &FrameGraph, lights: &[m::Light], e: &dyn Element, depth: u32) -> Option<Mat4> {
        if depth >= 32 {
            return None;
        }
        let (pid, w) = parent3(e)?;
        Some(toward(Self::world_of(g, lights, &pid, depth + 1)?, w))
    }

    /// World matrix of the node or light `id`: objects, cameras and lights in 3D, other nodes as their 2D world.
    fn world_of(g: &FrameGraph, lights: &[m::Light], id: &str, depth: u32) -> Option<Mat4> {
        if let Some(j) = g.nodes.iter().position(|m| &*m.id == id) {
            let n = &g.nodes[j];
            return Some(match n.kind {
                kind if sr_eval::draws_in_3d(kind) => Self::world3(g, lights, j, depth),
                // the camera as it looks: its pose, look-at target and shake
                "camera" => {
                    let cp = Self::cam_params(g, lights, j, depth);
                    camera::resolve(&cp, g.size[0] as f32, g.size[1] as f32).view.inverse()
                }
                _ => embed(&n.world),
            });
        }
        let l = lights.iter().find(|l| l.id == id)?;
        let a = Attrs { e: l as &dyn Element, props: element_props(g, &l.id) };
        Some(Self::pose_world(g, lights, l, &a, depth))
    }

    /// World matrix of a camera or light: its pose in its parent's frame.
    fn pose_world(g: &FrameGraph, lights: &[m::Light], e: &dyn Element, a: &Attrs, depth: u32) -> Mat4 {
        let own = pose3(a);
        Self::parent_world(g, lights, e, depth).map(|p| p * own).unwrap_or(own)
    }

    /// World position of a node for look-at and focus targets.
    fn target_point(g: &FrameGraph, lights: &[m::Light], id: &str) -> Option<Vec3> {
        Self::world_point(g, lights, id, 0)
    }

    fn world_point(g: &FrameGraph, lights: &[m::Light], id: &str, depth: u32) -> Option<Vec3> {
        if let Some(n) =
            g.nodes.iter().find(|m| &*m.id == id).filter(|n| !(sr_eval::draws_in_3d(n.kind) || n.kind == "camera"))
        {
            let p = n.world.apply(n.anchor);
            return Some(Vec3::new(p[0] as f32, p[1] as f32, n.three_d.map(|t| t[0]).unwrap_or(0.0) as f32));
        }
        Some(Self::world_of(g, lights, id, depth)?.transform_point3(Vec3::ZERO))
    }

    /// Horizontal field of view of a camera node: `fov`, or `focalLength` over `sensorWidth`.
    fn cam_fov(a: &Attrs) -> f32 {
        match a.opt("focalLength") {
            Some(f) => camera::fov_of_lens(f as f32, a.num("sensorWidth", 36.0) as f32),
            None => a.num("fov", 60.0) as f32,
        }
    }

    /// Camera inputs of camera node `ci`: projection, pose (in its parent's frame when parented),
    /// look-at target and shake.
    fn cam_params(g: &FrameGraph, lights: &[m::Light], ci: usize, depth: u32) -> CameraParams {
        let n = &g.nodes[ci];
        let a = attrs(n);
        let mut cp = CameraParams {
            fov: Self::cam_fov(&a),
            orthographic: a.str("projection").as_deref() == Some("orthographic"),
            ortho_height: a.opt("orthoHeight").map(|v| v as f32),
            near: a.num("near", 0.1) as f32,
            far: a.num("far", 10000.0) as f32,
            // absolute scene position; the defaults put the eye at the frame's top-left corner on z = 0
            position: Some(Vec3::new(a.num("x", 0.0) as f32, a.num("y", 0.0) as f32, a.num("z", 0.0) as f32)),
            yaw: a.num("yaw", 0.0) as f32,
            pitch: a.num("pitch", 0.0) as f32,
            roll: a.num("roll", 0.0) as f32,
            // a parented camera's position and angles are in its parent's frame
            frame: Self::parent_world(g, lights, &*n.elem, depth),
            ..Default::default()
        };
        if let Some(t) = a.str("target").filter(|_| depth < 32) {
            cp.target = Self::world_point(g, lights, &t, depth + 1);
        }
        // shake children
        for c in sr_model::element::children(&*n.elem) {
            if c.element_name() != "shake" {
                continue;
            }
            let sa = Attrs { e: c, props: None };
            let t = n.local_time;
            if t < sa.num("start", 0.0) || sa.opt("end").map(|e| t >= e).unwrap_or(false) {
                continue;
            }
            // four fractal channels at frequency · t, with @seed or the project's seed
            let seed = sa.opt("seed").map(|s| s as u64).unwrap_or(g.seed);
            let (f, oct) = (sa.num("frequency", 2.0), sa.num("octaves", 2.0).max(1.0) as u32);
            let noise = [0, 1, 2, 3].map(|k| sr_eval::rng::fractal(seed, k, f * t, oct));
            let (dx, dy, droll, dz) = camera::shake(
                sa.num("amplitude", 10.0) as f32,
                sa.num("rotation", 0.0) as f32,
                sa.num("zoom", 0.0) as f32,
                noise,
            );
            cp.offset += Vec3::new(dx, dy, 0.0);
            cp.roll += droll;
            cp.fov = (cp.fov / dz.max(0.05)).clamp(0.1, 179.0);
        }
        cp
    }

    /// The frame camera (the active camera node, else the default 2.5D camera).
    pub(super) fn camera3(&self, g: &FrameGraph, p: &Program, frame: [f32; 2]) -> (CameraView, CamExtras, [f32; 2]) {
        let lights = doc_lights(p);
        let mut cp = CameraParams::default();
        let mut ex = CamExtras { exposure: 1.0, dof: None, lens_k1: 0.0, ao: None, ssr: false, path: None };
        let mut focal_mm = camera::lens_of_fov(cp.fov, 36.0);
        let (mut sensor, mut fstop, mut focus, mut blades, mut dof_on, mut focus_target) =
            (36.0f32, 2.8f32, 1000.0f32, 0u32, false, None);
        if let Some(ci) = g.camera {
            let n = &g.nodes[ci as usize];
            let a = attrs(n);
            sensor = a.num("sensorWidth", 36.0) as f32;
            if flag(&a, "ambientOcclusion", false) {
                ex.ao = Some([a.num("aoRadius", 40.0) as f32, a.num("aoIntensity", 1.0) as f32]);
            }
            ex.ssr = flag(&a, "screenSpaceReflections", false);
            if a.str("renderer").as_deref() == Some("pathtrace") {
                ex.path = Some(crate::pathtrace::PathOpts {
                    samples: a.num("pathSamples", 64.0).clamp(1.0, 65536.0) as u32,
                    bounces: a.num("maxBounces", 4.0).clamp(1.0, 64.0) as u32,
                    denoise: flag(&a, "denoise", true),
                });
            }
            focal_mm = camera::lens_of_fov(Self::cam_fov(&a), sensor);
            cp = Self::cam_params(g, lights, ci as usize, 0);
            ex.exposure = 2f32.powf(a.num("exposure", 0.0) as f32);
            ex.lens_k1 = a.num("lensDistortion", 0.0) as f32;
            dof_on = flag(&a, "depthOfField", false);
            fstop = a.num("fStop", 2.8) as f32;
            focus = a.num("focusDistance", 1000.0) as f32;
            blades = a.num("apertureBlades", 0.0) as u32;
            focus_target = a.str("focusTarget");
        }
        let base = self.view_override.map(|o| o.frame).unwrap_or(frame);
        let mut view = camera::resolve(&cp, base[0], base[1]);
        let mut size = frame;
        if let Some(o) = self.view_override {
            // keep the viewport camera's eye; turn to the face and square the frustum
            let cam_to_world = view.view.inverse();
            let right = cam_to_world.transform_vector3(Vec3::X).normalize_or(Vec3::X);
            let eye = view.eye + right * o.eye_shift;
            let rot =
                Mat4::from_cols(cam_to_world.x_axis, cam_to_world.y_axis, cam_to_world.z_axis, glam::Vec4::W) * o.face;
            let face_view = (Mat4::from_translation(eye) * rot).inverse();
            let side = frame[0].max(1.0);
            let zoom = camera::zoom(side, o.fov);
            let (n, f) = (view.near, view.far);
            let fx = 2.0 * zoom / side;
            let proj = Mat4::from_cols_array(&[
                fx,
                0.0,
                0.0,
                0.0,
                0.0,
                -fx,
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
            ]);
            view = CameraView { view: face_view, proj, eye, focal_px: zoom, near: n, far: f, orthographic: false };
            size = [side, side];
        }
        if dof_on && !view.orthographic {
            if let Some(t) = focus_target.and_then(|t| Self::target_point(g, lights, &t)) {
                focus = view.depth_of(t).max(1.0);
            }
            ex.dof = Some(Dof {
                coc_scale: camera::coc_scale(focal_mm, fstop, sensor, size[0]),
                focus,
                max_coc: (size[0] / 40.0).clamp(4.0, 48.0),
                blades,
            });
        }
        (view, ex, size)
    }

    fn working_color(&self, v: Option<Value>, d: [f64; 4]) -> [f32; 4] {
        let c = match v {
            Some(Value::Color(c)) => self.working.from_literal(c),
            Some(Value::Str(s)) if s.starts_with("token:") => {
                self.tokens.get(&s[6..]).map(|c| self.working.from_literal(*c)).unwrap_or(d)
            }
            _ => d,
        };
        let l = self.working.to_linear([c[0], c[1], c[2]]);
        [l[0] as f32, l[1] as f32, l[2] as f32, c[3] as f32]
    }

    /// Linear sRGB → linear working values.
    fn lin_srgb(&self, c: [f32; 3]) -> [f32; 3] {
        let s = self.working.from_linear_srgb([c[0] as f64, c[1] as f64, c[2] as f64, 1.0]);
        let l = self.working.to_linear([s[0], s[1], s[2]]);
        [l[0] as f32, l[1] as f32, l[2] as f32]
    }

    fn literal_linear(&self, c: [f64; 4]) -> [f64; 4] {
        let s = self.working.from_literal(c);
        let l = self.working.to_linear([s[0], s[1], s[2]]);
        [l[0], l[1], l[2], s[3]]
    }

    fn map_texture(
        &mut self,
        plan: &mut Plan,
        base: &std::path::Path,
        uri: &str,
        srgb: bool,
        owner: &str,
    ) -> Option<Arc<crate::three::TexGpu>> {
        let path = match sr_model::assets::resolve(uri, base) {
            sr_model::assets::Resolved::Local(p) => p,
            sr_model::assets::Resolved::Remote(u) => {
                plan.stats.errors.push(format!("{owner}: remote texture {u} is not fetched while rendering"));
                return None;
            }
        };
        let key = format!("{}|{srgb}", path.display());
        if let Some(t) = self.three_engine().textures.get(&key) {
            return Some(t.clone());
        }
        match sr_media::still::open(&path) {
            Ok(still) => {
                let img = still.image.to_rgba8();
                let t = self.three_engine().upload_rgba8(img.width(), img.height(), img.as_raw(), srgb);
                self.three_engine().textures.insert(key, t.clone());
                Some(t)
            }
            Err(e) => {
                plan.stats.errors.push(format!("{owner}: cannot read texture {}: {e}", path.display()));
                None
            }
        }
    }

    /// A document material (animated values at this frame) with its maps.
    fn document_material(&mut self, plan: &mut Plan, ctx: &Ctx, id: &str) -> Option<(MaterialParams, Maps)> {
        let mat = ctx.p.scene.materials.as_ref()?.materials.iter().find(|m| m.id == id)?;
        let a = Attrs { e: mat as &dyn Element, props: element_props(ctx.g, id) };
        let base = Self::base_dir(ctx.p);
        let col = |r: &Self, n: &str, d: [f64; 4]| r.working_color(a.paint(n), r.literal_linear(d));
        let mut p = MaterialParams {
            base_color: col(self, "baseColor", [1.0; 4]),
            metallic: a.num("metallic", 0.0) as f32,
            roughness: a.num("roughness", 0.5) as f32,
            emissive: {
                let e = col(self, "emissive", [0.0, 0.0, 0.0, 1.0]);
                [e[0], e[1], e[2]]
            },
            emissive_strength: a.num("emissiveStrength", 1.0) as f32,
            opacity: a.num("opacity", 1.0) as f32,
            alpha_mode: match a.str("alphaMode").as_deref() {
                Some("mask") => AlphaMode::Mask,
                Some("blend") => AlphaMode::Blend,
                _ => AlphaMode::Opaque,
            },
            alpha_cutoff: a.num("alphaCutoff", 0.5) as f32,
            double_sided: flag(&a, "doubleSided", false),
            unlit: flag(&a, "unlit", false),
            clearcoat: a.num("clearcoat", 0.0) as f32,
            clearcoat_roughness: a.num("clearcoatRoughness", 0.0) as f32,
            transmission: a.num("transmission", 0.0) as f32,
            ior: a.num("ior", 1.5) as f32,
            thickness: a.num("thickness", 0.0) as f32,
            attenuation_color: {
                let c = col(self, "attenuationColor", [1.0; 4]);
                [c[0], c[1], c[2]]
            },
            attenuation_distance: a.opt("attenuationDistance").map(|v| v as f32).unwrap_or(f32::INFINITY),
            sheen_color: {
                let c = col(self, "sheenColor", [0.0, 0.0, 0.0, 1.0]);
                [c[0], c[1], c[2]]
            },
            sheen_roughness: a.num("sheenRoughness", 0.0) as f32,
            specular: a.num("specular", 1.0) as f32,
            specular_color: {
                let c = col(self, "specularColor", [1.0; 4]);
                [c[0], c[1], c[2]]
            },
            iridescence: a.num("iridescence", 0.0) as f32,
            iridescence_ior: a.num("iridescenceIor", 1.3) as f32,
            anisotropy: a.num("anisotropy", 0.0) as f32,
            anisotropy_rotation: (a.num("anisotropyRotation", 0.0) as f32).to_radians(),
            dispersion: a.num("dispersion", 0.0) as f32,
            normal_scale: a.num("normalScale", 1.0) as f32,
            unevenness: a.num("unevenness", 0.0) as f32,
            unevenness_scale: a.num("unevennessScale", 8.0) as f32,
            unevenness_seed: a.num("unevennessSeed", 0.0) as u32,
            displacement_scale: a.num("displacementScale", 0.0) as f32,
            uv_scale: [a.num("uvScaleX", 1.0) as f32, a.num("uvScaleY", 1.0) as f32],
            ..Default::default()
        };
        let mut maps: Maps = Default::default();
        for (slot, name, srgb) in [
            (0, "baseColorMap", true),
            (1, "normalMap", false),
            (2, "metallicRoughnessMap", false),
            (3, "occlusionMap", false),
            (4, "emissiveMap", true),
            (5, "displacementMap", false),
        ] {
            if let Some(uri) = a.str(name) {
                maps[slot] = self.map_texture(plan, &base, &uri, srgb, id);
            }
        }
        if let Some(uri) = a.str("materialX") {
            let path = match sr_model::assets::resolve(&uri, &base) {
                sr_model::assets::Resolved::Local(p) => p,
                sr_model::assets::Resolved::Remote(_) => base.join(&uri),
            };
            let key = path.display().to_string();
            let parsed = self.mtlx.entry(key).or_insert_with(|| Arc::new(sr_3d::mtlx::load(&path))).clone();
            match &*parsed {
                Ok(mx) => {
                    let mut q = mx.params.clone();
                    q.uv_scale = p.uv_scale;
                    q.double_sided = p.double_sided;
                    // MaterialX colours are linear sRGB; bring them into the working primaries
                    for c in [
                        &mut q.base_color[..3],
                        &mut q.emissive[..],
                        &mut q.sheen_color[..],
                        &mut q.attenuation_color[..],
                        &mut q.specular_color[..],
                    ] {
                        let l = self.lin_srgb([c[0], c[1], c[2]]);
                        c.copy_from_slice(&l);
                    }
                    p = q;
                    // MaterialX resolves filenames against its document already.
                    // Joining that directory again duplicates relative prefixes.
                    let dir = std::path::Path::new(".");
                    if let Some(f) = &mx.base_color_map {
                        maps[0] = self.map_texture(plan, dir, &f.display().to_string(), true, id);
                    }
                    if let Some(f) = &mx.normal_map {
                        maps[1] = self.map_texture(plan, dir, &f.display().to_string(), false, id);
                    }
                    if let Some(f) = &mx.roughness_map {
                        // a roughness image drives the green channel of the metallic-roughness slot
                        maps[2] = self.map_texture(plan, dir, &f.display().to_string(), false, id);
                        p.metallic = 0.0;
                    }
                    for (slot, texture) in mx.generated_maps.iter().enumerate() {
                        if let Some(texture) = texture {
                            let key = format!("mtlx:{}:{slot}", path.display());
                            maps[slot] = Some(match self.three_engine().textures.get(&key) {
                                Some(texture) => texture.clone(),
                                None => {
                                    let uploaded = self.three_engine().upload_texture(texture);
                                    self.three_engine().textures.insert(key, uploaded.clone());
                                    uploaded
                                }
                            });
                        }
                    }
                    for w in &mx.warnings {
                        plan.stats.unsupported.push(format!("{id}: {w}"));
                    }
                }
                Err(e) => plan.stats.errors.push(format!("{id}: {e}")),
            }
        }
        Some((p, maps))
    }

    /// The mesh of a clay object, re-extracted when its blobs, finish or boil frame change.
    fn clay_mesh(&mut self, n: &sr_eval::FrameNode) -> Option<Arc<crate::three::MeshGpu>> {
        let sr_eval::solid::Clay { blobs, finish, resolution: res } = sr_eval::solid::clay(n);
        let boil_frame = if finish.boil > 0.0 { (n.local_time * finish.boil as f64).floor() as i64 } else { 0 };
        let key = format!("clay|{}|{blobs:?}|{finish:?}|{res}|{boil_frame}", n.id);
        if let Some(m) = self.three_engine().meshes.get(&key) {
            return Some(m.clone());
        }
        let prim = sr_3d::clay::mesh(&blobs, &finish, res, n.local_time);
        if prim.indices.is_empty() {
            return None;
        }
        let m = self.three_engine().upload_mesh(&prim.vertices, &prim.indices);
        if let Some(old) = self.clay_keys.insert(n.id.clone(), key.clone()) {
            self.three_engine().meshes.remove(&old);
        }
        self.three_engine().meshes.insert(key, m.clone());
        Some(m)
    }

    fn primitive_mesh(
        &mut self,
        plan: &mut Plan,
        ctx: &Ctx,
        n: &sr_eval::FrameNode,
    ) -> Option<Arc<crate::three::MeshGpu>> {
        if n.kind == "ocean" {
            let Some(surface) = &n.sim_ocean else {
                // a failed solver is already reported as an error with its cause
                if !ctx.g.failed(&n.id) {
                    plan.stats.errors.push(format!("{}: ocean evaluation produced no surface", n.id));
                }
                return None;
            };
            if surface.mesh.indices.is_empty() {
                return None;
            }
            let vertex_bytes = surface.mesh.vertices.len() as u64 * std::mem::size_of::<sr_3d::Vertex>() as u64;
            let index_bytes = surface.mesh.indices.len() as u64 * 4;
            let cap = self.gpu.device.limits().max_buffer_size;
            if vertex_bytes > cap || index_bytes > cap {
                plan.stats.errors.push(format!("{}: ocean surface exceeds device buffer limits", n.id));
                return None;
            }
            return Some(self.three_engine().upload_mesh(&surface.mesh.vertices, &surface.mesh.indices));
        }
        let a = attrs(n);
        let kind = primitive_kind(n);
        let r = a.num("radius", 50.0) as f32;
        let segs = a.num("segments", 32.0).clamp(3.0, 512.0) as u32;
        let (w, hh) = (a.opt("width").map(|v| v as f32).unwrap_or(2.0 * r), a.opt("height").map(|v| v as f32));
        let depth = a.num("depth", 10.0) as f32;
        let bevel = a.num("bevel", 0.0) as f32;
        if kind == "clay" {
            return self.clay_mesh(n);
        }
        let key = match kind.as_str() {
            "text" => format!(
                "text|{}|{:?}|{:?}|{depth}|{bevel}|{}",
                n.text.as_deref().or(a.str("text").as_deref()).unwrap_or(""),
                a.str("font"),
                hh,
                a.num("tracking", 0.0)
            ),
            "extrude" => format!("extrude|{}|{depth}|{bevel}", a.str("path").unwrap_or_default()),
            other => format!("{other}|{r}|{segs}|{w}|{hh:?}|{depth}"),
        };
        if let Some(m) = self.three_engine().meshes.get(&key) {
            return Some(m.clone());
        }
        let prim = match kind.as_str() {
            "sphere" => Ok(sr_3d::prim::sphere(r, segs)),
            "box" => Ok(sr_3d::prim::cuboid(w, hh.unwrap_or(2.0 * r), depth)),
            // a plane is a grid of `segments` × `segments` quads: a displacement map needs its vertices to move
            "plane" => {
                Ok(sr_3d::prim::plane(w, hh.unwrap_or(2.0 * r), a.num("segments", 32.0).clamp(1.0, 1024.0) as u32))
            }
            "cylinder" => Ok(sr_3d::prim::cylinder(r, r, hh.unwrap_or(2.0 * r), segs)),
            "cone" => Ok(sr_3d::prim::cylinder(0.0, r, hh.unwrap_or(2.0 * r), segs)),
            // tube radius: half the height, else 0.35 of the ring radius
            "torus" => Ok(sr_3d::prim::torus(r, hh.map(|h| h * 0.5).unwrap_or(0.35 * r).min(r), segs)),
            "capsule" => Ok(sr_3d::prim::capsule(r, hh.unwrap_or(4.0 * r).max(2.0 * r), segs)),
            "extrude" => a
                .str("path")
                .ok_or_else(|| "extrude without @path".to_string())
                .and_then(|d| sr_3d::prim::path_polygons(&d, 0.25))
                .and_then(|polys| sr_3d::prim::extrude(&polys, depth, bevel)),
            "text" => {
                let text = n.text.as_deref().map(str::to_string).or(a.str("text")).unwrap_or_default();
                let size = hh.unwrap_or(100.0) as f64;
                let mut tc = std::mem::take(&mut self.text);
                let polys = crate::text::outline_polygons(
                    &mut tc,
                    ctx.p,
                    &text,
                    a.str("font").as_deref(),
                    size,
                    a.num("tracking", 0.0),
                    0.25,
                );
                self.text = tc;
                let mut polys: Vec<Vec<glam::Vec2>> = polys
                    .into_iter()
                    .map(|p| p.into_iter().map(|q| glam::Vec2::new(q[0] as f32, q[1] as f32)).collect())
                    .collect();
                // centre the block on the object's origin
                let (lo, hi) = polys
                    .iter()
                    .flatten()
                    .fold((glam::Vec2::splat(f32::MAX), glam::Vec2::splat(f32::MIN)), |(l, h), p| {
                        (l.min(*p), h.max(*p))
                    });
                let c = (lo + hi) * 0.5;
                polys.iter_mut().flatten().for_each(|p| *p -= c);
                sr_3d::prim::extrude(&polys, depth, bevel)
            }
            "mesh" => return None,
            other => Err(format!("unknown primitive {other}")),
        };
        match prim {
            Ok(p) => {
                let m = self.three_engine().upload_mesh(&p.vertices, &p.indices);
                self.three_engine().meshes.insert(key, m.clone());
                Some(m)
            }
            Err(e) => {
                plan.stats.errors.push(format!("{}: {e}", n.id));
                None
            }
        }
    }

    fn mesh_asset(&mut self, plan: &mut Plan, ctx: &Ctx, n: &sr_eval::FrameNode) -> Option<(String, LoadedMesh)> {
        match sr_eval::mesh_sequence::sample(ctx.p, n) {
            Ok(Some(sequence)) => {
                if let Some(model) = &sequence.frame.model {
                    let limits = self.gpu.device.limits();
                    if model.primitives.iter().any(|p| {
                        p.vertices.len() as u64 * std::mem::size_of::<sr_3d::Vertex>() as u64 > limits.max_buffer_size
                            || p.indices.len() as u64 * 4 > limits.max_buffer_size
                    }) || model.textures.iter().any(|t| {
                        t.width > limits.max_texture_dimension_2d || t.height > limits.max_texture_dimension_2d
                    }) {
                        plan.stats
                            .errors
                            .push(format!("{}: mesh sequence exceeds device geometry/texture limits", n.id));
                        return None;
                    }
                    for warning in &model.warnings {
                        plan.stats.unsupported.push(format!("{}: {warning}", n.id));
                    }
                }
                let prefix = format!("mesh-sequence:{}|", n.id);
                let key = format!("{prefix}{}|", sequence.key);
                let engine = self.three_engine();
                engine.meshes.retain(|k, _| !k.starts_with(&prefix) || k.starts_with(&key));
                engine.textures.retain(|k, _| !k.starts_with(&prefix) || k.starts_with(&key));
                return Some((key, LoadedMesh::Sequence(sequence)));
            }
            Ok(None) => {}
            Err(e) => {
                plan.stats.errors.push(format!("{}: {e}", n.id));
                return None;
            }
        }
        let a = attrs(n);
        let id = a.str("mesh")?;
        let key = n.asset.as_deref().map(str::to_string).unwrap_or_else(|| id.clone());
        let Some((AssetsChild::Mesh(ma), doc)) = self.asset(ctx.p, &key) else {
            plan.stats.errors.push(format!("{}: mesh asset {id} not found", n.id));
            return None;
        };
        let base = ctx.p.base_dirs.get(doc).cloned().unwrap_or_default();
        let path = match sr_model::assets::resolve(&ma.src, &base) {
            sr_model::assets::Resolved::Local(p) => p,
            sr_model::assets::Resolved::Remote(u) => {
                plan.stats.errors.push(format!("{}: remote mesh {u} is not fetched while rendering", n.id));
                return None;
            }
        };
        let key = path.display().to_string();
        let fmt = ma.format.map(|f| f.to_string());
        let asset = self
            .three_assets
            .entry(key.clone())
            .or_insert_with(|| Arc::new(sr_3d::import::load(&path, fmt.as_deref())))
            .clone();
        match &*asset {
            Ok(Asset::Model(m)) => {
                for w in &m.warnings {
                    let msg = format!("{id}: {w}");
                    if !plan.stats.unsupported.contains(&msg) {
                        plan.stats.unsupported.push(msg);
                    }
                }
            }
            Ok(Asset::Splats(_)) => {}
            Err(e) => plan.stats.errors.push(format!("{}: {e}", n.id)),
        }
        Some((key, LoadedMesh::Static(asset)))
    }

    /// Draws of one 3D object.
    /// A missing cache is distinct from a malformed or unreadable cache. Keep
    /// immutable frames shared across instances and bounded by bytes and entries.
    fn volume_cache(
        &mut self,
        path: &std::path::Path,
        format: &str,
    ) -> Result<Option<Arc<sr_volume::Volume>>, sr_volume::Error> {
        let key = (path.to_owned(), format.to_owned());
        if let Some(cached) = self.volume_assets.get(&key) {
            return cached.clone().map_err(|e| sr_volume::Error::Io(std::io::Error::other(e)));
        }
        let loaded = (|| {
            let file = match std::fs::File::open(path) {
                Ok(file) => file,
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
                Err(e) => {
                    return Err(sr_volume::Error::Io(std::io::Error::new(e.kind(), format!("{}: {e}", path.display()))))
                }
            };
            let cap = self.gpu.device.limits().max_storage_buffer_binding_size.min(128 << 20);
            let limits =
                sr_volume::CacheLimits { max_bytes: cap, max_bricks: (cap / 2060) as usize, ..Default::default() };
            let reader = std::io::BufReader::new(file);
            let decoded = match format {
                "srvol" => sr_volume::Volume::read(reader, limits),
                "openvdb" => sr_volume::openvdb::read(reader, limits),
                _ => Err(sr_volume::Error::Invalid("unsupported volume format")),
            };
            decoded
                .map(|v| Some(Arc::new(v)))
                .map_err(|e| sr_volume::Error::Io(std::io::Error::other(format!("{}: {e}", path.display()))))
        })();
        let bytes = loaded.as_ref().ok().and_then(|v| v.as_ref()).map_or(0, |v| v.bytes());
        if self.volume_cache_bytes.saturating_add(bytes) > 256 << 20 || self.volume_assets.len() >= 64 {
            self.volume_assets.clear();
            self.baked_frames.clear();
            self.volume_cache_bytes = 0;
        }
        self.volume_cache_bytes += bytes;
        self.volume_assets.insert(key, loaded.as_ref().cloned().map_err(ToString::to_string));
        loaded
    }

    fn baked_volume(
        &mut self,
        path: &std::path::Path,
        sha: &str,
        time: f64,
        interpolation: sr_volume::sequence::Interpolation,
        advect: bool,
    ) -> Result<sr_volume::sequence::TimedFramePair, sr_volume::Error> {
        let cap = self.gpu.device.limits().max_storage_buffer_binding_size.min(128 << 20);
        let limits = sr_volume::bake::BakeLimits {
            frame: sr_volume::CacheLimits { max_bytes: cap, max_bricks: (cap / 2060) as usize, ..Default::default() },
            ..Default::default()
        };
        let key = (path.to_owned(), sha.to_owned());
        let sequence = match self.baked_sequences.get(&key) {
            Some(sequence) => sequence.clone(),
            None => {
                let sequence = Arc::new(sr_volume::bake::BakedSequence::open(path, Some(sha), limits)?);
                // Each manifest is at most 4 MB; keep at most eight resident.
                if self.baked_sequences.len() >= 8 {
                    self.baked_sequences.clear();
                }
                self.baked_sequences.insert(key, sequence.clone());
                sequence
            }
        };
        let load = |frame: &sr_volume::bake::BakedFrame| {
            let key = (sequence.directory().to_owned(), frame.sha256());
            if let Some(volume) = self.baked_frames.get(&key) {
                return Ok(volume.clone());
            }
            let volume = frame.read(sequence.directory(), limits.frame)?;
            let bytes = volume.bytes();
            if bytes > 256 << 20 {
                return Err(sr_volume::Error::Limit("baked frame resident bytes"));
            }
            if self.volume_cache_bytes.saturating_add(bytes) > 256 << 20 || self.baked_frames.len() >= 64 {
                self.volume_assets.clear();
                self.baked_frames.clear();
                self.volume_cache_bytes = 0;
            }
            self.volume_cache_bytes += bytes;
            self.baked_frames.insert(key, volume.clone());
            Ok(volume)
        };
        if advect {
            sequence.load_timed_with(time, interpolation, 256 << 20, load)
        } else {
            sequence
                .load_with(time, interpolation, 256 << 20, load)
                .map(|frames| sr_volume::sequence::TimedFramePair { frames, elapsed: [0.; 2] })
        }
    }

    fn volume_draw(&mut self, ctx: &Ctx, j: usize, opacity: f32) -> Result<Option<crate::volume::VolumeDraw>, String> {
        use sr_volume::medium::{Bounds, March, Medium, Optical};
        use sr_volume::sequence::{FramePair, Interpolation, MissingFrame, Sequence, TimedFramePair};
        let n = &ctx.g.nodes[j];
        let a = attrs(n);
        let (timed, label, channel, temperature_channel, bounds, velocity_channels) = if let Some(native) =
            &n.sim_volume
        {
            (
                TimedFramePair {
                    frames: FramePair {
                        first: Some(native.data.clone()),
                        second: Some(native.data.clone()),
                        blend: 0.0,
                    },
                    elapsed: [0.; 2],
                },
                n.id.to_string(),
                "density".to_string(),
                Some("temperature".to_string()),
                None,
                None,
            )
        } else {
            let id = a.str("volume").ok_or("volume primitive requires @volume")?;
            let key = n.asset.as_deref().unwrap_or(&id);
            let Some((AssetsChild::Volume(asset), doc)) = self.asset(ctx.p, key) else {
                return Err(format!("volume asset {id} not found"));
            };
            let a = Attrs { e: asset, props: None };
            let base = ctx.p.base_dirs.get(doc).cloned().unwrap_or_default();
            let mut read = |src: &str| match sr_model::assets::resolve(src, &base) {
                sr_model::assets::Resolved::Local(path) => self.volume_cache(&path, asset.format.as_str()),
                sr_model::assets::Resolved::Remote(_) => {
                    Err(sr_volume::Error::Invalid("remote volume must be resolved before rendering"))
                }
            };
            let advect = asset.interpolation.as_str() == "advect";
            let frames = if asset.format.as_str() == "srvseq" {
                let sha = asset.sha256.as_ref().ok_or("srvseq requires a manifest SHA-256")?.to_string();
                let interpolation =
                    if asset.interpolation.as_str() != "hold" { Interpolation::Linear } else { Interpolation::Hold };
                match sr_model::assets::resolve(&asset.src, &base) {
                    sr_model::assets::Resolved::Local(path) => self
                        .baked_volume(&path, &sha, ctx.g.time, interpolation, advect)
                        .map_err(|e| format!("{}: {e}", path.display()))?,
                    sr_model::assets::Resolved::Remote(_) => {
                        return Err("remote volume bake must be resolved before rendering".into())
                    }
                }
            } else if let (Some(first), Some(last)) = (a.opt("first"), a.opt("last")) {
                let interpolation =
                    if asset.interpolation.as_str() != "hold" { Interpolation::Linear } else { Interpolation::Hold };
                let missing = match asset.missing_frame.as_str() {
                    "hold" => MissingFrame::Hold,
                    "transparent" => MissingFrame::Transparent,
                    _ => MissingFrame::Error,
                };
                let sequence = Sequence::new(
                    first as i64,
                    last as i64,
                    asset.fps.unwrap_or(ctx.p.fps).as_f64(),
                    interpolation,
                    missing,
                )
                .map_err(|e| e.to_string())?;
                let object = attrs(n);
                let time = n.local_time * object.num("animationSpeed", 1.0) + object.num("animationOffset", 0.0);
                let load = |frame| {
                    let src = sr_model::assets::sequence_frame(&asset.src, frame)
                        .ok_or(sr_volume::Error::Invalid("volume sequence src has no valid frame placeholder"))?;
                    read(&src)
                };
                (if advect {
                    sequence.load_timed(time, 256 << 20, load)
                } else {
                    sequence.load(time, 256 << 20, load).map(|frames| TimedFramePair { frames, elapsed: [0.; 2] })
                })
                .map_err(|e| format!("{}: {e}", asset.src))?
            } else {
                let cache = read(&asset.src)
                    .map_err(|e| e.to_string())?
                    .ok_or_else(|| format!("volume {} does not exist", asset.src))?;
                TimedFramePair {
                    frames: FramePair { first: Some(cache.clone()), second: Some(cache), blend: 0.0 },
                    elapsed: [0.; 2],
                }
            };
            let bounds = if a.opt("boundsMinX").is_some() {
                Some(
                    Bounds::new(
                        [a.num("boundsMinX", 0.0), a.num("boundsMinY", 0.0), a.num("boundsMinZ", 0.0)],
                        [a.num("boundsMaxX", 0.0), a.num("boundsMaxY", 0.0), a.num("boundsMaxZ", 0.0)],
                    )
                    .map_err(|e| e.to_string())?,
                )
            } else {
                None
            };
            (
                frames,
                asset.src.clone(),
                a.str("densityGrid").unwrap_or_else(|| "density".into()),
                a.str("temperatureGrid"),
                bounds,
                if advect {
                    Some([
                        a.str("velocityGridX").ok_or("advect requires velocityGridX")?,
                        a.str("velocityGridY").ok_or("advect requires velocityGridY")?,
                        a.str("velocityGridZ").ok_or("advect requires velocityGridZ")?,
                    ])
                } else {
                    None
                },
            )
        };
        let TimedFramePair { frames, elapsed } = timed;
        if frames.first.is_none() && frames.second.is_none() {
            return Ok(None);
        }
        let zero =
            Arc::new(sr_volume::SparseGrid::new(sr_volume::Transform::identity(), 0.0, 0).map_err(|e| e.to_string())?);
        let grid = |frame: &Option<Arc<sr_volume::Volume>>, channel: &str, kind: &str| match frame {
            None => Ok(zero.clone()),
            Some(frame) => {
                frame.shared_grid(channel).ok_or_else(|| format!("volume {} lacks {kind} channel {channel}", label))
            }
        };
        // Validate selected channels even at exact or clamped endpoints.
        let advection = if let Some(channels) = velocity_channels {
            let trace = |frame, dt| {
                let components = [
                    grid(frame, &channels[0], "velocity")?,
                    grid(frame, &channels[1], "velocity")?,
                    grid(frame, &channels[2], "velocity")?,
                ];
                sr_volume::advection::Advection::new(components, dt).map_err(|e| e.to_string())
            };
            Some([trace(&frames.first, elapsed[0])?, trace(&frames.second, elapsed[1])?])
        } else {
            None
        };
        let density = grid(&frames.first, &channel, "density")?;
        let next_density = grid(&frames.second, &channel, "density")?;
        let mut optical = Optical { albedo: [1.0; 3], ..Default::default() };
        let mut march = March { step_size: 1.0, max_steps: 2048 };
        // Declaring a channel is an asset contract even when this instance does
        // not emit thermally. Missing files alone may become transparent.
        let temperature_grids = if let Some(channel) = temperature_channel {
            let first = grid(&frames.first, &channel, "temperature")?;
            let second = grid(&frames.second, &channel, "temperature")?;
            for grid in [&first, &second] {
                if std::iter::once(grid.background())
                    .chain(grid.bricks().flat_map(|(_, v)| v.iter().copied()))
                    .any(|v| v < 0.0)
                {
                    return Err(format!("volume {} has negative kelvin samples in {channel}", label));
                }
            }
            Some((first, second))
        } else {
            None
        };
        let mut temperature = None;
        let mut light_grid = None;
        if let Some(child) = sr_model::element::children(&*n.elem).into_iter().find(|c| c.element_name() == "medium") {
            let key = format!("{}/medium[0]", n.id);
            let props = n.parts.iter().find(|p| *p.key == key).map(|p| &p.props);
            let a = Attrs { e: child, props };
            let albedo = self.working_color(a.paint("albedo"), self.literal_linear([1.0; 4]));
            let emission = self.working_color(a.paint("emissionColor"), self.literal_linear([0.0, 0.0, 0.0, 1.0]));
            optical = Optical {
                density_scale: a.num("densityScale", 1.0),
                extinction: a.num("extinction", 1.0),
                albedo: std::array::from_fn(|i| f64::from(albedo[i])),
                emission: std::array::from_fn(|i| f64::from(emission[i]) * a.num("emissionScale", 0.0)),
                anisotropy: a.num("anisotropy", 0.0),
            };
            march = March { step_size: a.num("stepSize", 1.0), max_steps: a.num("maxSteps", 2048.0) as u32 };
            if a.str("lighting").as_deref() == Some("grid") {
                light_grid = Some((
                    a.num("lightGridCell", 1.0) as u32,
                    a.num("lightGridDomeDirections", 64.0) as u32,
                    a.num("lightGridMemoryMiB", 128.0) as u64,
                ));
            }
            if flag(&a, "blackbody", false) {
                let (first, second) = temperature_grids.as_ref().ok_or("blackbody requires temperatureGrid")?;
                temperature =
                    Some((first.clone(), second.clone(), a.num("temperatureScale", 1.0), a.num("emissionScale", 0.0)));
            }
        }
        optical.density_scale *= f64::from(opacity);
        let world = Self::world3(ctx.g, doc_lights(ctx.p), j, 0);
        let transform = sr_volume::Transform::new(world.as_dmat4().to_cols_array()).map_err(|e| e.to_string())?;
        let mut medium = Medium::new(density, bounds, transform, optical).map_err(|e| e.to_string())?;
        let mut next_temperature = None;
        if let Some((grid, next, scale, emission_scale)) = temperature {
            medium = medium.with_temperature(grid, scale, emission_scale).map_err(|e| e.to_string())?;
            next_temperature = Some(next);
        }
        let same = matches!((&frames.first,&frames.second),(Some(a),Some(b)) if Arc::ptr_eq(a,b));
        // Content-addressed bakes may share storage for different sample times.
        // Only identical timing, not identical storage, freezes a moving pair.
        let moving = advection.as_ref().is_some_and(|traces| traces.iter().any(|t| t.elapsed() != 0.0));
        if frames.blend > 0.0 && (!same || moving) {
            medium = medium.with_next_frame(next_density, next_temperature, frames.blend).map_err(|e| e.to_string())?;
            if let Some([first, second]) = advection {
                medium = medium.with_advection(first, second).map_err(|e| e.to_string())?;
            }
        }
        let thermal_color = glam::DMat3::from_cols(
            Vec3::from_array(self.lin_srgb([1.0, 0.0, 0.0])).as_dvec3(),
            Vec3::from_array(self.lin_srgb([0.0, 1.0, 0.0])).as_dvec3(),
            Vec3::from_array(self.lin_srgb([0.0, 0.0, 1.0])).as_dvec3(),
        );
        let mut draw = crate::volume::VolumeDraw::new(Arc::new(medium), march)?.with_thermal_color(thermal_color)?;
        if let Some((cell, directions, memory)) = light_grid {
            draw = draw.with_light_grid(cell, directions, memory)?;
        }
        Ok(Some(draw.with_shadows(flag(&attrs(n), "castShadow", true), flag(&attrs(n), "receiveShadow", true))))
    }

    /// Draws of one 3D object.
    #[allow(clippy::too_many_arguments)]
    fn object_draws(
        &mut self,
        plan: &mut Plan,
        ctx: &Ctx,
        j: usize,
        opacity: f32,
        draws: &mut Vec<Draw3>,
        splats: &mut Vec<SplatDraw>,
    ) {
        let g = ctx.g;
        let n = &g.nodes[j];
        if let Some(fracture) = &n.fracture {
            self.fracture_draws3(plan, ctx, j, fracture, opacity, draws);
            return;
        }
        let world = Self::world3(g, doc_lights(ctx.p), j, 0);
        let draw_start = draws.len();
        let splat_start = splats.len();
        self.object_draws_at(plan, ctx, j, n, world, opacity, draws, splats);
        let deformation = (|| -> Result<(), String> {
            let Some(crater) = sr_eval::crater::at(n)? else { return Ok(()) };
            if splats.len() != splat_start {
                return Err("craters require triangle surfaces, not Gaussian splats".into());
            }
            if crater.progress == 0. {
                return Ok(());
            }
            let bytes = draws[draw_start..]
                .iter()
                .try_fold(0usize, |total, draw| {
                    draw.mesh
                        .cpu()
                        .0
                        .len()
                        .checked_mul(std::mem::size_of::<sr_3d::Vertex>())
                        .and_then(|b| total.checked_add(b))
                })
                .ok_or("crater vertex memory overflow")?;
            if bytes > crater.max_bytes {
                return Err("crater draw vertices exceed memory budget".into());
            }
            let inverse = world.as_dmat4().inverse();
            for draw in &mut draws[draw_start..] {
                let transform = inverse * draw.model.as_dmat4();
                let vertices =
                    crater.kernel.deform_in(draw.mesh.cpu().0, transform, crater.progress, crater.max_bytes)?;
                let mesh = match &draw.mesh {
                    MeshSrc::Cached(mesh) | MeshSrc::Deformed(_, mesh) => mesh.clone(),
                };
                draw.mesh = MeshSrc::Deformed(vertices, mesh);
            }
            Ok(())
        })();
        if let Err(error) = deformation {
            draws.truncate(draw_start);
            splats.truncate(splat_start);
            plan.stats.errors.push(format!("{}: {error}", n.id));
        }
        if n.kind == "ocean" {
            if let Some(sea) = &n.sim_ocean {
                let config =
                    sr_model::element::children(&*n.elem).into_iter().find(|e| e.element_name() == "whitewater");
                for (i, mesh) in sea.whitewater_mesh.iter().enumerate() {
                    if mesh.indices.is_empty() {
                        continue;
                    }
                    let cap = self.gpu.device.limits().max_buffer_size;
                    if mesh.vertices.len() as u64 * std::mem::size_of::<sr_3d::Vertex>() as u64 > cap
                        || mesh.indices.len() as u64 * 4 > cap
                    {
                        plan.stats.errors.push(format!("{}: whitewater exceeds device buffer limits", n.id));
                        continue;
                    }
                    let id = config
                        .and_then(|c| c.get_attr(if i == 0 { "foamMaterial" } else { "sprayMaterial" }))
                        .map(|v| v.to_string());
                    let material = match id {
                        Some(id) => match self.document_material(plan, ctx, &id) {
                            Some(m) => m,
                            None => {
                                plan.stats.errors.push(format!("{}: whitewater material {id} not found", n.id));
                                continue;
                            }
                        },
                        None => (
                            MaterialParams {
                                base_color: [1.; 4],
                                roughness: if i == 0 { 0.8 } else { 0.12 },
                                transmission: if i == 0 { 0. } else { 0.6 },
                                ior: 1.333,
                                double_sided: true,
                                ..Default::default()
                            },
                            Maps::default(),
                        ),
                    };
                    draws.push(Draw3 {
                        mesh: MeshSrc::Cached(self.three_engine().upload_mesh(&mesh.vertices, &mesh.indices)),
                        model: world,
                        material: material.0,
                        maps: material.1,
                        opacity,
                        cast_shadow: flag(&attrs(n), "castShadow", true),
                        receive_shadow: flag(&attrs(n), "receiveShadow", true),
                        shadow_catcher: false,
                    });
                }
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn object_draws_at(
        &mut self,
        plan: &mut Plan,
        ctx: &Ctx,
        j: usize,
        n: &FrameNode,
        world: Mat4,
        opacity: f32,
        draws: &mut Vec<Draw3>,
        splats: &mut Vec<SplatDraw>,
    ) {
        let a = attrs(n);
        let (cast, receive) = (flag(&a, "castShadow", true), flag(&a, "receiveShadow", true));
        // a shadow catcher draws only the darkening of the casters and casts no shadow itself
        let catcher = flag(&a, "shadowCatcher", false);
        let cast = cast && !catcher;
        let doc_mat = match a.str("material") {
            Some(id) => {
                let m = self.document_material(plan, ctx, &id);
                if m.is_none() {
                    plan.stats.errors.push(format!("{}: material {id} not found", n.id));
                }
                m
            }
            None => None,
        };
        let default_mat = || {
            let material = if n.kind == "ocean" {
                MaterialParams {
                    base_color: [1.0; 4],
                    roughness: 0.05,
                    transmission: 1.0,
                    ior: 1.333,
                    double_sided: true,
                    ..Default::default()
                }
            } else {
                MaterialParams { base_color: [0.8, 0.8, 0.8, 1.0], ..Default::default() }
            };
            (material, Maps::default())
        };
        if matches!(primitive_kind(n).as_str(), "map" | "globe") {
            self.map3d_draws(plan, ctx, j, world, opacity, doc_mat, cast, receive, draws);
            return;
        }
        if primitive_kind(n) != "mesh" {
            if let Some(mesh) = self.primitive_mesh(plan, ctx, n) {
                let (material, maps) = doc_mat.unwrap_or_else(default_mat);
                draws.push(Draw3 {
                    mesh: MeshSrc::Cached(mesh),
                    model: world,
                    material,
                    maps,
                    opacity,
                    cast_shadow: cast,
                    receive_shadow: receive,
                    shadow_catcher: catcher,
                });
            }
            return;
        }
        let Some((key, asset)) = self.mesh_asset(plan, ctx, n) else { return };
        let opacity = opacity * asset.opacity();
        if let Some(s) = asset.splats() {
            let gpu = match self.three_engine().splat_cache.get(&key) {
                Some(g) => g.clone(),
                None => {
                    let g = self.three_engine().upload_splats(s);
                    self.three_engine().splat_cache.insert(key.clone(), g.clone());
                    g
                }
            };
            if let Some(m) = crate::three::splat_note(s.len() as u64, gpu.n as u64) {
                let m = format!("{}: {m}", n.id);
                if !plan.stats.unsupported.contains(&m) {
                    plan.stats.unsupported.push(m);
                }
            }
            splats.push(SplatDraw { gpu, model: world * s.basis, opacity });
        }
        if let Some(model) = asset.model() {
            // animation clip at the object's local time
            let clip = a.str("animationClip").and_then(|c| {
                model
                    .animations
                    .iter()
                    .find(|x| x.name == c)
                    .or_else(|| c.parse::<usize>().ok().and_then(|k| model.animations.get(k)))
            });
            if let (Some(c), None) = (a.str("animationClip"), clip) {
                plan.stats.errors.push(format!("{}: animation clip {c} not found", n.id));
            }
            let t = (n.local_time * a.num("animationSpeed", 1.0) + a.num("animationOffset", 0.0)) as f32;
            let t = clip.map(|c| if c.duration > 0.0 { t.rem_euclid(c.duration) } else { 0.0 }).unwrap_or(0.0);
            let (locals, weights) = sr_3d::anim::pose(model, clip, t);
            let morph: Option<Vec<f32>> = a.nums("morphWeights").map(|v| v.iter().map(|x| *x as f32).collect());
            let variant = a.str("materialVariant");
            // `node`: only that node and what is under it, placed at the object's origin
            let selected = match a.str("node") {
                Some(name) => match model.nodes.iter().position(|x| x.name == name) {
                    Some(k) => Some((k, model.world_matrices(&locals)[k].inverse())),
                    None => {
                        plan.stats.errors.push(format!("{}: node {name} not found in the model", n.id));
                        return;
                    }
                },
                None => None,
            };
            let under = |mut k: usize, root: usize| loop {
                if k == root {
                    break true;
                }
                match model.nodes[k].parent {
                    Some(p) => k = p,
                    None => break false,
                }
            };
            // `materialOverride`: imported material name -> document material id
            let overrides: Vec<(String, String)> = a
                .str("materialOverride")
                .map(|s| {
                    s.split_whitespace()
                        .filter_map(|pair| pair.split_once(':'))
                        .map(|(old, new)| (old.to_string(), new.to_string()))
                        .collect()
                })
                .unwrap_or_default();
            // an imported name that matches no material of the model has no effect: say so, with the names there are
            for (old, _) in &overrides {
                if !model.materials.iter().any(|m| &m.name == old) {
                    let names: Vec<&str> = model.materials.iter().map(|m| m.name.as_str()).collect();
                    let msg = format!(
                        "{}: materialOverride names {old}, which is no material of the model (it has: {})",
                        n.id,
                        if names.is_empty() { "none".to_string() } else { names.join(", ") }
                    );
                    if !plan.stats.unsupported.contains(&msg) {
                        plan.stats.unsupported.push(msg);
                    }
                }
            }
            for item in sr_3d::anim::draw_list(model, &locals, &weights, morph.as_deref()) {
                if selected.as_ref().is_some_and(|(root, _)| !under(item.node, *root)) {
                    continue;
                }
                let prim = &model.primitives[item.prim];
                let mi = variant
                    .as_ref()
                    .and_then(|v| prim.variants.iter().find(|(name, _)| name == v).map(|(_, m)| *m))
                    .or(prim.material);
                // @material replaces every material; a pair of @materialOverride replaces the ones it names
                let item_mat = match &doc_mat {
                    Some(m) => Some(m.clone()),
                    None => {
                        let name = mi.and_then(|k| model.materials.get(k)).map(|m| m.name.as_str());
                        match overrides.iter().find(|(old, _)| Some(old.as_str()) == name) {
                            Some((_, id)) => {
                                let m = self.document_material(plan, ctx, id);
                                if m.is_none() {
                                    let e = format!("{}: material {id} not found", n.id);
                                    if !plan.stats.errors.contains(&e) {
                                        plan.stats.errors.push(e);
                                    }
                                }
                                m
                            }
                            None => None,
                        }
                    }
                };
                let imported = if item_mat.is_none() { mi.and_then(|k| model.materials.get(k)) } else { None };
                let mkey = format!("{key}#{}#material{:?}", item.prim, imported.map(|_| mi));
                let mesh = match self.three_engine().meshes.get(&mkey) {
                    Some(m) => m.clone(),
                    None => {
                        let mut vertices = prim.vertices.clone();
                        if let Some(material) = imported {
                            material.apply_texture_coordinates(prim, &mut vertices);
                        }
                        let m = self.three_engine().upload_mesh(&vertices, &prim.indices);
                        self.three_engine().meshes.insert(mkey, m.clone());
                        m
                    }
                };
                let (material, maps) = match &item_mat {
                    Some((p, m)) => (p.clone(), m.clone()),
                    None => {
                        let mi = variant
                            .as_ref()
                            .and_then(|v| prim.variants.iter().find(|(name, _)| name == v).map(|(_, m)| *m))
                            .or(prim.material);
                        match mi.and_then(|k| model.materials.get(k).map(|m| (k, m))) {
                            Some((k, im)) => {
                                let mut p = im.params.clone();
                                // imported colours are linear sRGB
                                for c in [
                                    &mut p.base_color[..3],
                                    &mut p.emissive[..],
                                    &mut p.sheen_color[..],
                                    &mut p.attenuation_color[..],
                                    &mut p.specular_color[..],
                                ] {
                                    let l = self.lin_srgb([c[0], c[1], c[2]]);
                                    c.copy_from_slice(&l);
                                }
                                let mut maps = Maps::default();
                                for (slot, t) in [
                                    (0, im.maps.base_color),
                                    (1, im.maps.normal),
                                    (2, im.maps.metallic_roughness),
                                    (3, im.maps.occlusion),
                                    (4, im.maps.emissive),
                                ] {
                                    if let Some(ti) = t {
                                        let tkey = format!("{key}#tex{ti}");
                                        maps[slot] = Some(match self.three_engine().textures.get(&tkey) {
                                            Some(t) => t.clone(),
                                            None => {
                                                let tx = &model.textures[ti];
                                                let up = self.three_engine().upload_texture(tx);
                                                self.three_engine().textures.insert(tkey, up.clone());
                                                up
                                            }
                                        });
                                    }
                                }
                                let _ = k;
                                (p, maps)
                            }
                            None => default_mat(),
                        }
                    }
                };
                let src = match item.vertices {
                    Some(mut vs) => {
                        if let Some(material) = imported {
                            material.apply_texture_coordinates(prim, &mut vs);
                        }
                        MeshSrc::Deformed(vs, mesh)
                    }
                    None => MeshSrc::Cached(mesh),
                };
                draws.push(Draw3 {
                    mesh: src,
                    model: world
                        * model.basis
                        * selected.as_ref().map_or(Mat4::IDENTITY, |(_, inv)| *inv)
                        * item.matrix,
                    material,
                    maps,
                    opacity,
                    cast_shadow: cast,
                    receive_shadow: receive,
                    shadow_catcher: catcher,
                });
            }
        }
    }

    fn fracture_draws3(
        &mut self,
        plan: &mut Plan,
        ctx: &Ctx,
        j: usize,
        fracture: &sr_eval::fracture::SimFracture,
        opacity: f32,
        draws: &mut Vec<Draw3>,
    ) {
        let n = &ctx.g.nodes[j];
        let a = attrs(n);
        let mut exterior = a.str("material").and_then(|id| self.document_material(plan, ctx, &id));
        let Some(interior) = self.document_material(plan, ctx, &fracture.geometry.interior_material) else {
            plan.stats.errors.push(format!("{}: fracture interior material missing", n.id));
            return;
        };
        let (cast, receive) = (flag(&a, "castShadow", true), flag(&a, "receiveShadow", true));
        if primitive_kind(n) == "globe" {
            let mut templates = Vec::new();
            self.map3d_draws(plan, ctx, j, Mat4::IDENTITY, opacity, exterior, cast, receive, &mut templates);
            let Some(template) = templates.into_iter().next() else { return };
            exterior = Some((template.material, template.maps));
        }
        for (index, piece) in fracture.geometry.pieces.iter().enumerate() {
            if !fracture.enabled[index] {
                continue;
            }
            let pose = fracture.poses[index];
            let model = Mat4::from_rotation_translation(
                glam::Quat::from_array(pose.rot.map(|v| v as f32)),
                glam::Vec3::from_array(pose.pos.map(|v| v as f32)),
            );
            if !model.is_finite() {
                plan.stats.errors.push(format!("{}: fracture pose exceeds render precision", n.id));
                return;
            }
            for (batch, surface) in piece.surfaces.iter().enumerate() {
                let key = format!("fracture:{}:{index}:{batch}", fracture.geometry.key);
                let mesh = if let Some(mesh) = self.three_engine().meshes.get(&key) {
                    mesh.clone()
                } else {
                    let mesh = self.three_engine().upload_mesh(&surface.mesh.vertices, &surface.mesh.indices);
                    self.three_engine().meshes.insert(key, mesh.clone());
                    mesh
                };
                let (material, maps) = if surface.material == sr_3d::fracture::SurfaceMaterial::Interior {
                    interior.clone()
                } else if let Some(material) = &exterior {
                    material.clone()
                } else if let Some((asset, im)) = fracture
                    .geometry
                    .model
                    .as_ref()
                    .and_then(|asset| Some(asset).zip(surface.mesh.material.and_then(|k| asset.materials.get(k))))
                {
                    let mut material = im.params.clone();
                    for c in [
                        &mut material.base_color[..3],
                        &mut material.emissive[..],
                        &mut material.sheen_color[..],
                        &mut material.attenuation_color[..],
                        &mut material.specular_color[..],
                    ] {
                        c.copy_from_slice(&self.lin_srgb([c[0], c[1], c[2]]));
                    }
                    let mut maps = Maps::default();
                    for (slot, texture) in [
                        (0, im.maps.base_color),
                        (1, im.maps.normal),
                        (2, im.maps.metallic_roughness),
                        (3, im.maps.occlusion),
                        (4, im.maps.emissive),
                    ] {
                        if let Some(texture) = texture {
                            let key = format!("fracture:{}:texture{texture}", fracture.geometry.key);
                            let tex = if let Some(tex) = self.three_engine().textures.get(&key) {
                                tex.clone()
                            } else {
                                let tex = self.three_engine().upload_texture(&asset.textures[texture]);
                                self.three_engine().textures.insert(key, tex.clone());
                                tex
                            };
                            maps[slot] = Some(tex);
                        }
                    }
                    (material, maps)
                } else {
                    (MaterialParams { base_color: [0.8, 0.8, 0.8, 1.], ..Default::default() }, Maps::default())
                };
                draws.push(Draw3 {
                    mesh: MeshSrc::Cached(mesh),
                    model,
                    material,
                    maps,
                    opacity,
                    cast_shadow: cast,
                    receive_shadow: receive,
                    shadow_catcher: false,
                });
            }
        }
    }

    /// Shared prototype meshes, placed from each particle's world-space state.
    fn particle_draws3(
        &mut self,
        plan: &mut Plan,
        ctx: &Ctx,
        j: usize,
        cam: &CameraView,
        opacity: f32,
        draws: &mut Vec<Draw3>,
    ) {
        let n = &ctx.g.nodes[j];
        let Some(particles) = &n.particles3d else {
            return;
        };
        if particles.frame.particles.is_empty() {
            return;
        }
        let a = attrs(n);
        let shape = a.str("shape").unwrap_or_else(|| "sphere".into());
        let mut prototype = n.clone();
        prototype
            .props
            .0
            .retain(|(k, _)| !matches!(&**k, "primitive" | "radius" | "width" | "height" | "depth" | "segments"));
        for (key, value) in [
            ("radius", 0.5),
            ("width", 1.),
            ("height", 1.),
            ("depth", 1.),
            ("segments", if shape == "billboard" { 1. } else { a.num("segments", 12.).max(3.) }),
        ] {
            prototype.props.0.push((key.into(), Value::Num(value)));
        }
        let mut templates = Vec::new();
        let mut splats = Vec::new();
        self.object_draws_at(plan, ctx, j, &prototype, Mat4::IDENTITY, 1., &mut templates, &mut splats);
        if !splats.is_empty() {
            plan.stats
                .errors
                .push(format!("{}: particle prototypes require triangle meshes, not Gaussian splats", n.id));
            return;
        }
        let count = templates.len().checked_mul(particles.frame.particles.len());
        let budget = (a.num("maxMemoryMiB", 256.) as usize).saturating_mul(1 << 20);
        if count.and_then(|n| n.checked_mul(std::mem::size_of::<Draw3>())).is_none_or(|b| b > budget) {
            plan.stats.errors.push(format!("{}: particle render instances exceed memory budget", n.id));
            return;
        }
        // Rest-pose skinning/morph geometry is uploaded once, never copied for
        // every particle. Ordinary meshes already share their immutable buffers.
        for t in &mut templates {
            if let MeshSrc::Deformed(_, _) = &t.mesh {
                let (v, i) = t.mesh.cpu();
                t.mesh = MeshSrc::Cached(self.three_engine().upload_mesh(v, i));
            }
        }
        if let Some(sprite) = a.str("sprite") {
            let ns = ctx
                .p
                .nodes
                .iter()
                .find(|p| p.id == n.id)
                .and_then(|p| p.doc.checked_sub(1))
                .and_then(|d| ctx.p.includes.get(d as usize))
                .map(|d| &*d.0)
                .unwrap_or("");
            let key = sr_eval::sim::asset_key(&ctx.p.assets, ns, &sprite);
            let Some((AssetsChild::Image(img), doc)) = self.asset(ctx.p, &key) else {
                plan.stats.errors.push(format!("{}: sprite image {sprite} not found", n.id));
                return;
            };
            let img = img.clone();
            let base = ctx.p.base_dirs.get(doc).cloned().unwrap_or_default();
            let path = match sr_model::assets::resolve(&img.src, &base) {
                sr_model::assets::Resolved::Local(path) => path,
                sr_model::assets::Resolved::Remote(uri) => {
                    plan.stats
                        .errors
                        .push(format!("{}: remote particle sprite {uri} is not fetched while rendering", n.id));
                    return;
                }
            };
            let texture_key = format!(
                "particle-sprite|{}|{}|{}|{}|{}",
                path.display(),
                img.color_space,
                img.transfer,
                img.alpha,
                img.color_profile
            );
            let texture = if let Some(texture) = self.three_engine().textures.get(&texture_key) {
                texture.clone()
            } else {
                let decoded = match resources::decode_image(
                    &path,
                    img.color_space,
                    img.transfer,
                    img.alpha,
                    img.color_profile == m::ColorProfile::Embedded,
                    &self.working,
                    self.max_texture,
                ) {
                    Ok(decoded) => decoded,
                    Err(error) => {
                        plan.stats.errors.push(format!("{}: cannot decode particle sprite: {error}", n.id));
                        return;
                    }
                };
                if let Some(note) = decoded.note {
                    plan.stats.unsupported.push(format!("{}: {note}", n.id));
                }
                let (width, height, pixels) = &decoded.levels[0];
                // Surface maps are straight RGBA8. Decode through the image
                // asset's color/alpha policy, then encode linear working RGB
                // with sRGB transfer for the surface texture sampler.
                let rgba: Vec<u8> = pixels
                    .iter()
                    .flat_map(|p| {
                        let alpha = p[3] as f64;
                        let rgb = if alpha > 0. {
                            self.working.to_linear([p[0] as f64 / alpha, p[1] as f64 / alpha, p[2] as f64 / alpha])
                        } else {
                            [0.; 3]
                        };
                        let rgb = rgb.map(|v| (color::encode(m::Transfer::Srgb, v.clamp(0., 1.)) * 255.).round() as u8);
                        [rgb[0], rgb[1], rgb[2], (alpha.clamp(0., 1.) * 255.).round() as u8]
                    })
                    .collect();
                let texture = self.three_engine().upload_rgba8(*width, *height, &rgba, true);
                self.three_engine().textures.insert(texture_key, texture.clone());
                texture
            };
            for t in &mut templates {
                t.maps[0] = Some(texture.clone());
                t.material.alpha_mode = AlphaMode::Blend;
            }
        }
        let color = self.working_color(a.paint("color"), self.literal_linear([1.; 4]));
        let end_color = if a.paint("colorEnd").is_some() {
            self.working_color(a.paint("colorEnd"), self.literal_linear([1.; 4]))
        } else {
            color
        };
        let view = cam.view.inverse();
        let right = view.x_axis.truncate();
        let down = view.y_axis.truncate();
        let forward = view.z_axis.truncate();
        for particle in &particles.frame.particles {
            let u = (particle.age(particles.frame.time) / particle.lifetime).clamp(0., 1.);
            let size0 = a.num("size", 1.);
            let size1 = a.num("sizeEnd", size0);
            let size = (size0 + (size1 - size0) * particles.size_curve.apply(u)).max(0.) as f32;
            let color_u = particles.color_curve.apply(u).clamp(0., 1.) as f32;
            let color: [f32; 4] = std::array::from_fn(|k| color[k] + (end_color[k] - color[k]) * color_u);
            let alpha = opacity * (1. + (a.num("opacityEnd", 1.) as f32 - 1.) * color_u);
            if size == 0. || alpha <= 0. || color[3] <= 0. {
                continue;
            }
            let matrix = match particle.transform(particles.frame.time) {
                Ok(m) => Mat4::from_cols_array(&m.columns().map(|v| v as f32)),
                Err(e) => {
                    plan.stats.errors.push(format!("{}: {e}", n.id));
                    return;
                }
            };
            let position = Vec3::from_array(particle.position.map(|v| v as f32));
            let lengths = Vec3::new(
                matrix.x_axis.truncate().length(),
                matrix.y_axis.truncate().length(),
                matrix.z_axis.truncate().length(),
            ) * size;
            let model = if shape == "billboard" {
                let x = matrix.x_axis.truncate();
                let angle = x.dot(down).atan2(x.dot(right));
                let (sin, cos) = angle.sin_cos();
                Mat4::from_cols(
                    ((right * cos + down * sin) * lengths.x).extend(0.),
                    ((down * cos - right * sin) * lengths.y).extend(0.),
                    forward.extend(0.),
                    position.extend(1.),
                )
            } else if shape == "streak" {
                let velocity = Vec3::from_array(particle.velocity.map(|v| v as f32));
                let direction = velocity.try_normalize().unwrap_or(Vec3::Y);
                let length =
                    if velocity.length_squared() > 0. { (a.num("trail", 1.) as f32).max(lengths.y) } else { lengths.y };
                Mat4::from_scale_rotation_translation(
                    Vec3::new(lengths.x, length, lengths.z),
                    glam::Quat::from_rotation_arc(Vec3::Y, direction),
                    position - direction * ((length - lengths.y) * 0.5),
                )
            } else {
                matrix * Mat4::from_scale(Vec3::splat(size))
            };
            if !model.is_finite() || !model.determinant().is_finite() || model.determinant() == 0. {
                plan.stats.errors.push(format!("{}: particle instance exceeds render precision", n.id));
                return;
            }
            for t in &templates {
                let MeshSrc::Cached(mesh) = &t.mesh else { unreachable!("shared particle prototype") };
                let mut material = t.material.clone();
                for (a, b) in material.base_color.iter_mut().zip(color) {
                    *a *= b;
                }
                if shape == "billboard" {
                    material.double_sided = true;
                }
                if material.base_color[3] < 1. || alpha < 1. {
                    material.alpha_mode = AlphaMode::Blend;
                }
                draws.push(Draw3 {
                    mesh: MeshSrc::Cached(mesh.clone()),
                    model: model * t.model,
                    material,
                    maps: t.maps.clone(),
                    opacity: alpha,
                    cast_shadow: t.cast_shadow,
                    receive_shadow: t.receive_shadow,
                    shadow_catcher: false,
                });
            }
        }
    }

    /// Lights of the document at this frame (dome lights become the environment).
    fn lights3(&mut self, plan: &mut Plan, ctx: &Ctx) -> (Vec<Light3>, Option<Env3>) {
        let mut out = Vec::new();
        let mut env = None;
        let Some(ls) = ctx.p.scene.lights.as_ref() else {
            return (out, env);
        };
        let base = Self::base_dir(ctx.p);
        for l in &ls.lights {
            let a = Attrs { e: l as &dyn Element, props: element_props(ctx.g, &l.id) };
            let kind = a.str("type").unwrap_or_default();
            let mut color = self.working_color(a.paint("color"), self.literal_linear([1.0; 4]));
            if let Some(k) = a.opt("colorTemperature") {
                let t = sr_3d::light::kelvin_to_rgb(k);
                let w = self.lin_srgb(t);
                for c in 0..3 {
                    color[c] *= w[c] as f32;
                }
            }
            let scale = a.num("intensity", 1.0) as f32 * 2f32.powf(a.num("exposure", 0.0) as f32);
            // pose in the parent's frame when parented
            let world = Self::pose_world(ctx.g, &ls.lights, l, &a, 0);
            let dir = world.transform_vector3(Vec3::Z).normalize();
            let right = world.transform_vector3(Vec3::X).normalize();
            if kind == "dome" {
                let Some(uri) = a.str("environment") else {
                    // a dome without an image lights uniformly, like an ambient light
                    out.push(self.light(
                        LightKind::Ambient,
                        &a,
                        Vec3::from_slice(&color[..3]) * scale,
                        dir,
                        right,
                        None,
                    ));
                    continue;
                };
                let path = match sr_model::assets::resolve(&uri, &base) {
                    sr_model::assets::Resolved::Local(p) => p,
                    sr_model::assets::Resolved::Remote(_) => base.join(&uri),
                };
                let key = path.display().to_string();
                let gpu = match self.three_engine().envs.get(&key) {
                    Some(e) => Some(e.clone()),
                    None => match sr_3d::env::load(&path) {
                        Ok(eq) => {
                            let e = self.three_engine().upload_env(&eq);
                            self.three_engine().envs.insert(key, e.clone());
                            Some(e)
                        }
                        Err(e) => {
                            plan.stats.errors.push(format!("{}: {e}", l.id));
                            None
                        }
                    },
                };
                if let Some(e) = gpu {
                    env = Some(Env3 {
                        env: e,
                        intensity: scale * color[1].max(color[0]).max(color[2]),
                        rotation: Mat4::from_quat(world.to_scale_rotation_translation().1),
                        visible: flag(&a, "environmentVisible", false),
                    });
                }
                continue;
            }
            let k = match kind.as_str() {
                "ambient" => LightKind::Ambient,
                "directional" => LightKind::Directional,
                "point" => LightKind::Point,
                "spot" => LightKind::Spot,
                "rect-area" => LightKind::Rect,
                "disk-area" => LightKind::Disk,
                _ => LightKind::Sphere,
            };
            let ies = match a.str("ies") {
                Some(uri) => {
                    let path = match sr_model::assets::resolve(&uri, &base) {
                        sr_model::assets::Resolved::Local(p) => p,
                        sr_model::assets::Resolved::Remote(_) => base.join(&uri),
                    };
                    let key = path.display().to_string();
                    let parsed = self
                        .ies
                        .entry(key)
                        .or_insert_with(|| {
                            std::fs::read_to_string(&path)
                                .map_err(|e| e.to_string())
                                .and_then(|t| sr_3d::light::Ies::parse(&t))
                                .map(|i| ThreeEngine::bake_ies(&i))
                        })
                        .clone();
                    match parsed {
                        Ok(t) => Some(t),
                        Err(e) => {
                            plan.stats.errors.push(format!("{}: IES {}: {e}", l.id, path.display()));
                            None
                        }
                    }
                }
                None => None,
            };
            let mut lt = self.light(k, &a, Vec3::from_slice(&color[..3]) * scale, dir, right, ies);
            lt.pos = world.transform_point3(Vec3::ZERO);
            Self::constrain_light(plan, ctx.g, &ls.lights, l as &dyn Element, &l.id, &mut lt);
            out.push(lt);
        }
        (out, env)
    }

    /// Applies a light's transform constraints: look-at aims it, copy-position moves it to the
    /// target (plus offset), follow-path rides a path, distance clamps its range. `parent` is part
    /// of the light's pose (`pose_world`).
    fn constrain_light(
        plan: &mut Plan,
        g: &FrameGraph,
        lights: &[m::Light],
        e: &dyn Element,
        id: &str,
        lt: &mut Light3,
    ) {
        for c in sr_model::element::children(e) {
            if c.element_name() != "transformConstraint" {
                continue;
            }
            let ca = Attrs { e: c, props: None };
            let kind = ca.str("type").unwrap_or_default();
            let w = ca.num("influence", 1.0).clamp(0.0, 1.0) as f32;
            let target = ca.str("target").and_then(|t| Self::target_point(g, lights, &t));
            let off = Vec3::new(ca.num("offsetX", 0.0) as f32, ca.num("offsetY", 0.0) as f32, 0.0);
            match (kind.as_str(), target) {
                ("look-at", Some(t)) => {
                    let want = (t - lt.pos).normalize_or(lt.dir);
                    let dir = lt.dir.lerp(want, w).normalize_or(want);
                    let up = if dir.y.abs() > 0.99 { Vec3::Z } else { Vec3::Y };
                    lt.right = up.cross(dir).normalize_or(Vec3::X) * -1.0;
                    lt.dir = dir;
                }
                ("parent", Some(_)) => {}
                ("copy-position" | "copy-transform", Some(t)) => lt.pos = lt.pos.lerp(t + off, w),
                ("distance", Some(t)) => {
                    let d = lt.pos - t;
                    let len = d.length();
                    let lo = ca.num("minDistance", 0.0) as f32;
                    let hi = ca.opt("maxDistance").map(|v| v as f32).unwrap_or(f32::MAX);
                    if len > 1e-6 {
                        lt.pos = lt.pos.lerp(t + d / len * len.clamp(lo, hi), w);
                    }
                }
                ("follow-path", _) => match ca.str("path").map(|d| sr_eval::path::MotionPath::parse(&d)) {
                    Some(Ok(mp)) => {
                        let (q, _) = mp.sample(ca.num("progress", 0.0).clamp(0.0, 1.0), true);
                        let at = Vec3::new(q[0] as f32, q[1] as f32, lt.pos.z) + off;
                        lt.pos = lt.pos.lerp(at, w);
                    }
                    _ => plan.stats.errors.push(format!("{id}: follow-path constraint needs a valid @path")),
                },
                ("look-at" | "parent" | "copy-position" | "copy-transform" | "distance", None) => {
                    plan.stats.errors.push(format!("{id}: {kind} constraint target not found"))
                }
                (other, _) => {
                    let m = format!("{id}: {other} constraints do not apply to lights");
                    if !plan.stats.unsupported.contains(&m) {
                        plan.stats.unsupported.push(m);
                    }
                }
            }
        }
    }

    fn light(
        &self,
        kind: LightKind,
        a: &Attrs,
        color: Vec3,
        dir: Vec3,
        right: Vec3,
        ies: Option<Arc<Vec<f32>>>,
    ) -> Light3 {
        let outer = (a.num("spotAngle", 45.0) as f32 * 0.5).to_radians();
        let inner =
            a.opt("innerConeAngle").map(|v| (v as f32 * 0.5).to_radians()).unwrap_or(outer * 0.8).min(outer - 1e-3);
        let w = a.opt("width").map(|v| v as f32).unwrap_or(50.0);
        let hgt = a.opt("height").map(|v| v as f32).unwrap_or(w);
        let r = a.opt("radius").map(|v| v as f32).unwrap_or(if kind == LightKind::Disk { w * 0.5 } else { 10.0 });
        Light3 {
            contact: if flag(a, "contactShadows", false) { a.num("contactShadowLength", 20.0) as f32 } else { 0.0 },
            kind,
            pos: Vec3::new(a.num("x", 0.0) as f32, a.num("y", 0.0) as f32, a.num("z", 0.0) as f32),
            dir,
            right,
            color,
            range: a.opt("range").map(|v| v as f32).unwrap_or(0.0),
            falloff: a.num("falloff", 2.0) as f32,
            cos_outer: outer.cos(),
            cos_inner: inner.max(0.0).cos(),
            cast_shadow: flag(a, "castShadow", false),
            softness: a.num("shadowSoftness", 0.0) as f32,
            bias: a.num("shadowBias", 0.0005) as f32,
            map_size: a.num("shadowMapSize", 2048.0) as u32,
            size: [w, hgt, r],
            ies,
            affects_diffuse: flag(a, "affectsDiffuse", true),
            affects_specular: flag(a, "affectsSpecular", true),
        }
    }

    fn sphere_pipe(&mut self) -> &SpherePipe {
        let d = self.gpu.device.clone();
        self.sphere.get_or_insert_with(|| {
            let module = d.create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some("sphere"),
                source: wgpu::ShaderSource::Wgsl(crate::three::sphere_src().into()),
            });
            let fs = wgpu::ShaderStages::FRAGMENT;
            let bgl = d.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("sphere"),
                entries: &[
                    wgpu::BindGroupLayoutEntry {
                        binding: 0,
                        visibility: fs,
                        ty: wgpu::BindingType::Buffer {
                            ty: wgpu::BufferBindingType::Uniform,
                            has_dynamic_offset: false,
                            min_binding_size: None,
                        },
                        count: None,
                    },
                    wgpu::BindGroupLayoutEntry {
                        binding: 1,
                        visibility: fs,
                        ty: wgpu::BindingType::Texture {
                            sample_type: wgpu::TextureSampleType::Float { filterable: true },
                            view_dimension: wgpu::TextureViewDimension::D2Array,
                            multisampled: false,
                        },
                        count: None,
                    },
                    wgpu::BindGroupLayoutEntry {
                        binding: 2,
                        visibility: fs,
                        ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                        count: None,
                    },
                ],
            });
            let layout = d.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("sphere"),
                bind_group_layouts: &[Some(&bgl)],
                immediate_size: 0,
            });
            let pipe = {
                let _creation = crate::gpu::creation_lock();
                d.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                    label: Some("sphere"),
                    layout: Some(&layout),
                    vertex: wgpu::VertexState {
                        module: &module,
                        entry_point: Some("vs_main"),
                        compilation_options: Default::default(),
                        buffers: &[],
                    },
                    primitive: Default::default(),
                    depth_stencil: None,
                    multisample: Default::default(),
                    fragment: Some(wgpu::FragmentState {
                        module: &module,
                        entry_point: Some("fs_main"),
                        compilation_options: Default::default(),
                        targets: &[Some(resources::FORMAT.into())],
                    }),
                    multiview_mask: None,
                    cache: None,
                })
            };
            let smp = d.create_sampler(&wgpu::SamplerDescriptor {
                mag_filter: wgpu::FilterMode::Linear,
                min_filter: wgpu::FilterMode::Linear,
                ..Default::default()
            });
            SpherePipe { pipe, bgl, smp }
        })
    }

    /// Renders a `scene360` frame: six cube faces per eye from the viewport camera,
    /// reprojected into the layout. 2D content outside isolated groups is placed
    /// in space like 2.5D layers at its own depth.
    pub(super) fn render_360(
        &mut self,
        g: &FrameGraph,
        p: &Program,
        mut provider: Option<&mut dyn FnMut(f64) -> FrameGraph>,
    ) -> Frame {
        let s = p.scene.scene360.as_ref().expect("scene360");
        let (w, hgt) = (s.width.clamp(16, 16384) as f32, s.height.clamp(16, 16384) as f32);
        let ipd = s.interpupillary.get() as f32 * 100.0;
        let eyes: Vec<(f32, [f32; 4])> = match s.stereo {
            m::Stereo::Mono => vec![(0.0, [0.0, 0.0, w, hgt])],
            m::Stereo::TopBottom => {
                vec![(-ipd * 0.5, [0.0, 0.0, w, hgt * 0.5]), (ipd * 0.5, [0.0, hgt * 0.5, w, hgt * 0.5])]
            }
            m::Stereo::LeftRight => {
                vec![(-ipd * 0.5, [0.0, 0.0, w * 0.5, hgt]), (ipd * 0.5, [w * 0.5, 0.0, w * 0.5, hgt])]
            }
        };
        let (rw, rh) = (eyes[0].1[2], eyes[0].1[3]);
        let mode = match s.layout {
            m::Scene360Layout::Equirectangular => 0.0,
            m::Scene360Layout::Cubemap => 1.0,
            m::Scene360Layout::Eac => 2.0,
            m::Scene360Layout::Fisheye180 => 3.0,
        };
        let side = match mode as u32 {
            0 => rw / 4.0,
            1 | 2 => (rw / 3.0).max(rh / 2.0),
            _ => rw.min(rh) / 2.0,
        }
        .ceil()
        .clamp(16.0, 4096.0) as u32;
        // the face graph: square frames, the viewport camera, 2D content placed in space
        let mut gf = g.clone();
        gf.size = [side as f64, side as f64];
        let mut problems = Vec::new();
        if let Some(vc) = &s.viewport_camera {
            match gf.nodes.iter().position(|n| &*n.id == vc.as_str() && n.kind == "camera") {
                Some(k) => gf.camera = Some(k as u32),
                None => problems.push(format!("scene360: viewport camera {vc} is not a camera")),
            }
        }
        let mut has_kids = vec![false; gf.nodes.len()];
        for n in &gf.nodes {
            if let Some(pi) = n.parent {
                has_kids[pi as usize] = true;
            }
        }
        let mut blocked = vec![false; gf.nodes.len()];
        for k in 0..gf.nodes.len() {
            let under_iso = gf.nodes[k]
                .parent
                .map(|pi| blocked[pi as usize] || Self::isolated(&gf.nodes[pi as usize], has_kids[pi as usize], true))
                .unwrap_or(false);
            blocked[k] = under_iso;
            let n = &mut gf.nodes[k];
            if !under_iso && n.three_d.is_none() && !matches!(n.kind, "object3D" | "camera") {
                n.three_d = Some([0.0, 0.0, 0.0]);
            }
        }
        let d = self.gpu.device.clone();
        // the faces come back at the quality tier's size (half a side at draft)
        let scale = Tier::of(self.quality.unwrap_or(p.scene.project.quality)).scale;
        let fside = (side as f64 * scale).round().max(1.0) as u32;
        let faces = d.create_texture(&wgpu::TextureDescriptor {
            label: Some("360-faces"),
            size: wgpu::Extent3d { width: fside, height: fside, depth_or_array_layers: 6 * eyes.len() as u32 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: resources::FORMAT,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        let rots = face_rotations();
        let mut stats = RenderStats::default();
        for (e, (shift, _)) in eyes.iter().enumerate() {
            for (k, rot) in rots.iter().enumerate() {
                self.view_override = Some(ViewOverride {
                    face: *rot,
                    eye_shift: *shift,
                    fov: 90.0,
                    frame: [g.size[0] as f32, g.size[1] as f32],
                });
                let f = match provider.as_mut() {
                    Some(pv) => {
                        let mut wrap = |st: f64| pv(st);
                        self.render_with(&gf, p, Some(&mut wrap))
                    }
                    None => self.render_with(&gf, p, None),
                };
                let mut enc = d.create_command_encoder(&Default::default());
                enc.copy_texture_to_texture(
                    wgpu::TexelCopyTextureInfo {
                        texture: &f.texture.tex,
                        mip_level: 0,
                        origin: wgpu::Origin3d::ZERO,
                        aspect: wgpu::TextureAspect::All,
                    },
                    wgpu::TexelCopyTextureInfo {
                        texture: &faces,
                        mip_level: 0,
                        origin: wgpu::Origin3d { x: 0, y: 0, z: (e * 6 + k) as u32 },
                        aspect: wgpu::TextureAspect::All,
                    },
                    wgpu::Extent3d {
                        width: fside.min(f.texture.size[0]),
                        height: fside.min(f.texture.size[1]),
                        depth_or_array_layers: 1,
                    },
                );
                self.gpu.queue.submit([enc.finish()]);
                let fs = f.stats;
                stats.draws += fs.draws;
                stats.targets += fs.targets;
                stats.fx_passes += fs.fx_passes;
                stats.objects3d += fs.objects3d;
                stats.triangles += fs.triangles;
                stats.splats += fs.splats;
                stats.video_frames += fs.video_frames;
                stats.subframes += fs.subframes;
                stats.decode_wait += fs.decode_wait;
                stats.vector_seconds += fs.vector_seconds;
                stats.sim_rigid_seconds += fs.sim_rigid_seconds;
                stats.sim_ocean_seconds += fs.sim_ocean_seconds;
                stats.sim_smoke_seconds += fs.sim_smoke_seconds;
                stats.sim_particles_seconds += fs.sim_particles_seconds;
                stats.draw_prep_seconds += fs.draw_prep_seconds;
                stats.volume_prep_seconds += fs.volume_prep_seconds;
                stats.pt_assemble_seconds += fs.pt_assemble_seconds;
                stats.pt_bvh_seconds += fs.pt_bvh_seconds;
                stats.pt_pack_seconds += fs.pt_pack_seconds;
                for m in fs.unsupported {
                    if !stats.unsupported.contains(&m) {
                        stats.unsupported.push(m);
                    }
                }
                for m in fs.errors {
                    if !stats.errors.contains(&m) {
                        stats.errors.push(m);
                    }
                }
            }
        }
        self.view_override = None;
        stats.unsupported.extend(problems);
        let out = Arc::new(resources::create(&d, &self.bgl1, [w as u32, hgt as u32], 1, "360"));
        let sp = self.sphere_pipe();
        let mut enc = d.create_command_encoder(&Default::default());
        {
            let mut ubytes = Vec::new();
            for (_, region) in &eyes {
                let u =
                    SphereU { faces: rots.map(|r| r.to_cols_array_2d()), mode: [mode, 0.0, 0.0, 0.0], region: *region };
                let mut b = bytemuck::bytes_of(&u).to_vec();
                b.resize(b.len().div_ceil(256) * 256, 0);
                ubytes.push(b);
            }
            let binds: Vec<(wgpu::BindGroup, wgpu::Buffer)> = ubytes
                .iter()
                .enumerate()
                .map(|(e, b)| {
                    use wgpu::util::DeviceExt;
                    let buf = d.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                        label: Some("360"),
                        contents: b,
                        usage: wgpu::BufferUsages::UNIFORM,
                    });
                    let view = faces.create_view(&wgpu::TextureViewDescriptor {
                        dimension: Some(wgpu::TextureViewDimension::D2Array),
                        base_array_layer: (e * 6) as u32,
                        array_layer_count: Some(6),
                        ..Default::default()
                    });
                    let bg = d.create_bind_group(&wgpu::BindGroupDescriptor {
                        label: Some("360"),
                        layout: &sp.bgl,
                        entries: &[
                            wgpu::BindGroupEntry { binding: 0, resource: buf.as_entire_binding() },
                            wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::TextureView(&view) },
                            wgpu::BindGroupEntry { binding: 2, resource: wgpu::BindingResource::Sampler(&sp.smp) },
                        ],
                    });
                    (bg, buf)
                })
                .collect();
            let mut rp = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("360"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &out.view,
                    resolve_target: None,
                    depth_slice: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            rp.set_pipeline(&sp.pipe);
            for ((bg, _), (_, region)) in binds.iter().zip(&eyes) {
                rp.set_viewport(region[0], region[1], region[2], region[3], 0.0, 1.0);
                rp.set_bind_group(0, bg, &[]);
                rp.draw(0..3, 0..1);
            }
        }
        self.gpu.queue.submit([enc.finish()]);
        stats.targets += 1;
        Frame { texture: out, stats }
    }

    /// Frame clip space → this target's clip space.
    fn clip_fix(space: &Space, cam_size: [f32; 2]) -> Mat4 {
        let (tw, th) = (space.size[0] as f32, space.size[1] as f32);
        let to_px = Mat4::from_cols_array(&[
            cam_size[0] * 0.5,
            0.0,
            0.0,
            0.0,
            0.0,
            -cam_size[1] * 0.5,
            0.0,
            0.0,
            0.0,
            0.0,
            1.0,
            0.0,
            cam_size[0] * 0.5,
            cam_size[1] * 0.5,
            0.0,
            1.0,
        ]);
        let to_ndc = Mat4::from_cols_array(&[
            2.0 / tw,
            0.0,
            0.0,
            0.0,
            0.0,
            -2.0 / th,
            0.0,
            0.0,
            0.0,
            0.0,
            1.0,
            0.0,
            -1.0,
            1.0,
            0.0,
            1.0,
        ]);
        to_ndc * embed(&space.xform) * to_px
    }

    /// Projection of 2.5D layers: target pixels (with z) → target clip space through the frame camera.
    pub(super) fn proj25(&self, g: &FrameGraph, p: &Program, space: &Space) -> Mat4 {
        let (cam, _, cam_size) = self.camera3(g, p, [g.size[0] as f32, g.size[1] as f32]);
        let back = space.xform.inverse().map(|a| embed(&a)).unwrap_or(Mat4::IDENTITY);
        Self::clip_fix(space, cam_size) * cam.view_proj() * back
    }

    /// The 3D objects of node `i`'s block, which render together in one pass (one depth buffer, one
    /// environment dome): the run of its siblings, in paint order, that no drawn 2D sibling interrupts
    /// (cameras and 2.5D nodes belong to the 3D content and do not end a block). Blocks
    /// composite in document order like other nodes. Motion blur accumulates a whole pass over the
    /// shutter, driven by its first member: rendering blurred objects one pass each would repaint the dome
    /// over the objects drawn before them.
    pub(super) fn three_members(g: &FrameGraph, i: usize) -> Vec<usize> {
        let parent = g.nodes[i].parent;
        let mut run = Vec::new();
        let mut found = false;
        for (j, m) in g.nodes.iter().enumerate().filter(|(_, m)| m.parent == parent) {
            let three = sr_eval::draws_in_3d(m.kind) || m.kind == "camera" || m.three_d.is_some();
            if !three && m.draw {
                // a drawn 2D sibling ends the block
                if found {
                    break;
                }
                run.clear();
                continue;
            }
            if sr_eval::draws_in_3d(m.kind) && visible3(g, j) {
                run.push(j);
            }
            found |= j == i;
        }
        run
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) fn three_run(
        &mut self,
        plan: &mut Plan,
        ctx: &Ctx,
        i: usize,
        space: &Space,
        iso_op: f64,
        cmds: &mut Vec<Cmd>,
        root_hash: u64,
    ) {
        let g = ctx.g;
        let n = &g.nodes[i];
        let members = Self::three_members(g, i);
        if let Some(why) = crate::gpu::three_d_warning(&self.gpu.info) {
            if members.first() == Some(&i) {
                plan.stats.unsupported.push(format!("{}: {why}", n.id));
            }
            return;
        }
        if members.first() != Some(&i) || !visible3(g, i) {
            return;
        }
        let frame = [g.size[0] as f32, g.size[1] as f32];
        let (cam, ex, cam_size) = self.camera3(g, ctx.p, frame);
        let (mut lights, env) = self.lights3(plan, ctx);
        if lights.is_empty()
            && env.is_none()
            && ctx.p.scene.lights.as_ref().map(|l| l.lights.is_empty()).unwrap_or(true)
        {
            // no lights in the document: neutral rig, ambient 0.35
            // (a uniform environment, so metals reflect it) and a directional key from the upper left
            // front whose irradiance π · 0.65 brings a white Lambertian surface facing it to 1
            let base = Light3 {
                contact: 0.0,
                kind: LightKind::Directional,
                pos: Vec3::ZERO,
                dir: Vec3::new(0.45, 0.7, 0.55).normalize(),
                right: Vec3::X,
                color: Vec3::splat(std::f32::consts::PI * 0.65),
                range: 0.0,
                falloff: 2.0,
                cos_outer: 0.0,
                cos_inner: 0.0,
                cast_shadow: false,
                softness: 0.0,
                bias: 0.0005,
                map_size: 1024,
                size: [0.0; 3],
                ies: None,
                affects_diffuse: true,
                affects_specular: true,
            };
            lights.push(Light3 { kind: LightKind::Ambient, color: Vec3::splat(0.35), ..base.clone() });
            lights.push(base);
        }
        let mut draws = Vec::new();
        let mut splats = Vec::new();
        let mut volumes = Vec::new();
        for &j in &members {
            let opacity = if iso_op > 0.0 { (g.nodes[j].world_opacity / iso_op) as f32 } else { 0.0 };
            if opacity <= 0.0 {
                continue;
            }
            if g.nodes[j].kind == "particles3D" {
                let clock = std::time::Instant::now();
                self.particle_draws3(plan, ctx, j, &cam, opacity.min(1.0), &mut draws);
                plan.stats.draw_prep_seconds += clock.elapsed().as_secs_f64();
            } else if attrs(&g.nodes[j]).str("primitive").as_deref() == Some("volume") {
                if volumes.len() >= 64 {
                    plan.stats.errors.push(format!("{}: a 3D pass supports at most 64 volume domains", n.id));
                    return;
                }
                let clock = std::time::Instant::now();
                let drawn = self.volume_draw(ctx, j, opacity.min(1.0));
                plan.stats.volume_prep_seconds += clock.elapsed().as_secs_f64();
                match drawn {
                    Ok(Some(volume)) => {
                        let cap = self.gpu.device.limits().max_storage_buffer_binding_size;
                        let bytes =
                            crate::volume::bytes(&volumes) + crate::volume::bytes(std::slice::from_ref(&volume));
                        if bytes > cap {
                            plan.stats.errors.push(format!(
                                "{}: volume grids exceed the device's {} MiB scene binding",
                                n.id,
                                cap >> 20
                            ));
                            return;
                        }
                        if let Some(note) = volume.light_grid_note() {
                            let note = format!("{}: {note}", g.nodes[j].id);
                            if !plan.stats.unsupported.contains(&note) {
                                plan.stats.unsupported.push(note);
                            }
                        }
                        volumes.push(volume);
                    }
                    Ok(None) => {}
                    // a failed simulation is already reported as an error with its cause
                    Err(_) if g.failed(&g.nodes[j].id) => {}
                    Err(error) => plan.stats.errors.push(format!("{}: {error}", g.nodes[j].id)),
                }
            } else {
                let clock = std::time::Instant::now();
                self.object_draws(plan, ctx, j, opacity.min(1.0), &mut draws, &mut splats);
                plan.stats.draw_prep_seconds += clock.elapsed().as_secs_f64();
            }
        }
        if draws.is_empty()
            && splats.is_empty()
            && volumes.is_empty()
            && !env.as_ref().map(|e| e.visible).unwrap_or(false)
        {
            return;
        }
        plan.stats.objects3d += draws.len();
        plan.stats.triangles += draws.iter().map(|d| d.mesh_triangles()).sum::<u64>();
        plan.stats.splats += splats.iter().map(|s| s.gpu.n as u64).sum::<u64>();
        let clip_fix = Self::clip_fix(space, cam_size);
        let scene = Scene3 {
            cam,
            clip_fix,
            size: space.size,
            exposure: ex.exposure,
            dof: ex.dof,
            lens_k1: ex.lens_k1,
            draws,
            lights,
            env,
            splats,
            volumes,
            encode_srgb: !self.working.linear,
            ao: ex.ao,
            ssr: ex.ssr,
            path: None,
        };
        let mut scene = scene;
        let limits = self.gpu.device.limits();
        // Surface/medium depth and shadows are evaluated together. Raster-authored
        // passes containing media use the transport pipeline with deterministic samples.
        let transport = ex.path.or_else(|| {
            (!scene.volumes.is_empty()).then_some(crate::pathtrace::PathOpts { samples: 4, bounces: 2, denoise: true })
        });
        if let Some(opts) = transport {
            if let Some(m) = crate::pathtrace::limit_note(&scene, &limits) {
                if !scene.volumes.is_empty() {
                    plan.stats.errors.push(format!("{}: volume pass cannot render: {m}", n.id));
                    return;
                }
                plan.stats.unsupported.push(format!("{}: {m}", n.id));
            } else {
                plan.stats
                    .unsupported
                    .extend(crate::pathtrace::notes(&scene).into_iter().map(|m| format!("{}: {m}", n.id)));
                scene.path = Some(opts);
            }
        }
        if scene.path.is_none() {
            let lost =
                crate::three::shadow_note(&scene.lights, !scene.cam.orthographic, limits.max_texture_array_layers);
            plan.stats.unsupported.extend(lost.map(|m| format!("{}: {m}", n.id)));
        }
        // Mesh upload normally creates this lazily, but a volume-only pass has no mesh.
        self.three_engine();
        self.flush_vec(plan, cmds);
        let snapshot = self.temp(plan, space.size);
        let out = self.temp(plan, space.size);
        let ids: String = members.iter().map(|j| &*g.nodes[*j].id).collect::<Vec<_>>().join(" ");
        let hash = h(&[root_hash, ctx.elements, sr_eval::rng::hash_str(&ids), hf(g.time), hf(iso_op), 0x3d3d]);
        let (w, hgt) = (space.size[0] as f64, space.size[1] as f64);
        self.bare.insert(n.id.clone());
        self.composite(plan, ctx, i, space, 1.0, out.clone(), [0.0, 0.0, w, hgt], cmds, hash);
        self.bare.remove(&n.id);
        if let Some(c) = cmds.last_mut() {
            c.pre = Some(Box::new(AdjPre { snapshot, passes: Vec::new(), three: Some(Box::new((scene, out))) }));
        }
    }
}

impl Renderer {
    /// A colour value in the compositor's stored working representation.
    fn stored(&self, v: Option<&Value>, d: [f64; 4]) -> [f64; 4] {
        match v {
            Some(Value::Color(c)) => self.working.from_literal(*c),
            Some(Value::Str(s)) if s.starts_with("token:") => {
                self.tokens.get(&s[6..]).map(|c| self.working.from_literal(*c)).unwrap_or(d)
            }
            _ => d,
        }
    }

    /// Draws an emitter's particles into an offscreen and composites it as the emitter's layer.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn emit_particles(
        &mut self,
        plan: &mut Plan,
        ctx: &Ctx,
        i: usize,
        space: &Space,
        op: f64,
        cmds: &mut Vec<Cmd>,
        root_hash: u64,
    ) {
        let n = &ctx.g.nodes[i];
        let Some(pf) = n.particles.clone() else { return };
        let white = self.working.from_literal([1.0; 4]);
        let c0 = self.stored(pf.color0.as_ref(), white);
        let c1 = pf.color1.as_ref().map(|v| self.stored(Some(v), c0)).unwrap_or(c0);
        // a sprite sheet from an image asset
        let mut sprite = None;
        if let Some(key) = pf.sprite.as_deref() {
            match self.asset(ctx.p, key) {
                Some((AssetsChild::Image(img), doc)) => {
                    let base = ctx.p.base_dirs.get(doc).cloned().unwrap_or_default();
                    if let sr_model::assets::Resolved::Local(path) = sr_model::assets::resolve(&img.src, &base) {
                        if let Some(t) = self.image(
                            path,
                            img.color_space,
                            img.transfer,
                            img.alpha,
                            img.color_profile == sr_model::model::ColorProfile::Embedded,
                        ) {
                            sprite = Some((key.to_string(), t.view.clone()));
                            plan.fx_temps.push(t);
                        }
                    }
                }
                _ => plan.stats.errors.push(format!("{}: sprite {key} is not an image asset", n.id)),
            }
        }
        let xf = space.xform;
        let scale = Xf(xf.0).max_scale();
        let turn = xf.0[1].atan2(xf.0[0]);
        let shape = match pf.shape {
            sr_eval::sim::ParticleShape::Disc => 0.0,
            sr_eval::sim::ParticleShape::Square => 1.0,
            sr_eval::sim::ParticleShape::Sprite if sprite.is_some() => 2.0,
            sr_eval::sim::ParticleShape::Sprite => 0.0,
            sr_eval::sim::ParticleShape::Streak => 3.0,
        };
        let (cols, rows) = (pf.cols.max(1), pf.rows.max(1));
        let mut insts = Vec::with_capacity(pf.pos.len() * if pf.trail > 0.0 && shape != 3.0 { 2 } else { 1 });
        for k in 0..pf.pos.len() {
            let t = pf.color_t[k] as f64;
            let a = pf.alpha[k] as f64;
            let c: [f32; 4] = std::array::from_fn(|ch| (c0[ch] + (c1[ch] - c0[ch]) * t) as f32);
            // premultiply by the colour's own alpha and the particle's opacity
            let alpha = (c[3] as f64 * a) as f32;
            let col = [c[0] * alpha, c[1] * alpha, c[2] * alpha, alpha];
            let p = xf.apply([pf.pos[k][0] as f64, pf.pos[k][1] as f64]);
            let v = [pf.vel[k][0] as f64 * scale, pf.vel[k][1] as f64 * scale];
            let speed = (v[0] * v[0] + v[1] * v[1]).sqrt();
            let size = pf.size[k] as f64 * scale;
            let heading = v[1].atan2(v[0]);
            if shape == 3.0 {
                let len = (speed * if pf.trail > 0.0 { pf.trail } else { 1.0 / 30.0 }).max(size);
                let back = (len - size) * 0.5;
                insts.push(crate::particles::Inst {
                    centre_half: [
                        (p[0] - heading.cos() * back) as f32,
                        (p[1] - heading.sin() * back) as f32,
                        (len * 0.5) as f32,
                        (size * 0.5) as f32,
                    ],
                    rot_shape: [heading as f32, 3.0, 0.0, 0.0],
                    color: col,
                    cell: [0.0; 4],
                });
                continue;
            }
            if pf.trail > 0.0 && speed > 0.0 {
                let len = speed * pf.trail;
                insts.push(crate::particles::Inst {
                    centre_half: [
                        (p[0] - heading.cos() * len * 0.5) as f32,
                        (p[1] - heading.sin() * len * 0.5) as f32,
                        (len * 0.5).max(size * 0.5) as f32,
                        (size * 0.4) as f32,
                    ],
                    rot_shape: [heading as f32, 3.0, 0.0, 0.0],
                    color: col.map(|x| x * 0.5),
                    cell: [0.0; 4],
                });
            }
            let cell = pf.frame[k];
            let (cx, cy) = ((cell % cols) as f32, (cell / cols % rows) as f32);
            insts.push(crate::particles::Inst {
                centre_half: [p[0] as f32, p[1] as f32, (size * 0.5) as f32, (size * 0.5) as f32],
                rot_shape: [((pf.rot[k] as f64).to_radians() + turn) as f32, shape as f32, 0.0, 0.0],
                color: col,
                cell: [cx / cols as f32, cy / rows as f32, (cx + 1.0) / cols as f32, (cy + 1.0) / rows as f32],
            });
        }
        let tex = self.temp(plan, space.size);
        plan.jobs.push(Job {
            target: tex.clone(),
            clear: true,
            cmds: Vec::new(),
            root: false,
            fx: Vec::new(),
            flow: None,
            draw: false,
            parts: Some(Box::new(crate::particles::ParticleJob { insts, sprite })),
        });
        let (w, hgt) = (space.size[0] as f64, space.size[1] as f64);
        // the cloud is drawn in target pixels: a node placed in 2.5D carries that sheet through the frame camera
        self.frame_three = n.three_d.map(|t| (t, n.anchor, self.proj25(ctx.g, ctx.p, space)));
        let placed = self.frame_three.map(|(t, a, m)| {
            let cols = m.to_cols_array().map(|v| v.to_bits() as u64);
            h(&[hf(t[0]), hf(t[1]), hf(t[2]), hf(a[0]), hf(a[1]), h(&cols)])
        });
        let hash = h(&[
            root_hash,
            sr_eval::rng::hash_str(&n.id),
            hf(ctx.g.time),
            pf.pos.len() as u64,
            placed.unwrap_or(0),
            0x5a17,
        ]);
        self.composite(plan, ctx, i, space, op, tex, [0.0, 0.0, w, hgt], cmds, hash);
        self.frame_three = None;
    }
}

fn visible3(g: &FrameGraph, j: usize) -> bool {
    g.nodes[j].draw && flag(&attrs(&g.nodes[j]), "visible", true)
}

#[cfg(test)]
mod scope_tests {
    use super::*;

    #[test]
    fn instantiated_three_dimensional_parents_resolve_per_instance() {
        for weight in [1.0_f32, 0.5] {
            let link = if weight == 1.0 { r#"parent="carrier""# } else { "" };
            let constraint = if weight == 1.0 {
                ""
            } else {
                r#"<transformConstraint type="parent" target="carrier" influence="0.5"/>"#
            };
            let xml = format!(
                r#"<scene version="1.2"><project width="32" height="32" fps="10" duration="1"/>
              <symbols><symbol id="assembly" width="32" height="32">
              <object3D id="carrier" primitive="box" x="5" rotationY="90"/>
              <object3D id="child" primitive="box" x="1" y="2" z="3" {link}>{constraint}</object3D>
              </symbol></symbols><composition><instance id="a" symbol="assembly"/><instance id="b" symbol="assembly" x="100"/></composition></scene>"#
            );
            let doc = sr_model::load_str(&xml, &sr_model::LoadOptions::without_assets()).unwrap();
            let graph = sr_eval::Evaluator::new(&doc, &Default::default()).unwrap().evaluate(0.0);
            for (id, offset) in [("a/child", 0.0), ("b/child", 100.0)] {
                let i = graph.nodes.iter().position(|n| &*n.id == id).unwrap();
                let actual = Renderer::world3(&graph, &[], i, 0).transform_point3(Vec3::ZERO);
                let parent = Mat4::from_translation(Vec3::new(5.0 + offset, 0.0, 0.0))
                    * Mat4::from_rotation_y(std::f32::consts::FRAC_PI_2);
                let expected = toward(parent, weight).transform_point3(Vec3::new(1.0, 2.0, 3.0));
                assert!((actual - expected).length() < 1e-4, "{id}, weight={weight}: {actual:?} != {expected:?}");
            }
        }
    }
}
