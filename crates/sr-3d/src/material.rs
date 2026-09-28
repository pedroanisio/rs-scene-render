//! Material parameters shared by imported and document materials: glTF 2.0
//! metallic-roughness with the ratified KHR_materials_* extensions.

/// How alpha is used.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum AlphaMode {
    #[default]
    Opaque,
    Mask,
    Blend,
}

/// Every scalar and colour input of the shading model. Colours are linear.
#[derive(Clone, Debug, PartialEq)]
pub struct MaterialParams {
    pub base_color: [f32; 4],
    pub metallic: f32,
    pub roughness: f32,
    pub emissive: [f32; 3],
    pub emissive_strength: f32,
    pub opacity: f32,
    pub alpha_mode: AlphaMode,
    pub alpha_cutoff: f32,
    pub double_sided: bool,
    pub unlit: bool,
    pub clearcoat: f32,
    pub clearcoat_roughness: f32,
    pub transmission: f32,
    pub ior: f32,
    pub thickness: f32,
    pub attenuation_color: [f32; 3],
    /// Infinite when absent.
    pub attenuation_distance: f32,
    pub sheen_color: [f32; 3],
    pub sheen_roughness: f32,
    pub specular: f32,
    pub specular_color: [f32; 3],
    pub iridescence: f32,
    pub iridescence_ior: f32,
    pub iridescence_thickness: f32,
    pub anisotropy: f32,
    /// Radians.
    pub anisotropy_rotation: f32,
    pub dispersion: f32,
    pub normal_scale: f32,
    pub occlusion_strength: f32,
    pub displacement_scale: f32,
    pub uv_scale: [f32; 2],
}

impl Default for MaterialParams {
    fn default() -> MaterialParams {
        MaterialParams {
            base_color: [1.0; 4],
            metallic: 0.0,
            roughness: 0.5,
            emissive: [0.0; 3],
            emissive_strength: 1.0,
            opacity: 1.0,
            alpha_mode: AlphaMode::Opaque,
            alpha_cutoff: 0.5,
            double_sided: false,
            unlit: false,
            clearcoat: 0.0,
            clearcoat_roughness: 0.0,
            transmission: 0.0,
            ior: 1.5,
            thickness: 0.0,
            attenuation_color: [1.0; 3],
            attenuation_distance: f32::INFINITY,
            sheen_color: [0.0; 3],
            sheen_roughness: 0.0,
            specular: 1.0,
            specular_color: [1.0; 3],
            iridescence: 0.0,
            iridescence_ior: 1.3,
            iridescence_thickness: 400.0,
            anisotropy: 0.0,
            anisotropy_rotation: 0.0,
            dispersion: 0.0,
            normal_scale: 1.0,
            occlusion_strength: 1.0,
            displacement_scale: 0.0,
            uv_scale: [1.0, 1.0],
        }
    }
}

/// Linear value of an 8-bit-style sRGB-encoded channel in 0..1.
pub fn srgb_to_linear(v: f32) -> f32 {
    if v <= 0.04045 {
        v / 12.92
    } else {
        ((v + 0.055) / 1.055).powf(2.4)
    }
}
