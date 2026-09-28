//! MaterialX documents translated onto the shading model.
//!
//! The first `standard_surface`, `open_pbr_surface` or `gltf_pbr` node of a
//! `.mtlx` document supplies the material: constant inputs map onto
//! [`MaterialParams`], and inputs connected to `image`/`tiledimage` nodes
//! (directly, or through a `normalmap` node for normals) become texture maps.
//! Other node graphs are reported, because no MaterialX shader generator is
//! linked.

use std::path::{Path, PathBuf};

use crate::MaterialParams;

/// A translated MaterialX material.
#[derive(Clone, Debug, Default)]
pub struct MtlxMaterial {
    pub params: MaterialParams,
    pub base_color_map: Option<PathBuf>,
    pub normal_map: Option<PathBuf>,
    pub roughness_map: Option<PathBuf>,
    pub warnings: Vec<String>,
}

fn nums(s: &str) -> Vec<f32> {
    s.split(|c: char| c == ',' || c.is_whitespace()).filter_map(|t| t.parse().ok()).collect()
}

/// Parses a `.mtlx` file.
pub fn load(path: &Path) -> Result<MtlxMaterial, String> {
    let text = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
    parse(&text, path.parent().unwrap_or(Path::new(".")))
}

/// Parses MaterialX text; file names resolve against `base` (and the document's `fileprefix`).
pub fn parse(text: &str, base: &Path) -> Result<MtlxMaterial, String> {
    let doc = roxmltree::Document::parse(text).map_err(|e| format!("MaterialX: {e}"))?;
    let root = doc.root_element();
    let prefix = root.attribute("fileprefix").unwrap_or("");
    let all: Vec<roxmltree::Node> = root.descendants().filter(|n| n.is_element()).collect();
    let by_name = |name: &str| all.iter().find(|n| n.attribute("name") == Some(name)).copied();
    let surface = all
        .iter()
        .find(|n| matches!(n.tag_name().name(), "standard_surface" | "open_pbr_surface" | "gltf_pbr"))
        .copied()
        .ok_or("no standard_surface, open_pbr_surface or gltf_pbr node")?;
    let kind = surface.tag_name().name();
    let mut out = MtlxMaterial::default();
    let mut vals: std::collections::HashMap<String, Vec<f32>> = Default::default();
    let mut maps: std::collections::HashMap<String, PathBuf> = Default::default();
    for input in surface.children().filter(|c| c.has_tag_name("input")) {
        let Some(name) = input.attribute("name") else { continue };
        if let Some(v) = input.attribute("value") {
            vals.insert(name.to_string(), nums(v));
        } else if let Some(node) = input.attribute("nodename").and_then(by_name) {
            // image, tiledimage, or normalmap → image
            let img = if node.tag_name().name() == "normalmap" {
                node.children()
                    .filter(|c| c.has_tag_name("input") && c.attribute("name") == Some("in"))
                    .find_map(|c| c.attribute("nodename"))
                    .and_then(by_name)
            } else {
                Some(node)
            };
            match img.filter(|n| matches!(n.tag_name().name(), "image" | "tiledimage")) {
                Some(img) => {
                    if let Some(file) = img
                        .children()
                        .find(|c| c.has_tag_name("input") && c.attribute("name") == Some("file"))
                        .and_then(|c| c.attribute("value"))
                    {
                        maps.insert(name.to_string(), base.join(format!("{prefix}{file}")));
                    }
                }
                None => out
                    .warnings
                    .push(format!("MaterialX input {name}: node graph <{}> is not translated", node.tag_name().name())),
            }
        } else if input.attribute("nodegraph").is_some() || input.attribute("output").is_some() {
            out.warnings.push(format!("MaterialX input {name}: node graphs are not translated"));
        }
    }
    let f = |n: &str, d: f32| vals.get(n).and_then(|v| v.first().copied()).unwrap_or(d);
    let c3 = |n: &str, d: [f32; 3]| vals.get(n).filter(|v| v.len() >= 3).map(|v| [v[0], v[1], v[2]]).unwrap_or(d);
    let p = &mut out.params;
    let scale3 = |c: [f32; 3], k: f32| [c[0] * k, c[1] * k, c[2] * k];
    match kind {
        "standard_surface" => {
            let bc = scale3(c3("base_color", [0.8; 3]), f("base", 1.0));
            let op = c3("opacity", [1.0; 3]);
            p.base_color = [bc[0], bc[1], bc[2], (op[0] + op[1] + op[2]) / 3.0];
            p.metallic = f("metalness", 0.0);
            p.roughness = f("specular_roughness", 0.2);
            p.specular = f("specular", 1.0);
            p.specular_color = c3("specular_color", [1.0; 3]);
            p.ior = f("specular_IOR", 1.5);
            p.anisotropy = f("specular_anisotropy", 0.0);
            p.anisotropy_rotation = f("specular_rotation", 0.0) * std::f32::consts::TAU;
            p.transmission = f("transmission", 0.0);
            p.attenuation_color = c3("transmission_color", [1.0; 3]);
            p.clearcoat = f("coat", 0.0);
            p.clearcoat_roughness = f("coat_roughness", 0.1);
            p.sheen_color = scale3(c3("sheen_color", [1.0; 3]), f("sheen", 0.0));
            p.sheen_roughness = f("sheen_roughness", 0.3);
            p.emissive = scale3(c3("emission_color", [1.0; 3]), f("emission", 0.0));
            if f("thin_film_thickness", 0.0) > 0.0 {
                p.iridescence = 1.0;
                p.iridescence_ior = f("thin_film_IOR", 1.5);
                p.iridescence_thickness = f("thin_film_thickness", 0.0);
            }
        }
        "open_pbr_surface" => {
            let bc = scale3(c3("base_color", [0.8; 3]), f("base_weight", 1.0));
            p.base_color = [bc[0], bc[1], bc[2], f("geometry_opacity", 1.0)];
            p.metallic = f("base_metalness", 0.0);
            p.roughness = f("specular_roughness", 0.3);
            p.specular = f("specular_weight", 1.0);
            p.specular_color = c3("specular_color", [1.0; 3]);
            p.ior = f("specular_ior", 1.5);
            p.anisotropy = f("specular_roughness_anisotropy", 0.0);
            p.transmission = f("transmission_weight", 0.0);
            p.attenuation_color = c3("transmission_color", [1.0; 3]);
            p.clearcoat = f("coat_weight", 0.0);
            p.clearcoat_roughness = f("coat_roughness", 0.0);
            p.sheen_color = scale3(c3("fuzz_color", [1.0; 3]), f("fuzz_weight", 0.0));
            p.sheen_roughness = f("fuzz_roughness", 0.5);
            // emission_luminance is in nits; 1000 nits maps to an emissive strength of 1
            p.emissive = c3("emission_color", [1.0; 3]);
            p.emissive_strength = f("emission_luminance", 0.0) / 1000.0;
            if f("thin_film_weight", 0.0) > 0.0 {
                p.iridescence = f("thin_film_weight", 0.0);
                p.iridescence_ior = f("thin_film_ior", 1.4);
                p.iridescence_thickness = f("thin_film_thickness", 0.5) * 1000.0;
            }
        }
        _ => {
            let bc = c3("base_color", [1.0; 3]);
            p.base_color = [bc[0], bc[1], bc[2], f("alpha", 1.0)];
            p.metallic = f("metallic", 1.0);
            p.roughness = f("roughness", 1.0);
            p.ior = f("ior", 1.5);
            p.transmission = f("transmission", 0.0);
            p.clearcoat = f("clearcoat", 0.0);
            p.clearcoat_roughness = f("clearcoat_roughness", 0.0);
            p.sheen_color = c3("sheen_color", [0.0; 3]);
            p.sheen_roughness = f("sheen_roughness", 0.0);
            p.emissive = c3("emissive", [0.0; 3]);
            p.emissive_strength = f("emissive_strength", 1.0);
            p.iridescence = f("iridescence", 0.0);
            p.iridescence_ior = f("iridescence_ior", 1.3);
        }
    }
    if p.base_color[3] < 1.0 {
        p.alpha_mode = crate::AlphaMode::Blend;
    }
    out.base_color_map = maps.remove("base_color");
    out.normal_map = maps.remove("normal");
    out.roughness_map = maps.remove("specular_roughness").or_else(|| maps.remove("roughness"));
    for k in maps.keys() {
        out.warnings.push(format!("MaterialX input {k}: textures are only used for base colour, normal and roughness"));
    }
    Ok(out)
}
