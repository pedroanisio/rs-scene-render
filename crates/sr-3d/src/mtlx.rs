//! MaterialX documents translated onto the shading model.
//!
//! The first `standard_surface`, `open_pbr_surface` or `gltf_pbr` node of a
//! `.mtlx` document supplies the material: constant inputs map onto
//! [`MaterialParams`], and inputs connected to `image`/`tiledimage` nodes
//! (directly, or through a `normalmap` node for normals) become texture maps.
//! Pattern graphs resolve named outputs, evaluate constants and bake varying
//! inputs into linear textures. Cycles and unsupported operators are errors.

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
    /// Baked linear graph outputs in the renderer's six texture slots.
    pub generated_maps: [Option<crate::Texture>; 6],
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
    let all: Vec<roxmltree::Node> = root.descendants().filter(|n| n.is_element()).collect();
    fn by_name<'a, 'input>(from: roxmltree::Node<'a, 'input>, name: &str) -> Option<roxmltree::Node<'a, 'input>> {
        from.ancestors().skip(1).find_map(|scope| scope.children().find(|node| node.attribute("name") == Some(name)))
    }
    let surface = all
        .iter()
        .find(|n| matches!(n.tag_name().name(), "standard_surface" | "open_pbr_surface" | "gltf_pbr"))
        .copied()
        .ok_or("no standard_surface, open_pbr_surface or gltf_pbr node")?;
    let kind = surface.tag_name().name();
    let mut out = MtlxMaterial::default();
    let mut vals: std::collections::HashMap<String, Vec<f32>> = Default::default();
    let mut maps: std::collections::HashMap<String, PathBuf> = Default::default();
    let mut graph = crate::mtlx_graph::Compiler::new(root, base);
    let mut graph_maps = std::collections::HashMap::new();
    for input in surface.children().filter(|c| c.has_tag_name("input")) {
        let Some(name) = input.attribute("name") else { continue };
        if let Some(v) = input.attribute("value") {
            vals.insert(name.to_string(), nums(v));
        } else if let Some(node) = input.attribute("nodename").and_then(|name| by_name(input, name)) {
            // Direct image paths remain lazy. Normal-map nodes need graph
            // evaluation so strength and vector decoding are preserved.
            let img = Some(node);
            match img.filter(|n| {
                n.has_tag_name("image")
                    && matches!(name, "base_color" | "normal" | "roughness" | "specular_roughness")
                    && n.attribute("colorspace").is_none()
                    && n.children().filter(|c| c.has_tag_name("input")).all(|c| c.attribute("name") == Some("file"))
            }) {
                Some(img) => {
                    if let Some(file) = img
                        .children()
                        .find(|c| c.has_tag_name("input") && c.attribute("name") == Some("file"))
                        .and_then(|c| c.attribute("value"))
                    {
                        let prefix = img
                            .ancestors()
                            .filter_map(|n| n.attribute("fileprefix"))
                            .collect::<Vec<_>>()
                            .into_iter()
                            .rev()
                            .collect::<String>();
                        maps.insert(name.to_string(), base.join(format!("{prefix}{file}")));
                    }
                }
                None => {
                    let expression = graph.input(input)?;
                    if expression.varying || name == "normal" {
                        graph_maps.insert(name.to_string(), expression);
                    } else {
                        vals.insert(name.to_string(), expression.eval([0.0; 2]).to_vec());
                    }
                }
            }
        } else if input.attribute("nodegraph").is_some()
            || input.attribute("output").is_some()
            || input.attribute("nodename").is_some()
        {
            let expression = graph.input(input)?;
            if expression.varying || name == "normal" {
                graph_maps.insert(name.to_string(), expression);
            } else {
                vals.insert(name.to_string(), expression.eval([0.0; 2]).to_vec());
            }
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
    let base_weight = match kind {
        "standard_surface" => f("base", 1.0),
        "open_pbr_surface" => f("base_weight", 1.0),
        _ => 1.0,
    };
    let emission_weight = if kind == "standard_surface" { f("emission", 0.0) } else { 1.0 };
    if maps.contains_key("base_color") {
        p.base_color[..3].fill(base_weight);
    }
    if maps.contains_key("specular_roughness") || maps.contains_key("roughness") {
        p.roughness = 1.0;
    }
    for (name, expression) in graph_maps {
        graph.reserve_bake(expression.size)?;
        let (mut texture, factor) = expression.bake(name == "normal");
        let slot = match name.as_str() {
            "base_color" => {
                p.base_color[..3].copy_from_slice(&scale3([factor[0], factor[1], factor[2]], base_weight));
                0
            }
            "normal" => 1,
            "specular_roughness" | "roughness" => {
                p.roughness = factor[0];
                if let Some(sampler) = texture.sampler.as_mut() {
                    sampler.border[1] = sampler.border[0];
                    sampler.border[2] = 1.0;
                }
                for pixel in texture.rgba.chunks_exact_mut(4) {
                    pixel[1] = pixel[0];
                    pixel[2] = 255;
                }
                2
            }
            "emission_color" | "emissive" => {
                p.emissive = scale3([factor[0], factor[1], factor[2]], emission_weight);
                4
            }
            "occlusion" => 3,
            "displacement" => 5,
            _ => return Err(format!("varying MaterialX input {name} has no texture slot in the shading model")),
        };
        out.generated_maps[slot] = Some(texture);
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
