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

/// The blobs of a clay object at this frame: static attributes, overridden by the animated
/// values of each `<blob>` (part `{id}/blob[k]`).
fn clay_blobs(n: &sr_eval::FrameNode) -> Vec<sr_3d::clay::Blob> {
    let mut out = Vec::new();
    for (k, c) in sr_model::element::children(&*n.elem).iter().filter(|c| c.element_name() == "blob").enumerate() {
        let key = format!("{}/blob[{k}]", n.id);
        let props = n.parts.iter().find(|p| *p.key == key).map(|p| &p.props);
        let a = Attrs { e: *c, props };
        let f = |name: &str, d: f64| a.num(name, d) as f32;
        let rot = glam::Quat::from_euler(
            glam::EulerRot::YXZ,
            f("rotationY", 0.0).to_radians(),
            f("rotationX", 0.0).to_radians(),
            f("rotation", 0.0).to_radians(),
        );
        out.push(sr_3d::clay::Blob {
            shape: match a.str("shape").as_deref() {
                Some("box") => sr_3d::clay::BlobShape::Box,
                Some("capsule") => sr_3d::clay::BlobShape::Capsule,
                Some("torus") => sr_3d::clay::BlobShape::Torus,
                _ => sr_3d::clay::BlobShape::Sphere,
            },
            center: Vec3::new(f("x", 0.0), f("y", 0.0), f("z", 0.0)),
            rotation: rot,
            radius: f("radius", 30.0),
            size: Vec3::new(f("width", 60.0), f("height", 60.0), f("depth", 60.0)),
            length: f("length", 60.0),
            blend: f("blend", 10.0),
            subtract: a.num("subtract", 0.0) != 0.0 || a.str("subtract").as_deref() == Some("true"),
        });
    }
    out
}

fn attrs<'a>(n: &'a sr_eval::FrameNode) -> Attrs<'a> {
    Attrs { e: &*n.elem, props: Some(&n.props) }
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

impl Renderer {
    fn three_engine(&mut self) -> &mut ThreeEngine {
        let g = &self.gpu;
        self.three.get_or_insert_with(|| Box::new(ThreeEngine::new(g.device.clone(), g.queue.clone())))
    }

    /// World matrix of a 3D object: its @parent chain of 3D objects, then the enclosing 2D world.
    fn world3(g: &FrameGraph, i: usize, depth: u32) -> Mat4 {
        let n = &g.nodes[i];
        let own = local3(&attrs(n));
        if depth < 32 {
            if let Some(AttrValue::Str(pid)) = n.elem.get_attr("parent") {
                if let Some(j) = g.nodes.iter().position(|m| &*m.id == pid.as_str()) {
                    if g.nodes[j].kind == "object3D" {
                        return Self::world3(g, j, depth + 1) * own;
                    }
                    return embed(&g.nodes[j].world) * own;
                }
            }
        }
        match n.parent {
            Some(p) => embed(&g.nodes[p as usize].world) * own,
            None => own,
        }
    }

    /// World position of a node for look-at and focus targets.
    fn target_point(g: &FrameGraph, id: &str) -> Option<Vec3> {
        let j = g.nodes.iter().position(|m| &*m.id == id)?;
        let n = &g.nodes[j];
        if n.kind == "object3D" {
            return Some(Self::world3(g, j, 0).transform_point3(Vec3::ZERO));
        }
        let p = n.world.apply(n.anchor);
        Some(Vec3::new(p[0] as f32, p[1] as f32, n.three_d.map(|t| t[0]).unwrap_or(0.0) as f32))
    }

    /// The frame camera (the active camera node, else the default 2.5D camera).
    pub(super) fn camera3(&self, g: &FrameGraph, frame: [f32; 2]) -> (CameraView, CamExtras, [f32; 2]) {
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
            cp.fov = a.num("fov", 60.0) as f32;
            if let Some(f) = a.opt("focalLength") {
                cp.fov = camera::fov_of_lens(f as f32, sensor);
            }
            focal_mm = camera::lens_of_fov(cp.fov, sensor);
            cp.orthographic = a.str("projection").as_deref() == Some("orthographic");
            cp.ortho_height = a.opt("orthoHeight").map(|v| v as f32);
            cp.near = a.num("near", 0.1) as f32;
            cp.far = a.num("far", 10000.0) as f32;
            // absolute scene position; the defaults put the eye at the frame's top-left corner on z = 0
            cp.position = Some(Vec3::new(a.num("x", 0.0) as f32, a.num("y", 0.0) as f32, a.num("z", 0.0) as f32));
            cp.yaw = a.num("yaw", 0.0) as f32;
            cp.pitch = a.num("pitch", 0.0) as f32;
            cp.roll = a.num("roll", 0.0) as f32;
            if let Some(t) = a.str("target") {
                cp.target = Self::target_point(g, &t);
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
                let seed = sa.opt("seed").map(|s| s as u64).unwrap_or_else(|| sr_eval::rng::hash_str(&n.id));
                let (dx, dy, droll, dz) = camera::shake(
                    sa.num("amplitude", 10.0) as f32,
                    sa.num("frequency", 2.0) as f32,
                    sa.num("rotation", 0.0) as f32,
                    sa.num("zoom", 0.0) as f32,
                    sa.num("octaves", 2.0) as u32,
                    seed,
                    t,
                );
                cp.offset += Vec3::new(dx, dy, 0.0);
                cp.roll += droll;
                cp.fov = (cp.fov / dz.max(0.05)).clamp(0.1, 179.0);
            }
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
            if let Some(t) = focus_target.and_then(|t| Self::target_point(g, &t)) {
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
        match image::open(&path) {
            Ok(img) => {
                let img = img.to_rgba8();
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
                    let dir = path.parent().unwrap_or(std::path::Path::new(".")).to_path_buf();
                    if let Some(f) = &mx.base_color_map {
                        maps[0] = self.map_texture(plan, &dir, &f.display().to_string(), true, id);
                    }
                    if let Some(f) = &mx.normal_map {
                        maps[1] = self.map_texture(plan, &dir, &f.display().to_string(), false, id);
                    }
                    if let Some(f) = &mx.roughness_map {
                        // a roughness image drives the green channel of the metallic-roughness slot
                        maps[2] = self.map_texture(plan, &dir, &f.display().to_string(), false, id);
                        p.metallic = 0.0;
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
        let a = attrs(n);
        let blobs = clay_blobs(n);
        let res = a.num("resolution", 64.0).clamp(8.0, 256.0) as u32;
        let seed = a.opt("seed").map(|s| s as u64).unwrap_or_else(|| sr_eval::rng::hash_str(&n.id));
        let finish =
            sr_3d::clay::Finish { amount: a.num("fingerprints", 0.0) as f32, seed, boil: a.num("boil", 0.0) as f32 };
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
        let a = attrs(n);
        let kind = a.str("primitive").unwrap_or_else(|| "box".into());
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
                "text|{}|{:?}|{:?}|{depth}|{bevel}",
                n.text.as_deref().or(a.str("text").as_deref()).unwrap_or(""),
                a.str("font"),
                hh
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
            "plane" => Ok(sr_3d::prim::plane(w, hh.unwrap_or(2.0 * r), 1)),
            "cylinder" => Ok(sr_3d::prim::cylinder(r, r, hh.unwrap_or(2.0 * r), segs)),
            "cone" => Ok(sr_3d::prim::cylinder(0.0, r, hh.unwrap_or(2.0 * r), segs)),
            "torus" => Ok(sr_3d::prim::torus(r, depth.min(r), segs)),
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
                let polys = crate::text::outline_polygons(&mut tc, ctx.p, &text, a.str("font").as_deref(), size, 0.25);
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

    fn mesh_asset(
        &mut self,
        plan: &mut Plan,
        ctx: &Ctx,
        n: &sr_eval::FrameNode,
    ) -> Option<(String, Arc<Result<Asset, String>>)> {
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
        Some((key, asset))
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
        let a = attrs(n);
        let world = Self::world3(g, j, 0);
        let (cast, receive) = (flag(&a, "castShadow", true), flag(&a, "receiveShadow", true));
        let instances = a.num("instances", 1.0).max(1.0) as u32;
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
        let default_mat =
            || (MaterialParams { base_color: [0.8, 0.8, 0.8, 1.0], ..Default::default() }, Maps::default());
        if a.str("primitive").as_deref() != Some("mesh") {
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
                    instances,
                });
            }
            return;
        }
        let Some((key, asset)) = self.mesh_asset(plan, ctx, n) else { return };
        match &*asset {
            Ok(Asset::Splats(s)) => {
                let gpu = match self.three_engine().splat_cache.get(&key) {
                    Some(g) => g.clone(),
                    None => {
                        let g = self.three_engine().upload_splats(s);
                        self.three_engine().splat_cache.insert(key.clone(), g.clone());
                        g
                    }
                };
                splats.push(SplatDraw { gpu, model: world * s.basis, opacity });
            }
            Ok(Asset::Model(model)) => {
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
                for item in sr_3d::anim::draw_list(model, &locals, &weights, morph.as_deref()) {
                    let prim = &model.primitives[item.prim];
                    let mkey = format!("{key}#{}", item.prim);
                    let mesh = match self.three_engine().meshes.get(&mkey) {
                        Some(m) => m.clone(),
                        None => {
                            let m = self.three_engine().upload_mesh(&prim.vertices, &prim.indices);
                            self.three_engine().meshes.insert(mkey, m.clone());
                            m
                        }
                    };
                    let (material, maps) = match &doc_mat {
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
                                                    let up = self
                                                        .three_engine()
                                                        .upload_rgba8(tx.width, tx.height, &tx.rgba, tx.srgb);
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
                        Some(vs) => MeshSrc::Deformed(vs, mesh),
                        None => MeshSrc::Cached(mesh),
                    };
                    draws.push(Draw3 {
                        mesh: src,
                        model: world * model.basis * item.matrix,
                        material,
                        maps,
                        opacity,
                        cast_shadow: cast,
                        receive_shadow: receive,
                        instances,
                    });
                }
            }
            Err(_) => {}
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
            let deg = |n: &str| (a.num(n, 0.0) as f32).to_radians();
            let rot = Mat4::from_rotation_y(deg("yaw"))
                * Mat4::from_rotation_x(deg("pitch"))
                * Mat4::from_rotation_z(deg("roll"));
            let dir = rot.transform_vector3(Vec3::Z).normalize();
            let right = rot.transform_vector3(Vec3::X).normalize();
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
                        rotation: deg("yaw"),
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
            Self::constrain_light(plan, ctx.g, l as &dyn Element, &l.id, &mut lt);
            out.push(lt);
        }
        (out, env)
    }

    /// Applies a light's transform constraints: look-at aims it, parent and copy-position
    /// move it to the target (plus offset), follow-path rides a path, distance clamps its range.
    fn constrain_light(plan: &mut Plan, g: &FrameGraph, e: &dyn Element, id: &str, lt: &mut Light3) {
        for c in sr_model::element::children(e) {
            if c.element_name() != "transformConstraint" {
                continue;
            }
            let ca = Attrs { e: c, props: None };
            let kind = ca.str("type").unwrap_or_default();
            let w = ca.num("influence", 1.0).clamp(0.0, 1.0) as f32;
            let target = ca.str("target").and_then(|t| Self::target_point(g, &t));
            let off = Vec3::new(ca.num("offsetX", 0.0) as f32, ca.num("offsetY", 0.0) as f32, 0.0);
            match (kind.as_str(), target) {
                ("look-at", Some(t)) => {
                    let want = (t - lt.pos).normalize_or(lt.dir);
                    let dir = lt.dir.lerp(want, w).normalize_or(want);
                    let up = if dir.y.abs() > 0.99 { Vec3::Z } else { Vec3::Y };
                    lt.right = up.cross(dir).normalize_or(Vec3::X) * -1.0;
                    lt.dir = dir;
                }
                ("parent" | "copy-position" | "copy-transform", Some(t)) => lt.pos = lt.pos.lerp(t + off, w),
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
            let pipe = d.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
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
            });
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
        let faces = d.create_texture(&wgpu::TextureDescriptor {
            label: Some("360-faces"),
            size: wgpu::Extent3d { width: side, height: side, depth_or_array_layers: 6 * eyes.len() as u32 },
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
                    wgpu::Extent3d { width: side, height: side, depth_or_array_layers: 1 },
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
    pub(super) fn proj25(&self, g: &FrameGraph, space: &Space) -> Mat4 {
        let (cam, _, cam_size) = self.camera3(g, [g.size[0] as f32, g.size[1] as f32]);
        let back = space.xform.inverse().map(|a| embed(&a)).unwrap_or(Mat4::IDENTITY);
        Self::clip_fix(space, cam_size) * cam.view_proj() * back
    }

    /// Whether node `i` starts a 3D run (or draws alone) and, if so, emits the run as one layer.
    /// The 3D objects that share node `i`'s parent and render together in one pass (one depth buffer, one
    /// environment dome). Motion blur accumulates this whole pass over the shutter, driven by its first member:
    /// rendering blurred objects one pass each would repaint the dome over the objects drawn before them.
    pub(super) fn three_members(g: &FrameGraph, i: usize) -> Vec<usize> {
        let parent = g.nodes[i].parent;
        g.nodes
            .iter()
            .enumerate()
            .filter(|(j, m)| m.kind == "object3D" && m.parent == parent && visible3(g, *j))
            .map(|(j, _)| j)
            .collect()
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
        if members.first() != Some(&i) || !visible3(g, i) {
            return;
        }
        let frame = [g.size[0] as f32, g.size[1] as f32];
        let (cam, ex, cam_size) = self.camera3(g, frame);
        let (mut lights, env) = self.lights3(plan, ctx);
        if lights.is_empty()
            && env.is_none()
            && ctx.p.scene.lights.as_ref().map(|l| l.lights.is_empty()).unwrap_or(true)
        {
            // no lights in the document: a headlight along the view and a soft fill
            let fwd = cam.view.inverse().transform_vector3(Vec3::Z).normalize();
            let base = Light3 {
                contact: 0.0,
                kind: LightKind::Directional,
                pos: Vec3::ZERO,
                dir: (fwd + Vec3::new(0.3, 0.4, 0.0)).normalize(),
                right: Vec3::X,
                color: Vec3::splat(2.5),
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
            lights.push(Light3 { kind: LightKind::Ambient, color: Vec3::splat(0.25), ..base.clone() });
            lights.push(base);
        }
        let mut draws = Vec::new();
        let mut splats = Vec::new();
        for &j in &members {
            let opacity = if iso_op > 0.0 { (g.nodes[j].world_opacity / iso_op) as f32 } else { 0.0 };
            if opacity <= 0.0 {
                continue;
            }
            self.object_draws(plan, ctx, j, opacity.min(1.0), &mut draws, &mut splats);
        }
        if draws.is_empty() && splats.is_empty() && !env.as_ref().map(|e| e.visible).unwrap_or(false) {
            return;
        }
        plan.stats.objects3d += draws.len();
        plan.stats.triangles += draws.iter().map(|d| d.mesh_triangles() * d.instances.max(1) as u64).sum::<u64>();
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
            encode_srgb: !self.working.linear,
            ao: ex.ao,
            ssr: ex.ssr,
            path: None,
        };
        let mut scene = scene;
        if let Some(opts) = ex.path {
            if scene.cam.orthographic || scene.clip_fix != Mat4::IDENTITY {
                plan.stats.unsupported.push(format!(
                    "{}: path tracing needs a perspective camera over the whole frame; rasterised instead",
                    n.id
                ));
            } else {
                plan.stats
                    .unsupported
                    .extend(crate::pathtrace::notes(&scene).into_iter().map(|m| format!("{}: {m}", n.id)));
                scene.path = Some(opts);
            }
        }
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
                        if let Some(t) = self.image(path, img.color_space, img.transfer, img.alpha) {
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
        let hash = h(&[root_hash, sr_eval::rng::hash_str(&n.id), hf(ctx.g.time), pf.pos.len() as u64, 0x5a17]);
        self.composite(plan, ctx, i, space, op, tex, [0.0, 0.0, w, hgt], cmds, hash);
    }
}

fn visible3(g: &FrameGraph, j: usize) -> bool {
    g.nodes[j].draw && flag(&attrs(&g.nodes[j]), "visible", true)
}
