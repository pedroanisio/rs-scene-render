//! Model and splat importers.
//!
//! glTF/GLB, OBJ, PLY, FBX, USDA/USDZ and `.splat` files become a [`Model`]
//! or [`Splats`]. Y-up, metre-based sources map into scene space through
//! [`crate::Y_UP_METRES`]; USD stages are first converted from their
//! `upAxis` and `metersPerUnit`, and FBX files are converted by ufbx.

use std::path::Path;

use glam::{Mat3, Mat4, Quat, Vec3};

use crate::material::srgb_to_linear;
use crate::{
    compute_normals, compute_tangents, Animation, Asset, Channel, ImportedMaterial, Interp, Model, Node,
    Path as AnimPath, Primitive, Skin, Splats, Texture, Trs, Vertex, Y_UP_METRES,
};

/// Loads a mesh asset; `format` is the schema's `@format` (else the file extension decides).
pub fn load(path: &Path, format: Option<&str>) -> Result<Asset, String> {
    let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("").to_ascii_lowercase();
    let fmt = format.unwrap_or(ext.as_str());
    let err = |e: String| format!("{}: {e}", path.display());
    match fmt {
        "gltf" | "glb" => gltf(path).map(Asset::Model).map_err(err),
        "obj" => obj(path).map(Asset::Model).map_err(err),
        "ply" => ply(&std::fs::read(path).map_err(|e| err(e.to_string()))?).map_err(err),
        "fbx" => fbx(path).map(Asset::Model).map_err(err),
        "usd" | "usda" | "usdz" | "usdc" => usd(path).map(Asset::Model).map_err(err),
        "splat" => {
            let data = std::fs::read(path).map_err(|e| err(e.to_string()))?;
            if ext == "ply" {
                ply(&data).map_err(err)
            } else {
                splat(&data).map(Asset::Splats).map_err(err)
            }
        }
        other => Err(err(format!("unknown mesh format {other}"))),
    }
}

fn finish_primitive(p: &mut Primitive, had_normals: bool) {
    if !had_normals {
        compute_normals(&mut p.vertices, &p.indices);
    }
    compute_tangents(&mut p.vertices, &p.indices);
}

fn decode_image(bytes: &[u8], srgb: bool) -> Result<Texture, String> {
    let img = image::load_from_memory(bytes).map_err(|e| format!("image: {e}"))?.to_rgba8();
    Ok(Texture { width: img.width(), height: img.height(), rgba: img.into_raw(), srgb })
}

fn load_uri(uri: &str, base: &Path) -> Result<Vec<u8>, String> {
    if let Some(rest) = uri.strip_prefix("data:") {
        let (_, b64) = rest.split_once(";base64,").ok_or("only base64 data URIs are supported")?;
        return base64_decode(b64);
    }
    let decoded = percent_decode(uri);
    std::fs::read(base.join(&decoded)).map_err(|e| format!("{decoded}: {e}"))
}

fn percent_decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' && i + 2 < b.len() {
            if let Ok(v) = u8::from_str_radix(&s[i + 1..i + 3], 16) {
                out.push(v);
                i += 3;
                continue;
            }
        }
        out.push(b[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn base64_decode(s: &str) -> Result<Vec<u8>, String> {
    let val = |c: u8| -> Option<u32> {
        Some(match c {
            b'A'..=b'Z' => (c - b'A') as u32,
            b'a'..=b'z' => (c - b'a' + 26) as u32,
            b'0'..=b'9' => (c - b'0' + 52) as u32,
            b'+' | b'-' => 62,
            b'/' | b'_' => 63,
            _ => return None,
        })
    };
    let mut out = Vec::with_capacity(s.len() * 3 / 4);
    let (mut acc, mut bits) = (0u32, 0);
    for c in s.bytes().filter(|c| !c.is_ascii_whitespace() && *c != b'=') {
        acc = (acc << 6) | val(c).ok_or("bad base64")?;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((acc >> bits) as u8);
        }
    }
    Ok(out)
}

// ---------------------------------------------------------------- glTF

fn ext_f(m: &serde_json::Map<String, serde_json::Value>, name: &str, d: f32) -> f32 {
    m.get(name).and_then(|v| v.as_f64()).map(|v| v as f32).unwrap_or(d)
}

fn ext_v3(m: &serde_json::Map<String, serde_json::Value>, name: &str, d: [f32; 3]) -> [f32; 3] {
    m.get(name)
        .and_then(|v| v.as_array())
        .filter(|a| a.len() >= 3)
        .map(|a| [0, 1, 2].map(|i| a[i].as_f64().unwrap_or(d[i] as f64) as f32))
        .unwrap_or(d)
}

/// Loads glTF 2.0 (`.gltf` with external or embedded buffers, or `.glb`).
pub fn gltf(path: &Path) -> Result<Model, String> {
    let data = std::fs::read(path).map_err(|e| e.to_string())?;
    if data.starts_with(b"glTF") {
        let header = data.get(..12).ok_or("truncated GLB header")?;
        let length = u32::from_le_bytes(header[8..12].try_into().expect("four header bytes")) as usize;
        // gltf 1.4 subtracts the header size before checking the declared length, which
        // panics on short lengths in debug builds. Validate the envelope before parsing.
        if length < 12 || length != data.len() {
            return Err(format!("invalid GLB length {length}: file contains {} bytes", data.len()));
        }
    }
    let g = gltf::Gltf::from_slice(&data).map_err(|e| e.to_string())?;
    let base = path.parent().unwrap_or(Path::new("."));
    let mut buffers = Vec::new();
    for b in g.buffers() {
        buffers.push(match b.source() {
            gltf::buffer::Source::Bin => g.blob.clone().ok_or("GLB without a binary chunk")?,
            gltf::buffer::Source::Uri(uri) => load_uri(uri, base)?,
        });
    }
    let mut m = Model { basis: Y_UP_METRES, ..Default::default() };
    // images decode lazily per (image, colour/data) pair
    let mut images: std::collections::HashMap<(usize, bool), usize> = Default::default();
    let mut tex = |t: gltf::Texture, srgb: bool, m: &mut Model| -> Option<usize> {
        let img = t.source();
        let key = (img.index(), srgb);
        if let Some(i) = images.get(&key) {
            return Some(*i);
        }
        let bytes = match img.source() {
            gltf::image::Source::View { view, .. } => {
                let b = &buffers[view.buffer().index()];
                b.get(view.offset()..view.offset() + view.length())?.to_vec()
            }
            gltf::image::Source::Uri { uri, .. } => match load_uri(uri, base) {
                Ok(b) => b,
                Err(e) => {
                    m.warnings.push(format!("texture: {e}"));
                    return None;
                }
            },
        };
        match decode_image(&bytes, srgb) {
            Ok(t) => {
                m.textures.push(t);
                images.insert(key, m.textures.len() - 1);
                Some(m.textures.len() - 1)
            }
            Err(e) => {
                m.warnings.push(e);
                None
            }
        }
    };
    for mat in g.materials() {
        let pbr = mat.pbr_metallic_roughness();
        let mut p = crate::MaterialParams {
            base_color: pbr.base_color_factor(),
            metallic: pbr.metallic_factor(),
            roughness: pbr.roughness_factor(),
            emissive: mat.emissive_factor(),
            emissive_strength: mat.emissive_strength().unwrap_or(1.0),
            alpha_mode: match mat.alpha_mode() {
                gltf::material::AlphaMode::Opaque => crate::AlphaMode::Opaque,
                gltf::material::AlphaMode::Mask => crate::AlphaMode::Mask,
                gltf::material::AlphaMode::Blend => crate::AlphaMode::Blend,
            },
            alpha_cutoff: mat.alpha_cutoff().unwrap_or(0.5),
            double_sided: mat.double_sided(),
            unlit: mat.unlit(),
            ior: mat.ior().unwrap_or(1.5),
            ..Default::default()
        };
        if let Some(t) = mat.transmission() {
            p.transmission = t.transmission_factor();
        }
        if let Some(v) = mat.volume() {
            p.thickness = v.thickness_factor() * 100.0;
            p.attenuation_color = v.attenuation_color();
            p.attenuation_distance = v.attenuation_distance() * 100.0;
        }
        if let Some(s) = mat.specular() {
            p.specular = s.specular_factor();
            p.specular_color = s.specular_color_factor();
        }
        if let Some(n) = mat.normal_texture() {
            p.normal_scale = n.scale();
        }
        if let Some(o) = mat.occlusion_texture() {
            p.occlusion_strength = o.strength();
        }
        if let Some(ext) = mat.extensions() {
            if let Some(c) = ext.get("KHR_materials_clearcoat").and_then(|v| v.as_object()) {
                p.clearcoat = ext_f(c, "clearcoatFactor", 0.0);
                p.clearcoat_roughness = ext_f(c, "clearcoatRoughnessFactor", 0.0);
            }
            if let Some(s) = ext.get("KHR_materials_sheen").and_then(|v| v.as_object()) {
                p.sheen_color = ext_v3(s, "sheenColorFactor", [0.0; 3]);
                p.sheen_roughness = ext_f(s, "sheenRoughnessFactor", 0.0);
            }
            if let Some(s) = ext.get("KHR_materials_iridescence").and_then(|v| v.as_object()) {
                p.iridescence = ext_f(s, "iridescenceFactor", 0.0);
                p.iridescence_ior = ext_f(s, "iridescenceIor", 1.3);
                p.iridescence_thickness = ext_f(s, "iridescenceThicknessMaximum", 400.0);
            }
            if let Some(s) = ext.get("KHR_materials_anisotropy").and_then(|v| v.as_object()) {
                p.anisotropy = ext_f(s, "anisotropyStrength", 0.0);
                p.anisotropy_rotation = ext_f(s, "anisotropyRotation", 0.0);
            }
            if let Some(s) = ext.get("KHR_materials_dispersion").and_then(|v| v.as_object()) {
                p.dispersion = ext_f(s, "dispersion", 0.0);
            }
        }
        if pbr.base_color_texture().map(|t| t.texture_transform().is_some()).unwrap_or(false) {
            m.warnings.push("KHR_texture_transform is ignored".into());
        }
        let maps = crate::MaterialMaps {
            base_color: pbr.base_color_texture().and_then(|t| tex(t.texture(), true, &mut m)),
            metallic_roughness: pbr.metallic_roughness_texture().and_then(|t| tex(t.texture(), false, &mut m)),
            normal: mat.normal_texture().and_then(|t| tex(t.texture(), false, &mut m)),
            occlusion: mat.occlusion_texture().and_then(|t| tex(t.texture(), false, &mut m)),
            emissive: mat.emissive_texture().and_then(|t| tex(t.texture(), true, &mut m)),
        };
        m.materials.push(ImportedMaterial { name: mat.name().unwrap_or("").to_string(), params: p, maps });
    }
    let variant_names: Vec<String> =
        g.variants().map(|vs| vs.map(|v| v.name().to_string()).collect()).unwrap_or_default();
    // meshes → primitive index lists
    let mut mesh_prims: Vec<Vec<usize>> = Vec::new();
    for mesh in g.meshes() {
        let mut list = Vec::new();
        for prim in mesh.primitives() {
            if prim.mode() != gltf::mesh::Mode::Triangles {
                m.warnings.push(format!("mesh {}: {:?} primitives are skipped", mesh.index(), prim.mode()));
                continue;
            }
            let r = prim.reader(|b| buffers.get(b.index()).map(|v| v.as_slice()));
            let Some(pos) = r.read_positions() else { continue };
            let mut vs: Vec<Vertex> = pos.map(|p| Vertex { pos: p, ..Default::default() }).collect();
            let had_normals = match r.read_normals() {
                Some(ns) => {
                    for (v, n) in vs.iter_mut().zip(ns) {
                        v.normal = n;
                    }
                    true
                }
                None => false,
            };
            if let Some(uv) = r.read_tex_coords(0) {
                for (v, t) in vs.iter_mut().zip(uv.into_f32()) {
                    v.uv = t;
                }
            }
            let indices: Vec<u32> = match r.read_indices() {
                Some(i) => i.into_u32().collect(),
                None => (0..vs.len() as u32).collect(),
            };
            let mut p = Primitive { vertices: vs, indices, material: prim.material().index(), ..Default::default() };
            if let (Some(j), Some(w)) = (r.read_joints(0), r.read_weights(0)) {
                p.joints = j.into_u16().collect();
                p.weights = w.into_f32().collect();
            }
            for (dp, dn, _) in r.read_morph_targets() {
                let n = p.vertices.len();
                p.morphs.push(crate::MorphTarget {
                    dpos: dp.map(|x| x.collect()).unwrap_or_else(|| vec![[0.0; 3]; n]),
                    dnormal: dn.map(|x| x.collect()).unwrap_or_else(|| vec![[0.0; 3]; n]),
                });
            }
            for mapping in prim.mappings() {
                if let Some(mi) = mapping.material().index() {
                    for vi in mapping.variants() {
                        if let Some(name) = variant_names.get(*vi as usize) {
                            p.variants.push((name.clone(), mi));
                        }
                    }
                }
            }
            finish_primitive(&mut p, had_normals);
            m.primitives.push(p);
            list.push(m.primitives.len() - 1);
        }
        mesh_prims.push(list);
    }
    for node in g.nodes() {
        let (t, r, s) = node.transform().decomposed();
        m.nodes.push(Node {
            name: node.name().unwrap_or("").to_string(),
            parent: None,
            local: Trs { t: Vec3::from(t), r: Quat::from_array(r), s: Vec3::from(s) },
            primitives: node.mesh().map(|me| mesh_prims[me.index()].clone()).unwrap_or_default(),
            skin: node.skin().map(|s| s.index()),
            weights: node
                .mesh()
                .and_then(|me| me.weights().map(|w| w.to_vec()))
                .or_else(|| node.weights().map(|w| w.to_vec()))
                .unwrap_or_default(),
        });
    }
    for node in g.nodes() {
        for c in node.children() {
            m.nodes[c.index()].parent = Some(node.index());
        }
    }
    for skin in g.skins() {
        let r = skin.reader(|b| buffers.get(b.index()).map(|v| v.as_slice()));
        let joints: Vec<usize> = skin.joints().map(|j| j.index()).collect();
        let inverse_bind = match r.read_inverse_bind_matrices() {
            Some(it) => it.map(|c| Mat4::from_cols_array_2d(&c)).collect(),
            None => vec![Mat4::IDENTITY; joints.len()],
        };
        m.skins.push(Skin { joints, inverse_bind });
    }
    for anim in g.animations() {
        let mut a = Animation { name: anim.name().unwrap_or("").to_string(), ..Default::default() };
        for ch in anim.channels() {
            let r = ch.reader(|b| buffers.get(b.index()).map(|v| v.as_slice()));
            let Some(times) = r.read_inputs() else { continue };
            let times: Vec<f32> = times.collect();
            let (path, values): (AnimPath, Vec<f32>) = match r.read_outputs() {
                Some(gltf::animation::util::ReadOutputs::Translations(it)) => {
                    (AnimPath::Translation, it.flatten().collect())
                }
                Some(gltf::animation::util::ReadOutputs::Scales(it)) => (AnimPath::Scale, it.flatten().collect()),
                Some(gltf::animation::util::ReadOutputs::Rotations(it)) => {
                    (AnimPath::Rotation, it.into_f32().flatten().collect())
                }
                Some(gltf::animation::util::ReadOutputs::MorphTargetWeights(it)) => {
                    (AnimPath::Weights, it.into_f32().collect())
                }
                None => continue,
            };
            let interp = match ch.sampler().interpolation() {
                gltf::animation::Interpolation::Step => Interp::Step,
                gltf::animation::Interpolation::Linear => Interp::Linear,
                gltf::animation::Interpolation::CubicSpline => Interp::CubicSpline,
            };
            a.duration = a.duration.max(times.last().copied().unwrap_or(0.0));
            a.channels.push(Channel { node: ch.target().node().index(), path, interp, times, values });
        }
        m.animations.push(a);
    }
    Ok(m)
}

// ---------------------------------------------------------------- OBJ

/// Loads Wavefront OBJ with its MTL materials (Y-up, metres).
pub fn obj(path: &Path) -> Result<Model, String> {
    let opts = tobj::LoadOptions { triangulate: true, single_index: true, ignore_points: true, ignore_lines: true };
    let (models, mats) = tobj::load_obj(path, &opts).map_err(|e| e.to_string())?;
    let mut m = Model { basis: Y_UP_METRES, ..Default::default() };
    let base = path.parent().unwrap_or(Path::new("."));
    match mats {
        Ok(mats) => {
            for mt in mats {
                let d = mt.diffuse.unwrap_or([0.8; 3]);
                let ns = mt.shininess.unwrap_or(10.0);
                let mut maps = crate::MaterialMaps::default();
                for (slot, file, srgb) in [(0, &mt.diffuse_texture, true), (1, &mt.normal_texture, false)] {
                    if let Some(f) = file {
                        match std::fs::read(base.join(f))
                            .map_err(|e| e.to_string())
                            .and_then(|b| decode_image(&b, srgb))
                        {
                            Ok(t) => {
                                m.textures.push(t);
                                let i = Some(m.textures.len() - 1);
                                if slot == 0 {
                                    maps.base_color = i;
                                } else {
                                    maps.normal = i;
                                }
                            }
                            Err(e) => m.warnings.push(format!("{f}: {e}")),
                        }
                    }
                }
                let params = crate::MaterialParams {
                    base_color: [d[0], d[1], d[2], mt.dissolve.unwrap_or(1.0)],
                    roughness: (2.0 / (ns + 2.0)).sqrt().clamp(0.02, 1.0),
                    alpha_mode: if mt.dissolve.unwrap_or(1.0) < 1.0 {
                        crate::AlphaMode::Blend
                    } else {
                        crate::AlphaMode::Opaque
                    },
                    ..Default::default()
                };
                m.materials.push(ImportedMaterial { name: mt.name, params, maps });
            }
        }
        Err(e) => m.warnings.push(format!("materials: {e}")),
    }
    for model in models {
        let mesh = model.mesh;
        let n = mesh.positions.len() / 3;
        let mut vs: Vec<Vertex> = (0..n)
            .map(|i| Vertex {
                pos: [mesh.positions[i * 3], mesh.positions[i * 3 + 1], mesh.positions[i * 3 + 2]],
                ..Default::default()
            })
            .collect();
        let had_normals = mesh.normals.len() == n * 3;
        if had_normals {
            for (i, v) in vs.iter_mut().enumerate() {
                v.normal = [mesh.normals[i * 3], mesh.normals[i * 3 + 1], mesh.normals[i * 3 + 2]];
            }
        }
        if mesh.texcoords.len() == n * 2 {
            for (i, v) in vs.iter_mut().enumerate() {
                v.uv = [mesh.texcoords[i * 2], 1.0 - mesh.texcoords[i * 2 + 1]];
            }
        }
        let mut p = Primitive { vertices: vs, indices: mesh.indices, material: mesh.material_id, ..Default::default() };
        finish_primitive(&mut p, had_normals);
        m.primitives.push(p);
        m.nodes.push(Node { name: model.name, primitives: vec![m.primitives.len() - 1], ..Default::default() });
    }
    Ok(m)
}

// ---------------------------------------------------------------- PLY and splats

#[derive(Clone, Copy)]
enum Ty {
    I8,
    U8,
    I16,
    U16,
    I32,
    U32,
    F32,
    F64,
}

impl Ty {
    fn parse(s: &str) -> Option<Ty> {
        Some(match s {
            "char" | "int8" => Ty::I8,
            "uchar" | "uint8" => Ty::U8,
            "short" | "int16" => Ty::I16,
            "ushort" | "uint16" => Ty::U16,
            "int" | "int32" => Ty::I32,
            "uint" | "uint32" => Ty::U32,
            "float" | "float32" => Ty::F32,
            "double" | "float64" => Ty::F64,
            _ => return None,
        })
    }
    fn size(self) -> usize {
        match self {
            Ty::I8 | Ty::U8 => 1,
            Ty::I16 | Ty::U16 => 2,
            Ty::I32 | Ty::U32 | Ty::F32 => 4,
            Ty::F64 => 8,
        }
    }
}

struct Prop {
    name: String,
    ty: Ty,
    /// List count type for list properties.
    list: Option<Ty>,
}

struct Element {
    name: String,
    count: usize,
    props: Vec<Prop>,
}

struct Reader<'a> {
    data: &'a [u8],
    pos: usize,
    big: bool,
    ascii: Option<std::iter::Peekable<std::str::SplitAsciiWhitespace<'a>>>,
}

impl Reader<'_> {
    fn read(&mut self, ty: Ty) -> Result<f64, String> {
        if let Some(it) = self.ascii.as_mut() {
            return it.next().ok_or("unexpected end of data")?.parse::<f64>().map_err(|e| e.to_string());
        }
        let n = ty.size();
        let b = self.data.get(self.pos..self.pos + n).ok_or("unexpected end of data")?;
        self.pos += n;
        let mut a = [0u8; 8];
        a[..n].copy_from_slice(b);
        if self.big {
            a[..n].reverse();
        }
        Ok(match ty {
            Ty::I8 => a[0] as i8 as f64,
            Ty::U8 => a[0] as f64,
            Ty::I16 => i16::from_le_bytes([a[0], a[1]]) as f64,
            Ty::U16 => u16::from_le_bytes([a[0], a[1]]) as f64,
            Ty::I32 => i32::from_le_bytes([a[0], a[1], a[2], a[3]]) as f64,
            Ty::U32 => u32::from_le_bytes([a[0], a[1], a[2], a[3]]) as f64,
            Ty::F32 => f32::from_le_bytes([a[0], a[1], a[2], a[3]]) as f64,
            Ty::F64 => f64::from_le_bytes(a),
        })
    }
}

/// Loads PLY: a triangle mesh, or 3D Gaussian Splatting splats when the vertices carry `f_dc_*` and `scale_*`.
pub fn ply(data: &[u8]) -> Result<Asset, String> {
    let end = data.windows(10).position(|w| w == b"end_header").ok_or("no end_header")?;
    let header = std::str::from_utf8(&data[..end]).map_err(|_| "header is not text")?;
    let mut body = end + 10;
    while body < data.len() && (data[body] == b'\r' || data[body] == b'\n') {
        body += 1;
        if data[body - 1] == b'\n' {
            break;
        }
    }
    let mut format = "";
    let mut elements: Vec<Element> = Vec::new();
    for line in header.lines() {
        let t: Vec<&str> = line.split_whitespace().collect();
        match t.as_slice() {
            ["ply"] | [] => {}
            ["format", f, ..] => format = f,
            ["element", name, count] => elements.push(Element {
                name: name.to_string(),
                count: count.parse().map_err(|_| "bad element count")?,
                props: Vec::new(),
            }),
            ["property", "list", ct, ty, name] => {
                elements.last_mut().ok_or("property before element")?.props.push(Prop {
                    name: name.to_string(),
                    ty: Ty::parse(ty).ok_or("bad type")?,
                    list: Some(Ty::parse(ct).ok_or("bad type")?),
                })
            }
            ["property", ty, name] => elements.last_mut().ok_or("property before element")?.props.push(Prop {
                name: name.to_string(),
                ty: Ty::parse(ty).ok_or("bad type")?,
                list: None,
            }),
            _ => {}
        }
    }
    let mut rd = Reader {
        data,
        pos: body,
        big: format == "binary_big_endian",
        ascii: if format == "ascii" {
            Some(
                std::str::from_utf8(&data[body..])
                    .map_err(|_| "ascii body is not text")?
                    .split_ascii_whitespace()
                    .peekable(),
            )
        } else {
            None
        },
    };
    if !matches!(format, "ascii" | "binary_little_endian" | "binary_big_endian") {
        return Err(format!("unsupported PLY format {format}"));
    }
    let mut verts: Vec<Vec<f64>> = Vec::new();
    let mut vnames: Vec<String> = Vec::new();
    let mut faces: Vec<Vec<u32>> = Vec::new();
    for el in &elements {
        let is_v = el.name == "vertex";
        if is_v {
            vnames = el.props.iter().map(|p| p.name.clone()).collect();
        }
        for _ in 0..el.count {
            let mut row = Vec::with_capacity(el.props.len());
            for p in &el.props {
                match p.list {
                    Some(ct) => {
                        let n = rd.read(ct)? as usize;
                        let mut list = Vec::with_capacity(n);
                        for _ in 0..n {
                            list.push(rd.read(p.ty)? as u32);
                        }
                        if el.name == "face" && (p.name == "vertex_indices" || p.name == "vertex_index") {
                            faces.push(list);
                        }
                        row.push(0.0);
                    }
                    None => row.push(rd.read(p.ty)?),
                }
            }
            if is_v {
                verts.push(row);
            }
        }
    }
    let col = |n: &str| vnames.iter().position(|x| x == n);
    let (x, y, z) = (
        col("x").ok_or("vertices without x")?,
        col("y").ok_or("vertices without y")?,
        col("z").ok_or("vertices without z")?,
    );
    if let (Some(dc0), Some(s0)) = (col("f_dc_0"), col("scale_0")) {
        const C0: f64 = 0.282_094_791_773_878_14;
        let op = col("opacity");
        let r0 = col("rot_0").ok_or("splats without rot_0")?;
        let mut s = Splats { basis: Mat4::from_scale(Vec3::splat(100.0)), ..Default::default() };
        // higher-order SH: f_rest_* holds (degree + 1)² − 1 coefficients per channel, channel-major
        let rest = (0..).take_while(|k| col(&format!("f_rest_{k}")).is_some()).count();
        let per = rest / 3;
        let degree = match per {
            3 => 1,
            8 => 2,
            15 => 3,
            _ => 0,
        };
        let rest0 = col("f_rest_0");
        s.sh_degree = degree;
        for v in &verts {
            if degree > 0 {
                let mut c = [0.0f32; 48];
                for ch in 0..3 {
                    c[ch] = v[dc0 + ch] as f32;
                    for j in 0..per {
                        c[(j + 1) * 3 + ch] = v[rest0.unwrap_or(0) + ch * per + j] as f32;
                    }
                }
                s.sh.push(c);
            }
            s.pos.push([v[x] as f32, v[y] as f32, v[z] as f32]);
            s.scale.push([0, 1, 2].map(|k| v[s0 + k].exp() as f32));
            let q = Quat::from_xyzw(v[r0 + 1] as f32, v[r0 + 2] as f32, v[r0 + 3] as f32, v[r0] as f32).normalize();
            s.rot.push(q.to_array());
            let c = [0, 1, 2].map(|k| srgb_to_linear((0.5 + C0 * v[dc0 + k]).clamp(0.0, 1.0) as f32));
            let a = op.map(|o| 1.0 / (1.0 + (-v[o]).exp())).unwrap_or(1.0) as f32;
            s.color.push([c[0], c[1], c[2], a]);
        }
        return Ok(Asset::Splats(s));
    }
    let (nx, u, vv) =
        (col("nx"), col("u").or(col("s")).or(col("texture_u")), col("v").or(col("t")).or(col("texture_v")));
    let mut vs: Vec<Vertex> = verts
        .iter()
        .map(|r| Vertex {
            pos: [r[x] as f32, r[y] as f32, r[z] as f32],
            normal: nx.map(|i| [r[i] as f32, r[i + 1] as f32, r[i + 2] as f32]).unwrap_or_default(),
            uv: match (u, vv) {
                (Some(a), Some(b)) => [r[a] as f32, 1.0 - r[b] as f32],
                _ => [0.0; 2],
            },
            tangent: [1.0, 0.0, 0.0, 1.0],
        })
        .collect();
    let mut indices = Vec::new();
    for f in &faces {
        for k in 1..f.len().saturating_sub(1) {
            indices.extend_from_slice(&[f[0], f[k], f[k + 1]]);
        }
    }
    if indices.iter().any(|&i| i as usize >= vs.len()) {
        return Err("face index out of range".into());
    }
    let mut m = Model { basis: Y_UP_METRES, ..Default::default() };
    if col("red").is_some() {
        m.warnings.push("PLY vertex colours are ignored".into());
    }
    if indices.is_empty() {
        return Err("PLY has no faces and no Gaussian splat properties".into());
    }
    let had = nx.is_some();
    if !had {
        compute_normals(&mut vs, &indices);
    }
    compute_tangents(&mut vs, &indices);
    m.primitives.push(Primitive { vertices: vs, indices, ..Default::default() });
    m.nodes.push(Node { name: "ply".into(), primitives: vec![0], ..Default::default() });
    Ok(Asset::Model(m))
}

/// Loads the 32-byte-per-splat `.splat` format (position, scale, sRGBA, quaternion).
pub fn splat(data: &[u8]) -> Result<Splats, String> {
    if data.len() % 32 != 0 {
        return Err(format!("size {} is not a multiple of 32 bytes", data.len()));
    }
    let mut s = Splats { basis: Mat4::from_scale(Vec3::splat(100.0)), ..Default::default() };
    for c in data.chunks_exact(32) {
        let f = |i: usize| f32::from_le_bytes([c[i], c[i + 1], c[i + 2], c[i + 3]]);
        s.pos.push([f(0), f(4), f(8)]);
        s.scale.push([f(12), f(16), f(20)]);
        s.color.push([
            srgb_to_linear(c[24] as f32 / 255.0),
            srgb_to_linear(c[25] as f32 / 255.0),
            srgb_to_linear(c[26] as f32 / 255.0),
            c[27] as f32 / 255.0,
        ]);
        let q = |b: u8| (b as f32 - 128.0) / 128.0;
        s.rot.push(Quat::from_xyzw(q(c[29]), q(c[30]), q(c[31]), q(c[28])).normalize().to_array());
    }
    Ok(s)
}

// ---------------------------------------------------------------- FBX

/// ufbx's row-major 3×4 matrix as a glam matrix.
fn fbx_mat(w: &ufbx::Matrix) -> Mat4 {
    Mat4::from_cols_array(&[
        w.m00 as f32,
        w.m10 as f32,
        w.m20 as f32,
        0.0,
        w.m01 as f32,
        w.m11 as f32,
        w.m21 as f32,
        0.0,
        w.m02 as f32,
        w.m12 as f32,
        w.m22 as f32,
        0.0,
        w.m03 as f32,
        w.m13 as f32,
        w.m23 as f32,
        1.0,
    ])
}

fn fbx_trs(t: &ufbx::Transform) -> Trs {
    let (v, q, s) = (t.translation, t.rotation, t.scale);
    Trs {
        t: Vec3::new(v.x as f32, v.y as f32, v.z as f32),
        r: glam::Quat::from_xyzw(q.x as f32, q.y as f32, q.z as f32, q.w as f32).normalize(),
        s: Vec3::new(s.x as f32, s.y as f32, s.z as f32),
    }
}

/// Loads FBX through ufbx, converted to Y-up metres: the node hierarchy, meshes with their
/// materials, skins (clusters become joints, `geometry_to_bone` their inverse bind), blend shapes
/// (morph targets) and every animation stack, baked by ufbx into linear keys (pivots, pre- and
/// post-rotations and rotation orders resolved) with blend-channel weights as morph weights.
pub fn fbx(path: &Path) -> Result<Model, String> {
    let opts = ufbx::LoadOpts {
        target_axes: ufbx::CoordinateAxes {
            right: ufbx::CoordinateAxis::PositiveX,
            up: ufbx::CoordinateAxis::PositiveY,
            front: ufbx::CoordinateAxis::PositiveZ,
        },
        target_unit_meters: 1.0,
        space_conversion: ufbx::SpaceConversion::ModifyGeometry,
        generate_missing_normals: true,
        ..Default::default()
    };
    let scene =
        ufbx::load_file(path.to_str().ok_or("path is not UTF-8")?, opts).map_err(|e| format!("{:?}", e.description))?;
    let mut m = Model { basis: Y_UP_METRES, ..Default::default() };
    let mut mat_index: std::collections::HashMap<u32, usize> = Default::default();
    for mat in scene.materials.iter() {
        let c = mat.pbr.base_color.value_vec4;
        let e = mat.pbr.emission_color.value_vec4;
        let params = crate::MaterialParams {
            base_color: [c.x as f32, c.y as f32, c.z as f32, c.w as f32].map(|v| if v.is_finite() { v } else { 1.0 }),
            metallic: mat.pbr.metalness.value_vec4.x as f32,
            roughness: (mat.pbr.roughness.value_vec4.x as f32).clamp(0.0, 1.0),
            emissive: [e.x as f32, e.y as f32, e.z as f32],
            emissive_strength: (mat.pbr.emission_factor.value_vec4.x as f32).max(0.0),
            ..Default::default()
        };
        mat_index.insert(mat.element.typed_id, m.materials.len());
        m.materials.push(ImportedMaterial { name: mat.element.name.to_string(), params, maps: Default::default() });
    }
    // every node, parents first (ufbx lists them in depth order), so the hierarchy animates
    let node_of: std::collections::HashMap<u32, usize> =
        scene.nodes.iter().enumerate().map(|(i, n)| (n.element.typed_id, i)).collect();
    for node in scene.nodes.iter() {
        m.nodes.push(Node {
            name: node.element.name.to_string(),
            parent: node.parent.as_ref().and_then(|p| node_of.get(&p.element.typed_id).copied()),
            local: fbx_trs(&node.local_transform),
            ..Default::default()
        });
    }
    // blend channels by element id: (node, morph index)
    let mut channel_of: std::collections::HashMap<u32, (usize, usize)> = Default::default();
    for (ni, node) in scene.nodes.iter().enumerate() {
        let Some(mesh) = node.mesh.as_ref() else { continue };
        let skin = mesh.skin_deformers.first();
        if mesh.skin_deformers.len() > 1 {
            m.warnings.push(format!(
                "{}: only the first of {} skins is used",
                node.element.name,
                mesh.skin_deformers.len()
            ));
        }
        // skinned vertices stay in geometry space (the clusters bind from there); others move into node space
        let geo = if skin.is_some() { Mat4::IDENTITY } else { fbx_mat(&node.geometry_to_node) };
        let geo_n = Mat3::from_mat4(geo).inverse().transpose();
        // morph targets: one per blend channel, offsets by logical vertex
        let mut morphs: Vec<std::collections::HashMap<u32, (Vec3, Vec3)>> = Vec::new();
        let mut weights = Vec::new();
        for bd in mesh.blend_deformers.iter() {
            for ch in bd.channels.iter() {
                let Some(shape) = ch.target_shape.as_ref() else { continue };
                if ch.keyframes.len() > 1 {
                    m.warnings.push(format!(
                        "{}: in-between blend shapes of {} use the full shape only",
                        node.element.name, ch.element.name
                    ));
                }
                let mut offs = std::collections::HashMap::new();
                for (k, &vi) in shape.offset_vertices.iter().enumerate() {
                    let d = shape.position_offsets[k];
                    let n = shape.normal_offsets.get(k).map(|n| Vec3::new(n.x as f32, n.y as f32, n.z as f32));
                    offs.insert(vi, (Vec3::new(d.x as f32, d.y as f32, d.z as f32), n.unwrap_or(Vec3::ZERO)));
                }
                channel_of.insert(ch.element.element_id, (ni, morphs.len()));
                morphs.push(offs);
                weights.push(ch.weight as f32);
            }
        }
        let mut tri = vec![0u32; mesh.max_face_triangles * 3];
        // one primitive per material slot
        let slots = mesh.materials.len().max(1);
        let mut prims: Vec<Primitive> = (0..slots).map(|_| Primitive::default()).collect();
        for (fi, face) in mesh.faces.iter().enumerate() {
            let slot = mesh.face_material.get(fi).copied().unwrap_or(0) as usize % slots;
            let n = mesh.triangulate_face(&mut tri, *face) as usize;
            let p = &mut prims[slot];
            for &ix in &tri[..n * 3] {
                let ix = ix as usize;
                let vi = mesh.vertex_indices[ix];
                let pos = mesh.vertex_position[ix];
                let nrm = if mesh.vertex_normal.exists {
                    mesh.vertex_normal[ix]
                } else {
                    ufbx::Vec3 { x: 0.0, y: 0.0, z: 0.0 }
                };
                let uv = if mesh.vertex_uv.exists { mesh.vertex_uv[ix] } else { ufbx::Vec2 { x: 0.0, y: 0.0 } };
                let pos = geo.transform_point3(Vec3::new(pos.x as f32, pos.y as f32, pos.z as f32));
                let nrm = geo_n * Vec3::new(nrm.x as f32, nrm.y as f32, nrm.z as f32);
                p.indices.push(p.vertices.len() as u32);
                p.vertices.push(Vertex {
                    pos: pos.into(),
                    normal: nrm.into(),
                    uv: [uv.x as f32, 1.0 - uv.y as f32],
                    tangent: [1.0, 0.0, 0.0, 1.0],
                });
                if p.morphs.len() < morphs.len() {
                    p.morphs.resize_with(morphs.len(), Default::default);
                }
                for (k, offs) in morphs.iter().enumerate() {
                    let (dp, dn) = offs.get(&vi).copied().unwrap_or((Vec3::ZERO, Vec3::ZERO));
                    p.morphs[k].dpos.push(geo.transform_vector3(dp).into());
                    p.morphs[k].dnormal.push((geo_n * dn).into());
                }
                if let Some(sk) = skin {
                    let sv = &sk.vertices[vi as usize];
                    let (mut js, mut ws) = ([0u16; 4], [0f32; 4]);
                    // ufbx sorts each vertex's weights by decreasing weight: keep the four largest
                    for k in 0..(sv.num_weights as usize).min(4) {
                        let w = &sk.weights[sv.weight_begin as usize + k];
                        js[k] = w.cluster_index as u16;
                        ws[k] = w.weight as f32;
                    }
                    p.joints.push(js);
                    p.weights.push(ws);
                }
            }
        }
        let mut list = Vec::new();
        for (slot, mut p) in prims.into_iter().enumerate() {
            if p.indices.is_empty() {
                continue;
            }
            p.material = mesh.materials.get(slot).and_then(|mt| mat_index.get(&mt.element.typed_id).copied());
            finish_primitive(&mut p, mesh.vertex_normal.exists);
            m.primitives.push(p);
            list.push(m.primitives.len() - 1);
        }
        if let Some(sk) = skin {
            m.skins.push(Skin {
                joints: sk
                    .clusters
                    .iter()
                    .map(|c| c.bone_node.as_ref().and_then(|b| node_of.get(&b.element.typed_id).copied()).unwrap_or(ni))
                    .collect(),
                inverse_bind: sk.clusters.iter().map(|c| fbx_mat(&c.geometry_to_bone)).collect(),
            });
            m.nodes[ni].skin = Some(m.skins.len() - 1);
        }
        m.nodes[ni].primitives = list;
        m.nodes[ni].weights = weights;
    }
    for stack in scene.anim_stacks.iter() {
        let opts = ufbx::BakeOpts { trim_start_time: true, ..Default::default() };
        let baked = match ufbx::bake_anim(&scene, &stack.anim, opts) {
            Ok(b) => b,
            Err(e) => {
                m.warnings.push(format!("animation {}: {:?}", stack.element.name, e.description));
                continue;
            }
        };
        let mut channels = Vec::new();
        for bn in baked.nodes.iter() {
            let Some(&node) = node_of.get(&bn.typed_id) else { continue };
            let t3 = |keys: &ufbx::List<ufbx::BakedVec3>, path| Channel {
                node,
                path,
                interp: Interp::Linear,
                times: keys.iter().map(|k| k.time as f32).collect(),
                values: keys.iter().flat_map(|k| [k.value.x as f32, k.value.y as f32, k.value.z as f32]).collect(),
            };
            channels.push(t3(&bn.translation_keys, AnimPath::Translation));
            channels.push(t3(&bn.scale_keys, AnimPath::Scale));
            channels.push(Channel {
                node,
                path: AnimPath::Rotation,
                interp: Interp::Linear,
                times: bn.rotation_keys.iter().map(|k| k.time as f32).collect(),
                values: bn
                    .rotation_keys
                    .iter()
                    .flat_map(|k| [k.value.x as f32, k.value.y as f32, k.value.z as f32, k.value.w as f32])
                    .collect(),
            });
        }
        // blend-channel weights (DeformPercent, 0–100): one weights channel per mesh node on the
        // union of the key times, each morph interpolated linearly
        let mut per_node: std::collections::BTreeMap<usize, Vec<MorphKeys>> = Default::default();
        for el in baked.elements.iter() {
            let Some(&(node, k)) = channel_of.get(&el.element_id) else { continue };
            for prop in el.props.iter() {
                if &*prop.name == "DeformPercent" {
                    let keys = prop.keys.iter().map(|x| (x.time as f32, x.value.x as f32 / 100.0)).collect();
                    per_node.entry(node).or_default().push((k, keys));
                }
            }
        }
        for (node, morphs) in per_node {
            let mut times: Vec<f32> = morphs.iter().flat_map(|(_, keys)| keys.iter().map(|k| k.0)).collect();
            times.sort_by(f32::total_cmp);
            times.dedup();
            let defaults = m.nodes[node].weights.clone();
            let mut values = Vec::with_capacity(times.len() * defaults.len());
            for &t in &times {
                let mut w = defaults.clone();
                for (k, keys) in &morphs {
                    if let Some(slot) = w.get_mut(*k) {
                        *slot = lerp_keys(keys, t);
                    }
                }
                values.extend(w);
            }
            channels.push(Channel { node, path: AnimPath::Weights, interp: Interp::Linear, times, values });
        }
        channels.retain(|c| !c.times.is_empty());
        let duration = if baked.playback_duration > 0.0 {
            baked.playback_duration
        } else {
            baked.key_time_max - baked.key_time_min.min(0.0)
        };
        m.animations.push(Animation { name: stack.element.name.to_string(), channels, duration: duration as f32 });
    }
    Ok(m)
}

/// A morph index with its (time, weight) keys.
type MorphKeys = (usize, Vec<(f32, f32)>);

/// Linear interpolation of (time, value) keys, held beyond the ends.
fn lerp_keys(keys: &[(f32, f32)], t: f32) -> f32 {
    match keys.iter().position(|k| k.0 > t) {
        None => keys.last().map_or(0.0, |k| k.1),
        Some(0) => keys[0].1,
        Some(i) => {
            let (a, b) = (keys[i - 1], keys[i]);
            a.1 + (b.1 - a.1) * (t - a.0) / (b.0 - a.0).max(1e-9)
        }
    }
}

// ---------------------------------------------------------------- USD

#[derive(Clone, Debug, PartialEq)]
enum UV {
    Num(f64),
    Str(String),
    Path(String),
    Ident(String),
    List(Vec<UV>),
    /// `{ time: value, … }` of a `.timeSamples` attribute.
    Samples(Vec<(f64, UV)>),
}

impl UV {
    fn nums(&self) -> Vec<f64> {
        match self {
            UV::Num(n) => vec![*n],
            UV::List(l) => l.iter().flat_map(|v| v.nums()).collect(),
            _ => Vec::new(),
        }
    }
    fn text(&self) -> Option<&str> {
        match self {
            UV::Str(s) | UV::Ident(s) | UV::Path(s) => Some(s),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, Default)]
struct Prim {
    kind: String,
    path: String,
    attrs: Vec<(String, UV)>,
    children: Vec<Prim>,
}

impl Prim {
    fn get(&self, name: &str) -> Option<&UV> {
        self.attrs.iter().rev().find(|(n, _)| n == name).map(|(_, v)| v)
    }
}

struct Usda<'a> {
    s: &'a [u8],
    i: usize,
    /// Time-sampled attributes seen.
    sampled: usize,
}

impl Usda<'_> {
    fn ws(&mut self) {
        while self.i < self.s.len() {
            let c = self.s[self.i];
            if c.is_ascii_whitespace() {
                self.i += 1;
            } else if c == b'#' {
                while self.i < self.s.len() && self.s[self.i] != b'\n' {
                    self.i += 1;
                }
            } else {
                break;
            }
        }
    }
    fn peek(&mut self) -> Option<u8> {
        self.ws();
        self.s.get(self.i).copied()
    }
    fn word(&mut self) -> String {
        self.ws();
        let st = self.i;
        while self.i < self.s.len() {
            let c = self.s[self.i];
            if c.is_ascii_alphanumeric() || b"_:.-+".contains(&c) {
                self.i += 1;
            } else if c == b'[' && self.s.get(self.i + 1) == Some(&b']') {
                // array type suffix: int[], point3f[]
                self.i += 2;
            } else {
                break;
            }
        }
        String::from_utf8_lossy(&self.s[st..self.i]).into_owned()
    }
    fn string(&mut self) -> String {
        let q = self.s[self.i];
        let triple = self.s.get(self.i..self.i + 3) == Some(&[q, q, q][..]);
        self.i += if triple { 3 } else { 1 };
        let st = self.i;
        loop {
            if self.i >= self.s.len() {
                break;
            }
            if triple && self.s.get(self.i..self.i + 3) == Some(&[q, q, q][..]) {
                let out = String::from_utf8_lossy(&self.s[st..self.i]).into_owned();
                self.i += 3;
                return out;
            }
            if !triple && self.s[self.i] == q {
                let out = String::from_utf8_lossy(&self.s[st..self.i]).into_owned();
                self.i += 1;
                return out;
            }
            if self.s[self.i] == b'\\' {
                self.i += 1;
            }
            self.i += 1;
        }
        String::from_utf8_lossy(&self.s[st..]).into_owned()
    }
    fn skip_group(&mut self, open: u8, close: u8) {
        let mut depth = 0;
        while self.i < self.s.len() {
            match self.s[self.i] {
                b'"' | b'\'' => {
                    self.string();
                    continue;
                }
                c if c == open => depth += 1,
                c if c == close => {
                    depth -= 1;
                    if depth == 0 {
                        self.i += 1;
                        return;
                    }
                }
                _ => {}
            }
            self.i += 1;
        }
    }
    fn value(&mut self) -> UV {
        match self.peek() {
            Some(b'(') | Some(b'[') => {
                let close = if self.s[self.i] == b'(' { b')' } else { b']' };
                self.i += 1;
                let mut items = Vec::new();
                loop {
                    match self.peek() {
                        Some(c) if c == close => {
                            self.i += 1;
                            break;
                        }
                        Some(b',') => self.i += 1,
                        None => break,
                        _ => items.push(self.value()),
                    }
                }
                UV::List(items)
            }
            Some(b'{') => {
                self.i += 1;
                let mut samples = Vec::new();
                loop {
                    match self.peek() {
                        Some(b'}') => {
                            self.i += 1;
                            break;
                        }
                        Some(b',') => self.i += 1,
                        None => break,
                        _ => {
                            let before = self.i;
                            let t = self.word().trim_end_matches(':').parse::<f64>().unwrap_or(0.0);
                            if self.peek() == Some(b':') {
                                self.i += 1;
                            }
                            samples.push((t, self.value()));
                            if self.i == before {
                                self.i += 1;
                            }
                        }
                    }
                }
                UV::Samples(samples)
            }
            Some(b'"') | Some(b'\'') => UV::Str(self.string()),
            Some(b'<') => {
                let st = self.i + 1;
                while self.i < self.s.len() && self.s[self.i] != b'>' {
                    self.i += 1;
                }
                self.i += 1;
                UV::Path(String::from_utf8_lossy(&self.s[st..self.i - 1]).into_owned())
            }
            Some(b'@') => {
                self.i += 1;
                let st = self.i;
                while self.i < self.s.len() && self.s[self.i] != b'@' {
                    self.i += 1;
                }
                self.i += 1;
                UV::Str(String::from_utf8_lossy(&self.s[st..self.i - 1]).into_owned())
            }
            Some(_) => {
                let w = self.word();
                if w.is_empty() {
                    self.i += 1;
                    return UV::Ident(String::new());
                }
                w.parse::<f64>().map(UV::Num).unwrap_or(UV::Ident(w))
            }
            None => UV::Ident(String::new()),
        }
    }
    /// Parses the body of a prim (or the stage) until `}` or the end.
    fn body(&mut self, parent: &str, out: &mut Prim) {
        loop {
            match self.peek() {
                None => return,
                Some(b'}') => {
                    self.i += 1;
                    return;
                }
                Some(b'(') => self.skip_group(b'(', b')'),
                Some(b'"') | Some(b'\'') => {
                    self.string();
                }
                _ => {
                    let w = self.word();
                    if w.is_empty() {
                        self.i += 1;
                        continue;
                    }
                    if matches!(w.as_str(), "def" | "over" | "class") {
                        let mut kind = String::new();
                        if self.peek() != Some(b'"') {
                            kind = self.word();
                        }
                        let name = if matches!(self.peek(), Some(b'"')) { self.string() } else { String::new() };
                        let path = format!("{parent}/{name}");
                        if self.peek() == Some(b'(') {
                            self.skip_group(b'(', b')');
                        }
                        let mut p = Prim { kind, path: path.clone(), ..Default::default() };
                        if self.peek() == Some(b'{') {
                            self.i += 1;
                            self.body(&path, &mut p);
                        }
                        out.children.push(p);
                        continue;
                    }
                    // attribute or relationship: [custom|uniform|rel] type[] name(.connect) (= value)? (metadata)?
                    let mut toks = vec![w];
                    while toks.len() < 4 {
                        match self.peek() {
                            Some(b'=') | Some(b'(') | Some(b'}') | None => break,
                            Some(b'\n') => break,
                            _ => {
                                let before = self.i;
                                let t = self.word();
                                if t.is_empty() {
                                    self.i = before + 1;
                                    break;
                                }
                                toks.push(t);
                                // a declaration without value ends at the line end
                                let mut j = self.i;
                                while j < self.s.len() && self.s[j] == b' ' {
                                    j += 1;
                                }
                                if j < self.s.len() && self.s[j] == b'\n' {
                                    break;
                                }
                            }
                        }
                    }
                    let name = toks.last().cloned().unwrap_or_default();
                    if self.peek() == Some(b'=') {
                        self.i += 1;
                        let v = self.value();
                        match (name.strip_suffix(".timeSamples"), v) {
                            // without a default, an attribute takes its first sample
                            (Some(base), UV::Samples(samples)) => {
                                self.sampled += 1;
                                if out.get(base).is_none() {
                                    if let Some((_, first)) = samples.into_iter().next() {
                                        out.attrs.insert(0, (base.to_string(), first));
                                    }
                                }
                            }
                            (_, v) => out.attrs.push((name, v)),
                        }
                    }
                    if self.peek() == Some(b'(') {
                        self.skip_group(b'(', b')');
                    }
                }
            }
        }
    }
}

fn prim_matrix(p: &Prim) -> Mat4 {
    let order: Vec<String> = match p.get("xformOpOrder") {
        Some(UV::List(l)) => l.iter().filter_map(|v| v.text().map(str::to_string)).collect(),
        _ => Vec::new(),
    };
    let mut m = Mat4::IDENTITY;
    for op in order {
        let inverse = op.starts_with("!invert!");
        let op = op.trim_start_matches("!invert!");
        let n = p.get(op).map(|v| v.nums()).unwrap_or_default();
        let v3 = |d: f64| {
            Vec3::new(*n.first().unwrap_or(&d) as f32, *n.get(1).unwrap_or(&d) as f32, *n.get(2).unwrap_or(&d) as f32)
        };
        let base = op.split(':').nth(1).unwrap_or("");
        let t = match base {
            "translate" => Mat4::from_translation(v3(0.0)),
            "scale" => Mat4::from_scale(v3(1.0)),
            "rotateX" => Mat4::from_rotation_x((n.first().copied().unwrap_or(0.0) as f32).to_radians()),
            "rotateY" => Mat4::from_rotation_y((n.first().copied().unwrap_or(0.0) as f32).to_radians()),
            "rotateZ" => Mat4::from_rotation_z((n.first().copied().unwrap_or(0.0) as f32).to_radians()),
            "rotateXYZ" => {
                let r = v3(0.0);
                Mat4::from_rotation_z(r.z.to_radians())
                    * Mat4::from_rotation_y(r.y.to_radians())
                    * Mat4::from_rotation_x(r.x.to_radians())
            }
            "orient" if n.len() >= 4 => {
                Mat4::from_quat(Quat::from_xyzw(n[1] as f32, n[2] as f32, n[3] as f32, n[0] as f32).normalize())
            }
            // USD matrices are row-vector (row-major) — the same memory as a column-major column-vector matrix
            "transform" if n.len() >= 16 => Mat4::from_cols_array(&std::array::from_fn(|i| n[i] as f32)),
            _ => Mat4::IDENTITY,
        };
        m *= if inverse { t.inverse() } else { t };
    }
    m
}

/// A crate layer as the prim tree the USDA parser builds: prims by path with their type, each
/// attribute's default (or, without one, its first time sample) and each relationship's first
/// target; stage metadata gives the up axis and metres per unit.
fn usdc_prims(data: &[u8], warnings: &mut Vec<String>) -> Result<(Prim, bool, f32), String> {
    use crate::usdc::{SpecKind, Val};
    let specs = crate::usdc::read(data)?;
    let uv = |v: &Val| -> Option<UV> {
        Some(match v {
            Val::Bool(b) => UV::Num(*b as u8 as f64),
            Val::Nums(n) if n.len() == 1 => UV::Num(n[0]),
            Val::Nums(n) | Val::Array(n) => UV::List(n.iter().map(|&x| UV::Num(x)).collect()),
            Val::Token(t) => UV::Ident(t.clone()),
            Val::Str(t) | Val::Asset(t) => UV::Str(t.clone()),
            Val::Tokens(t) => UV::List(t.iter().map(|x| UV::Ident(x.clone())).collect()),
            Val::Paths(p) => UV::Path(p.first()?.clone()),
            _ => return None,
        })
    };
    let (mut up_z, mut mpu) = (false, 0.01f32);
    let mut prims: std::collections::HashMap<String, Prim> = Default::default();
    let mut order: std::collections::HashMap<String, Vec<String>> = Default::default();
    let mut sampled = 0usize;
    for sp in &specs {
        match sp.kind {
            SpecKind::PseudoRoot | SpecKind::Prim => {
                if sp.kind == SpecKind::PseudoRoot {
                    up_z = matches!(sp.get("upAxis"), Some(Val::Token(t)) if t == "Z");
                    if let Some(Val::Nums(n)) = sp.get("metersPerUnit") {
                        mpu = n.first().copied().unwrap_or(0.01) as f32;
                    }
                }
                let kind = match sp.get("typeName") {
                    Some(Val::Token(t)) => t.clone(),
                    _ => String::new(),
                };
                let path = if sp.path == "/" { String::new() } else { sp.path.clone() };
                if let Some(Val::Tokens(kids)) = sp.get("primChildren") {
                    order.insert(path.clone(), kids.clone());
                }
                let p = prims.entry(path.clone()).or_default();
                p.kind = kind;
                p.path = path;
            }
            SpecKind::Attribute | SpecKind::Relationship => {
                let Some((owner, name)) = sp.path.rsplit_once('.') else { continue };
                let value = if sp.kind == SpecKind::Relationship {
                    sp.get("targetPaths")
                } else {
                    sp.get("default").or_else(|| match sp.get("timeSamples") {
                        Some(Val::TimeSamples(ts)) => ts.first().map(|(_, v)| {
                            sampled += 1;
                            v
                        }),
                        _ => None,
                    })
                };
                if let Some(v) = value.and_then(uv) {
                    let owner = if owner == "/" { String::new() } else { owner.to_string() };
                    prims.entry(owner).or_default().attrs.push((name.to_string(), v));
                }
            }
            SpecKind::Other => {}
        }
    }
    if sampled > 0 {
        warnings.push(sampled_warning(sampled));
    }
    fn build(
        path: &str,
        prims: &mut std::collections::HashMap<String, Prim>,
        order: &std::collections::HashMap<String, Vec<String>>,
    ) -> Prim {
        let mut p = prims.remove(path).unwrap_or_default();
        for name in order.get(path).cloned().unwrap_or_default() {
            let child = format!("{path}/{name}");
            if prims.contains_key(&child) {
                p.children.push(build(&child, prims, order));
            }
        }
        p
    }
    Ok((build("", &mut prims, &order), up_z, mpu))
}

/// Loads USD: USDA text, binary USDC crates, or USDZ archives holding either (their first
/// layer). The meshes, their transforms and UsdPreviewSurface materials are imported.
pub fn usd(path: &Path) -> Result<Model, String> {
    let data = std::fs::read(path).map_err(|e| e.to_string())?;
    let layer: Vec<u8> = if data.starts_with(b"PK") {
        let entries = sr_vector::zip::entries(&data)?;
        match entries.iter().find(|(n, _)| n.ends_with(".usda") || n.ends_with(".usdc") || n.ends_with(".usd")) {
            Some((_, b)) => b.clone(),
            None => return Err("the USDZ holds no USD layer".into()),
        }
    } else {
        data
    };
    let mut warnings = Vec::new();
    let (root, up_z, mpu) = if layer.starts_with(b"PXR-USDC") {
        usdc_prims(&layer, &mut warnings)?
    } else {
        usda_prims(&layer, &mut warnings)
    };
    usd_model(root, up_z, mpu, warnings)
}

fn sampled_warning(n: usize) -> String {
    format!("{n} time-sampled attribute(s) use their first sample; USD animation is not imported")
}

/// Parses USDA text into its prim tree, up axis and metres per unit.
fn usda_prims(text: &[u8], warnings: &mut Vec<String>) -> (Prim, bool, f32) {
    let mut parser = Usda { s: text, i: 0, sampled: 0 };
    // stage metadata: #usda 1.0 ( upAxis = "Z" metersPerUnit = 0.01 )
    let mut up_z = false;
    let mut mpu = 0.01f32;
    let head = String::from_utf8_lossy(&text[..text.len().min(4096)]).into_owned();
    if let Some(i) = head.find("upAxis") {
        up_z = head[i..].split('"').nth(1) == Some("Z");
    }
    if let Some(i) = head.find("metersPerUnit") {
        mpu = head[i..]
            .split('=')
            .nth(1)
            .and_then(|v| v.split_whitespace().next())
            .and_then(|v| v.trim_end_matches(')').parse().ok())
            .unwrap_or(0.01);
    }
    if text.starts_with(b"#usda") {
        while parser.i < text.len() && text[parser.i] != b'\n' {
            parser.i += 1;
        }
        if parser.peek() == Some(b'(') {
            parser.skip_group(b'(', b')');
        }
    }
    let mut root = Prim::default();
    parser.body("", &mut root);
    if parser.sampled > 0 {
        warnings.push(sampled_warning(parser.sampled));
    }
    (root, up_z, mpu)
}

fn usd_model(root: Prim, up_z: bool, mpu: f32, warnings: Vec<String>) -> Result<Model, String> {
    let mut m = Model { warnings, ..Default::default() };
    let axis = if up_z { Mat4::from_rotation_x(-std::f32::consts::FRAC_PI_2) } else { Mat4::IDENTITY };
    m.basis = Y_UP_METRES * Mat4::from_scale(Vec3::splat(mpu)) * axis;
    // materials: prim path → UsdPreviewSurface inputs
    let mut materials: std::collections::HashMap<String, usize> = Default::default();
    fn walk<'a>(p: &'a Prim, f: &mut dyn FnMut(&'a Prim)) {
        f(p);
        for c in &p.children {
            walk(c, f);
        }
    }
    walk(&root, &mut |p| {
        if p.kind != "Material" {
            return;
        }
        let mut params = crate::MaterialParams::default();
        walk(p, &mut |s| {
            if s.kind == "Shader" && s.get("info:id").and_then(|v| v.text()) == Some("UsdPreviewSurface") {
                let v3 = |n: &str, d: [f32; 3]| {
                    s.get(n)
                        .map(|v| v.nums())
                        .filter(|v| v.len() >= 3)
                        .map(|v| [v[0] as f32, v[1] as f32, v[2] as f32])
                        .unwrap_or(d)
                };
                let f = |n: &str, d: f32| {
                    s.get(n).map(|v| v.nums()).and_then(|v| v.first().copied()).map(|v| v as f32).unwrap_or(d)
                };
                let c = v3("inputs:diffuseColor", [0.18; 3]);
                params.base_color = [c[0], c[1], c[2], f("inputs:opacity", 1.0)];
                params.metallic = f("inputs:metallic", 0.0);
                params.roughness = f("inputs:roughness", 0.5);
                params.emissive = v3("inputs:emissiveColor", [0.0; 3]);
                params.ior = f("inputs:ior", 1.5);
                params.clearcoat = f("inputs:clearcoat", 0.0);
                params.clearcoat_roughness = f("inputs:clearcoatRoughness", 0.01);
                if params.base_color[3] < 1.0 {
                    params.alpha_mode = crate::AlphaMode::Blend;
                }
            }
        });
        materials.insert(p.path.clone(), m.materials.len());
        m.materials.push(ImportedMaterial { name: p.path.clone(), params, maps: Default::default() });
    });
    // meshes with their accumulated transforms
    fn meshes(p: &Prim, parent: Mat4, out: &mut Vec<(Mat4, Prim)>) {
        let local = prim_matrix(p);
        let w = parent * local;
        if p.kind == "Mesh" {
            out.push((w, p.clone()));
        }
        for c in &p.children {
            meshes(c, w, out);
        }
    }
    let mut found = Vec::new();
    meshes(&root, Mat4::IDENTITY, &mut found);
    for (w, p) in found {
        let pts = p.get("points").map(|v| v.nums()).unwrap_or_default();
        let counts: Vec<usize> =
            p.get("faceVertexCounts").map(|v| v.nums()).unwrap_or_default().iter().map(|&c| c as usize).collect();
        let fidx: Vec<u32> =
            p.get("faceVertexIndices").map(|v| v.nums()).unwrap_or_default().iter().map(|&c| c as u32).collect();
        if pts.len() < 9 || counts.is_empty() {
            m.warnings.push(format!("{}: mesh without points or faces", p.path));
            continue;
        }
        let nverts = pts.len() / 3;
        let normals = p.get("normals").or(p.get("primvars:normals")).map(|v| v.nums()).unwrap_or_default();
        let st = p.get("primvars:st").or(p.get("primvars:UVMap")).map(|v| v.nums()).unwrap_or_default();
        let st_idx: Vec<u32> =
            p.get("primvars:st:indices").map(|v| v.nums()).unwrap_or_default().iter().map(|&c| c as u32).collect();
        let face_varying_n = normals.len() / 3 == fidx.len();
        let face_varying_st = if st_idx.len() == fidx.len() { true } else { st.len() / 2 == fidx.len() };
        // emit one vertex per face corner (face-varying attributes are common in USD)
        let mut prim = Primitive::default();
        let mut corner = 0usize;
        for &cnt in &counts {
            let first = prim.vertices.len() as u32;
            for k in 0..cnt {
                let vi = *fidx.get(corner + k).ok_or("faceVertexIndices too short")? as usize;
                if vi >= nverts {
                    return Err(format!("{}: vertex index out of range", p.path));
                }
                let mut vx = Vertex {
                    pos: [pts[vi * 3] as f32, pts[vi * 3 + 1] as f32, pts[vi * 3 + 2] as f32],
                    tangent: [1.0, 0.0, 0.0, 1.0],
                    ..Default::default()
                };
                let ni = if face_varying_n { corner + k } else { vi };
                if normals.len() >= (ni + 1) * 3 {
                    vx.normal = [normals[ni * 3] as f32, normals[ni * 3 + 1] as f32, normals[ni * 3 + 2] as f32];
                }
                let si = if !st_idx.is_empty() {
                    *st_idx.get(if face_varying_st { corner + k } else { vi }).unwrap_or(&0) as usize
                } else if face_varying_st {
                    corner + k
                } else {
                    vi
                };
                if st.len() >= (si + 1) * 2 {
                    vx.uv = [st[si * 2] as f32, 1.0 - st[si * 2 + 1] as f32];
                }
                prim.vertices.push(vx);
            }
            for k in 1..cnt.saturating_sub(1) {
                prim.indices.extend_from_slice(&[first, first + k as u32, first + k as u32 + 1]);
            }
            corner += cnt;
        }
        // USD's default orientation is right-handed (counter-clockwise front faces), like glTF
        if p.get("orientation").and_then(|v| v.text()) == Some("leftHanded") {
            for t in prim.indices.chunks_exact_mut(3) {
                t.swap(1, 2);
            }
        }
        prim.material = p.get("material:binding").and_then(|v| v.text()).and_then(|path| materials.get(path).copied());
        let had = !normals.is_empty();
        finish_primitive(&mut prim, had);
        m.primitives.push(prim);
        let (s, r, t) = w.to_scale_rotation_translation();
        m.nodes.push(Node {
            name: p.path.clone(),
            local: Trs { t, r, s },
            primitives: vec![m.primitives.len() - 1],
            ..Default::default()
        });
    }
    if m.nodes.is_empty() {
        return Err("no Mesh prims".into());
    }
    Ok(m)
}
