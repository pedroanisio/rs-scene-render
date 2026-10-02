//! Texture sampling shared by material graphs and retained CPU textures.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub enum AddressMode {
    #[default]
    Periodic,
    Clamp,
    Mirror,
    Constant,
}
impl AddressMode {
    fn index(self, value: i64, size: u32) -> Option<u32> {
        let size = i64::from(size);
        Some(match self {
            Self::Periodic => value.rem_euclid(size),
            Self::Clamp => value.clamp(0, size - 1),
            Self::Mirror => {
                let v = value.rem_euclid(size * 2);
                if v < size {
                    v
                } else {
                    size * 2 - 1 - v
                }
            }
            Self::Constant if !(0..size).contains(&value) => return None,
            Self::Constant => value,
        } as u32)
    }
}
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub enum TextureFilter {
    Closest,
    #[default]
    Linear,
    Cubic,
}
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct TextureSampler {
    pub address: [AddressMode; 2],
    pub filter: TextureFilter,
    pub border: [f32; 4],
}
impl TextureSampler {
    /// Packed filter/address bits shared with the raster and path-tracing shaders.
    /// Bit zero is reserved for the texture's sRGB flag; bit seven marks explicit sampling.
    pub fn flags(self) -> u32 {
        let address = |a| match a {
            AddressMode::Periodic => 0,
            AddressMode::Clamp => 1,
            AddressMode::Mirror => 2,
            AddressMode::Constant => 3,
        };
        let filter = match self.filter {
            TextureFilter::Linear => 0,
            TextureFilter::Closest => 1,
            TextureFilter::Cubic => 2,
        };
        128 | (filter << 1) | (address(self.address[0]) << 3) | (address(self.address[1]) << 5)
    }
    pub fn sample(&self, size: [u32; 2], uv: [f32; 2], fetch: impl Fn(u32, u32) -> [f32; 4]) -> [f32; 4] {
        let default = self.border;
        if size.contains(&0) || uv.iter().any(|v| !v.is_finite()) {
            return default;
        }
        let [w, h] = size;
        let get = |x, y| match (self.address[0].index(x, w), self.address[1].index(y, h)) {
            (Some(x), Some(y)) => fetch(x, y),
            _ => default,
        };
        // Reduce large coordinates before conversion so neighbour offsets cannot overflow.
        let coord = |v: f32, mode: AddressMode| match mode {
            AddressMode::Periodic => v.rem_euclid(1.0),
            AddressMode::Mirror => v.rem_euclid(2.0),
            AddressMode::Clamp => v.clamp(0.0, 1.0),
            AddressMode::Constant => v.clamp(-1.0, 2.0),
        };
        let px = coord(uv[0], self.address[0]) * w as f32 - 0.5;
        let py = coord(uv[1], self.address[1]) * h as f32 - 0.5;
        let (x, y) = (px.floor() as i64, py.floor() as i64);
        let (u, v) = (px - px.floor(), py - py.floor());
        match self.filter {
            TextureFilter::Closest => get((px + 0.5).floor() as i64, (py + 0.5).floor() as i64),
            TextureFilter::Linear => {
                let (a, b, c, d) = (get(x, y), get(x + 1, y), get(x, y + 1), get(x + 1, y + 1));
                std::array::from_fn(|i| (a[i] * (1.0 - u) + b[i] * u) * (1.0 - v) + (c[i] * (1.0 - u) + d[i] * u) * v)
            }
            TextureFilter::Cubic => {
                let weights = |t: f32| {
                    [
                        -0.5 * t + t * t - 0.5 * t * t * t,
                        1.0 - 2.5 * t * t + 1.5 * t * t * t,
                        0.5 * t + 2.0 * t * t - 1.5 * t * t * t,
                        -0.5 * t * t + 0.5 * t * t * t,
                    ]
                };
                let (wx, wy) = (weights(u), weights(v));
                let mut out = [0.0; 4];
                for (j, wy) in wy.iter().enumerate() {
                    for (i, wx) in wx.iter().enumerate() {
                        let pixel = get(x + i as i64 - 1, y + j as i64 - 1);
                        for c in 0..4 {
                            out[c] += pixel[c] * wx * wy;
                        }
                    }
                }
                out
            }
        }
    }
}
