use crate::geo::DemEncoding;
use sr_geo::pmtiles::{self, Entry, Header};
use std::{
    collections::HashMap,
    fs::File,
    io::{Cursor, Read, Seek, SeekFrom},
    path::Path,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Missing {
    Error,
    Zero,
}

/// A sampler owns one file handle and bounded decoded tiles. It does not use
/// the process-global map/DEM caches. Tile dimensions are explicit, unbuffered
/// square powers of two; encoded channels are RGB8/RGBA8 numeric data.
pub struct Dem {
    header: Header,
    file: File,
    file_size: u64,
    root: Vec<Entry>,
    zoom: u8,
    encoding: DemEncoding,
    tile_size: u32,
    budget: usize,
    resident: usize,
    cache: HashMap<(u32, u32), Option<Vec<f64>>>,
}
impl Dem {
    pub fn open(path: &Path, zoom: u8, encoding: DemEncoding, tile_size: u32, budget: usize) -> Result<Self, String> {
        if zoom > 22 || tile_size == 0 || tile_size > 4096 || !tile_size.is_power_of_two() || budget < 4096 {
            return Err("invalid terrain zoom, tile dimensions or memory budget".into());
        }
        let mut file = File::open(path).map_err(|e| format!("{}: {e}", path.display()))?;
        let mut bytes = [0; pmtiles::HEADER_LEN];
        file.read_exact(&mut bytes).map_err(|e| e.to_string())?;
        let header = Header::parse(&bytes)?;
        if header.min_zoom > header.max_zoom || header.max_zoom > 31 || zoom < header.min_zoom || zoom > header.max_zoom
        {
            return Err("terrain zoom is outside the archive range".into());
        }
        if !matches!(header.tile_type, pmtiles::TileType::Png | pmtiles::TileType::Webp) {
            return Err("terrain archive must contain numeric PNG or WebP tiles".into());
        }
        let file_size = file.metadata().map_err(|e| e.to_string())?.len();
        let mut out = Self {
            header,
            file,
            file_size,
            root: Vec::new(),
            zoom,
            encoding,
            tile_size,
            budget,
            resident: 4096,
            cache: HashMap::new(),
        };
        let raw = out.section(out.header.root_offset, out.header.root_length, out.header.internal_compression)?;
        out.root = pmtiles::parse_directory(&raw)?;
        out.resident += out.root.capacity() * std::mem::size_of::<Entry>();
        if out.resident > budget {
            return Err("terrain archive directory exceeds memory budget".into());
        }
        Ok(out)
    }
    fn section(&mut self, offset: u64, len: u64, compression: u8) -> Result<Vec<u8>, String> {
        // Includes compressed/decompressed capacity and up to eight bytes of
        // parsed Entry storage per raw directory byte, including nested lookup.
        let limit = self.budget.saturating_sub(self.resident) / 16;
        if len > limit as u64 || offset.checked_add(len).is_none_or(|end| end > self.file_size) {
            return Err("terrain archive section exceeds file or memory budget".into());
        }
        let mut bytes = vec![0; len as usize];
        self.file
            .seek(SeekFrom::Start(offset))
            .and_then(|_| self.file.read_exact(&mut bytes))
            .map_err(|e| e.to_string())?;
        pmtiles::decompress_within(compression, bytes, limit as u64)
    }
    fn find(entries: &[Entry], id: u64) -> Option<Entry> {
        let at = entries.partition_point(|e| e.tile_id <= id);
        let e = *entries.get(at.checked_sub(1)?)?;
        (e.run_length == 0 || id - e.tile_id < u64::from(e.run_length)).then_some(e)
    }
    fn tile_bytes(&mut self, x: u32, y: u32) -> Result<Option<Vec<u8>>, String> {
        let id = pmtiles::tile_id(self.zoom, x, y);
        let mut entry = Self::find(&self.root, id);
        for _ in 0..4 {
            let Some(e) = entry else { return Ok(None) };
            if e.run_length > 0 {
                let offset = self.header.data_offset.checked_add(e.offset).ok_or("terrain tile offset overflow")?;
                return self.section(offset, u64::from(e.length), self.header.tile_compression).map(Some);
            }
            let offset = self.header.leaf_offset.checked_add(e.offset).ok_or("terrain directory offset overflow")?;
            let bytes = self.section(offset, u64::from(e.length), self.header.internal_compression)?;
            entry = Self::find(&pmtiles::parse_directory(&bytes)?, id);
        }
        Err("terrain archive directories nest too deeply".into())
    }
    fn load(&mut self, x: u32, y: u32) -> Result<(), String> {
        if self.cache.contains_key(&(x, y)) {
            return Ok(());
        }
        if self.cache.len() >= 4096 || self.resident.saturating_add(256) > self.budget {
            return Err("terrain tile cache exceeds memory or 4096-tile budget".into());
        }
        let tile = match self.tile_bytes(x, y)? {
            None => None,
            Some(bytes) => {
                let mut reader =
                    image::ImageReader::new(Cursor::new(&bytes)).with_guessed_format().map_err(|e| e.to_string())?;
                let format = reader.format();
                if !matches!(
                    (self.header.tile_type, format),
                    (pmtiles::TileType::Png, Some(image::ImageFormat::Png))
                        | (pmtiles::TileType::Webp, Some(image::ImageFormat::WebP))
                ) {
                    return Err("terrain tile encoding disagrees with archive header".into());
                }
                // Header inspection constructs a PNG decoder and can inflate
                // ancillary metadata. Apply its budget before inspecting the
                // dimensions, not only before decoding the pixel buffer.
                let mut header_limits = image::Limits::default();
                header_limits.max_alloc = Some((self.budget.saturating_sub(self.resident) / 16) as u64);
                header_limits.max_image_width = Some(self.tile_size);
                header_limits.max_image_height = Some(self.tile_size);
                reader.limits(header_limits);
                let (w, h) = reader.into_dimensions().map_err(|e| format!("terrain header/dimensions: {e}"))?;
                if w != self.tile_size || h != self.tile_size {
                    return Err("terrain tile dimensions do not match terrainTileSize".into());
                }
                let count = u64::from(w) * u64::from(h);
                let cost = count.saturating_mul(32).saturating_add(bytes.len() as u64 * 3).saturating_add(256);
                if cost > self.budget.saturating_sub(self.resident) as u64 {
                    return Err("terrain decoded tile exceeds memory budget".into());
                }
                reader =
                    image::ImageReader::new(Cursor::new(&bytes)).with_guessed_format().map_err(|e| e.to_string())?;
                let mut limits = image::Limits::default();
                limits.max_alloc = Some(count * 8);
                reader.limits(limits);
                let img = reader.decode().map_err(|e| e.to_string())?;
                if !matches!(img.color(), image::ColorType::Rgb8 | image::ColorType::Rgba8) {
                    return Err("terrain tile must use numeric RGB8 or RGBA8 channels".into());
                }
                let values: Vec<f64> = img
                    .into_rgb8()
                    .pixels()
                    .map(|p| {
                        let [r, g, b] = p.0.map(f64::from);
                        match self.encoding {
                            DemEncoding::Terrarium => r * 256. + g + b / 256. - 32768.,
                            DemEncoding::Mapbox => -10000. + (r * 65536. + g * 256. + b) * 0.1,
                        }
                    })
                    .collect();
                self.resident += values.capacity() * 8;
                Some(values)
            }
        };
        self.resident += 256;
        self.cache.insert((x, y), tile);
        Ok(())
    }
    fn pixel(&mut self, x: i64, y: i64, missing: Missing) -> Result<f64, String> {
        let width = i64::from(self.tile_size) * (1i64 << self.zoom);
        let x = x.rem_euclid(width);
        let y = y.clamp(0, width - 1);
        let size = i64::from(self.tile_size);
        let key = ((x / size) as u32, (y / size) as u32);
        self.load(key.0, key.1)?;
        match self.cache.get(&key).and_then(Option::as_ref) {
            Some(values) => Ok(values[((y % size) * size + x % size) as usize]),
            None if missing == Missing::Zero => Ok(0.),
            None => Err(format!("missing terrain tile {}/{}/{}", self.zoom, key.0, key.1)),
        }
    }
    /// Bilinear samples span tile boundaries; longitude wraps. Mercator stops
    /// at ±atan(sinh(pi)). Beyond it, elevation at that boundary fades to zero
    /// with smoothstep, closing both polar caps at the reference radius.
    pub fn sample(&mut self, lon: f64, lat: f64, missing: Missing) -> Result<f64, String> {
        if !lon.is_finite() || !lat.is_finite() || lat.abs() > 90. {
            return Err("invalid terrain longitude/latitude".into());
        }
        let limit = std::f64::consts::PI.sinh().atan().to_degrees();
        let u = ((90. - lat.abs()) / (90. - limit)).clamp(0., 1.);
        let fade = u * u * (3. - 2. * u);
        if fade == 0. {
            return Ok(0.);
        }
        let lon = (lon + 180.).rem_euclid(360.) - 180.;
        let [tx, ty] = sr_geo::tiles::tile_xy(self.zoom, lon, lat.clamp(-limit, limit));
        let px = tx * f64::from(self.tile_size) - 0.5;
        let py = ty * f64::from(self.tile_size) - 0.5;
        let (x, y) = (px.floor() as i64, py.floor() as i64);
        let (fx, fy) = (px - px.floor(), py - py.floor());
        let mut sum = 0.;
        for (ix, wx) in [(x, 1. - fx), (x + 1, fx)] {
            for (iy, wy) in [(y, 1. - fy), (y + 1, fy)] {
                if wx * wy > 0. {
                    sum += self.pixel(ix, iy, missing)? * wx * wy;
                }
            }
        }
        Ok(sum * fade)
    }
}
