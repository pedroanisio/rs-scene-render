//! The materials of a `.vox` file (`MATL` chunks) as numbers: what the file says of the surface of the cells of each palette index.
//!
//! A `MATL` chunk is a palette index and a dictionary of strings; the keys read here are the ones that `ogt_vox.h` of opengametools reads
//! (`_type`, `_media_type`, `_metal`, `_rough`, `_spec`, `_ior`, `_ri`, `_att`, `_flux`, `_emit`, `_ldr`, `_trans`, `_alpha`, `_d`, `_sp`,
//! `_g`, `_media`), and every other key is kept as it is spelt in [`Material::rest`].
//!
//! **Units.** MagicaVoxel's material panel is made of sliders, and the format defines no photometric unit: every number here is
//! dimensionless, the value that the file holds. `_metal`, `_rough`, `_spec`, `_trans`, `_alpha` and `_emit` are fractions of a slider
//! that goes from 0 to 1 (a value outside is kept, not clamped: it is the reader's to decide); `_ior` is the index of refraction minus one
//! and `_ri` the index of refraction itself (`ogt_vox.h` turns `_ri` into `_ior` by taking one away); `_flux` is the power of an emissive
//! material in the panel's own scale; `_att`, `_ldr`, `_d`, `_sp`, `_g` and `_media` are the panel's values of a medium and of a blend. What
//! the engine makes of them (a roughness, a transmission, an emission in nits) is the renderer's mapping and is not part of this reader.

use std::collections::BTreeMap;

/// What kind of surface a palette index is, from `_type`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Kind {
    /// `_diffuse`, and the kind of a material that names none.
    Diffuse,
    Metal,
    Glass,
    Emit,
    Blend,
    Media,
    /// A `_type` that this reader does not know, as spelt.
    Other(String),
}

/// The properties of one palette index.
#[derive(Clone, Debug, PartialEq)]
pub struct Material {
    pub kind: Kind,
    /// `_media_type` as spelt (`_absorb`, `_scatter`, `_emit`, `_sss`), if the file has one.
    pub media_type: Option<String>,
    pub metal: Option<f64>,
    pub rough: Option<f64>,
    pub spec: Option<f64>,
    /// `_ior`: the index of refraction minus one (see [`Material::refractive_index`]).
    pub ior: Option<f64>,
    /// `_ri`: the index of refraction.
    pub ri: Option<f64>,
    pub att: Option<f64>,
    pub flux: Option<f64>,
    pub emit: Option<f64>,
    pub ldr: Option<f64>,
    pub trans: Option<f64>,
    pub alpha: Option<f64>,
    pub d: Option<f64>,
    pub sp: Option<f64>,
    pub g: Option<f64>,
    pub media: Option<f64>,
    /// The keys that this reader does not read, as the file spells them.
    pub rest: BTreeMap<String, String>,
}

impl Material {
    /// The index of refraction: `_ri` if the file has it, else one plus `_ior`.
    pub fn refractive_index(&self) -> Option<f64> {
        self.ri.or(self.ior.map(|i| 1.0 + i))
    }

    /// The material of the palette index `index` from the dictionary of its `MATL` chunk; a value that is not a finite number is an error
    /// that names the index, the key and the value.
    pub fn parse(index: u8, dictionary: &BTreeMap<String, String>) -> Result<Material, String> {
        let mut rest = dictionary.clone();
        let mut number = |key: &str| -> Result<Option<f64>, String> {
            match rest.remove(key) {
                None => Ok(None),
                Some(text) => match text.trim().parse::<f64>() {
                    Ok(v) if v.is_finite() => Ok(Some(v)),
                    _ => Err(format!(
                        "the material of the palette index {index} has {key} {text:?}, which is not a finite number"
                    )),
                },
            }
        };
        let metal = number("_metal")?;
        let rough = number("_rough")?;
        let spec = number("_spec")?;
        let ior = number("_ior")?;
        let ri = number("_ri")?;
        let att = number("_att")?;
        let flux = number("_flux")?;
        let emit = number("_emit")?;
        let ldr = number("_ldr")?;
        let trans = number("_trans")?;
        let alpha = number("_alpha")?;
        let d = number("_d")?;
        let sp = number("_sp")?;
        let g = number("_g")?;
        let media = number("_media")?;
        let kind = match rest.remove("_type").as_deref() {
            None | Some("_diffuse") => Kind::Diffuse,
            Some("_metal") => Kind::Metal,
            Some("_glass") => Kind::Glass,
            Some("_emit") => Kind::Emit,
            Some("_blend") => Kind::Blend,
            Some("_media") => Kind::Media,
            Some(other) => Kind::Other(other.to_string()),
        };
        let media_type = rest.remove("_media_type");
        Ok(Material {
            kind,
            media_type,
            metal,
            rough,
            spec,
            ior,
            ri,
            att,
            flux,
            emit,
            ldr,
            trans,
            alpha,
            d,
            sp,
            g,
            media,
            rest,
        })
    }

    fn eat(&self, h: &mut Hasher) {
        match &self.kind {
            Kind::Diffuse => h.bytes(&[0]),
            Kind::Metal => h.bytes(&[1]),
            Kind::Glass => h.bytes(&[2]),
            Kind::Emit => h.bytes(&[3]),
            Kind::Blend => h.bytes(&[4]),
            Kind::Media => h.bytes(&[5]),
            Kind::Other(s) => {
                h.bytes(&[6]);
                h.string(s);
            }
        }
        match &self.media_type {
            None => h.bytes(&[0]),
            Some(s) => {
                h.bytes(&[1]);
                h.string(s);
            }
        }
        for v in [
            self.metal, self.rough, self.spec, self.ior, self.ri, self.att, self.flux, self.emit, self.ldr, self.trans,
            self.alpha, self.d, self.sp, self.g, self.media,
        ] {
            match v {
                None => h.bytes(&[0]),
                Some(v) => {
                    h.bytes(&[1]);
                    h.bytes(&v.to_bits().to_le_bytes());
                }
            }
        }
        h.bytes(&(self.rest.len() as u64).to_le_bytes());
        for (k, v) in &self.rest {
            h.string(k);
            h.string(v);
        }
    }
}

/// The materials of a file by palette index, from the dictionaries of its `MATL` chunks.
pub fn parse_all(dictionaries: &BTreeMap<u8, BTreeMap<String, String>>) -> Result<BTreeMap<u8, Material>, String> {
    dictionaries.iter().map(|(index, d)| Ok((*index, Material::parse(*index, d)?))).collect()
}

struct Hasher(u64);

impl Hasher {
    fn bytes(&mut self, bytes: &[u8]) {
        for b in bytes {
            self.0 ^= u64::from(*b);
            self.0 = self.0.wrapping_mul(0x100000001b3);
        }
    }

    fn string(&mut self, s: &str) {
        self.bytes(&(s.len() as u64).to_le_bytes());
        self.bytes(s.as_bytes());
    }
}

/// A hash of the content of the materials (every property of every index, in the order of the indices), equal for equal materials and
/// the same on any machine, for a cache that must know when a material changed. Not a cryptographic hash.
pub fn fingerprint(materials: &BTreeMap<u8, Material>) -> u64 {
    let mut h = Hasher(0xcbf29ce484222325);
    h.bytes(&(materials.len() as u64).to_le_bytes());
    for (index, m) in materials {
        h.bytes(&[*index]);
        m.eat(&mut h);
    }
    let mut x = h.0;
    x ^= x >> 33;
    x = x.wrapping_mul(0xff51afd7ed558ccd);
    x ^ (x >> 33)
}
