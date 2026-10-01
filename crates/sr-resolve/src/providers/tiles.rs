//! Map tiles from an online tile service, stored as a PMTiles archive.
//!
//! The request's `model` is the service's URL template (`{z}`, `{x}`, `{y}`,
//! `{s}` for subdomains a, b, c) and its `prompt` the tiles to fetch
//! (`z/x/y` separated by spaces), which the resolver works out from every view
//! the document's maps show. Tiles are fetched one at a time with curl
//! (`SR_CURL`), identifying the program in the User-Agent; a missing tile
//! (HTTP 404) is left out, anything else fails the run. More than
//! `SR_TILES_MAX` tiles (default 2000) is refused, to keep fetches small. The
//! service is on the network, so this provider runs only with `--allow-cloud`.

use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use sr_geo::pmtiles::{self, TileType};

use super::{tail, Provider};
use crate::protocol::{Request, Response};

pub struct Tiles;

/// The User-Agent sent to tile services.
pub fn user_agent() -> String {
    format!("scene-render/{} (+https://github.com/pedroanisio/rs-scene-render)", env!("CARGO_PKG_VERSION"))
}

/// The most tiles one tiles asset may fetch: `SR_TILES_MAX`, default 2000.
pub fn max_tiles() -> usize {
    std::env::var("SR_TILES_MAX").ok().and_then(|v| v.parse().ok()).unwrap_or(2000)
}

/// The refusal of a document whose maps need more than `max` tiles.
pub fn over_budget(max: usize) -> String {
    format!(
        "the maps need more tiles than SR_TILES_MAX ({max}); lower the zoom or the length of the moves, or use a \
         downloaded PMTiles archive"
    )
}

/// GETs `url` into `out`; returns the HTTP status.
fn get(url: &str, out: &Path) -> Result<u16, String> {
    let q = |s: &str| format!("\"{}\"", s.replace('\\', "\\\\").replace('"', "\\\""));
    let cfg = format!(
        "url = {}\nsilent\nshow-error\nlocation\nmax-time = 60\nretry = 2\nheader = {}\noutput = {}\nwrite-out = \"%{{http_code}}\"\n",
        q(url),
        q(&format!("User-Agent: {}", user_agent())),
        q(&out.display().to_string())
    );
    let curl = std::env::var("SR_CURL").unwrap_or_else(|_| "curl".into());
    let mut child = Command::new(&curl)
        .args(["--config", "-"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("cannot run {curl}: {e}"))?;
    child.stdin.take().expect("piped").write_all(cfg.as_bytes()).map_err(|e| e.to_string())?;
    let o = child.wait_with_output().map_err(|e| e.to_string())?;
    if !o.status.success() {
        return Err(format!("{url}: {}", tail(&String::from_utf8_lossy(&o.stderr))));
    }
    String::from_utf8_lossy(&o.stdout).trim().parse().map_err(|_| format!("{url}: no HTTP status"))
}

/// The URL of one tile.
pub fn tile_url(template: &str, z: u8, x: u32, y: u32) -> String {
    let s = ["a", "b", "c"][((x + y) % 3) as usize];
    template
        .replace("{z}", &z.to_string())
        .replace("{x}", &x.to_string())
        .replace("{y}", &y.to_string())
        .replace("{s}", s)
}

fn kind_of(b: &[u8]) -> (TileType, u8) {
    if b.starts_with(b"\x89PNG") {
        (TileType::Png, 1)
    } else if b.starts_with(&[0xff, 0xd8]) {
        (TileType::Jpeg, 1)
    } else if b.len() > 12 && &b[0..4] == b"RIFF" && &b[8..12] == b"WEBP" {
        (TileType::Webp, 1)
    } else if b.starts_with(&[0x1f, 0x8b]) {
        (TileType::Mvt, 2)
    } else {
        (TileType::Mvt, 1)
    }
}

impl Provider for Tiles {
    fn name(&self) -> String {
        "tiles".into()
    }
    fn cloud(&self) -> bool {
        true
    }

    fn run(&self, req: &Request) -> Result<Response, String> {
        let wanted: Vec<(u8, u32, u32)> = req
            .prompt
            .as_deref()
            .unwrap_or("")
            .split_whitespace()
            .filter_map(|t| {
                let mut it = t.split('/').map(|v| v.parse::<u32>());
                match (it.next(), it.next(), it.next()) {
                    (Some(Ok(z)), Some(Ok(x)), Some(Ok(y))) => Some((z as u8, x, y)),
                    _ => None,
                }
            })
            .collect();
        let max = max_tiles();
        if wanted.len() > max {
            return Err(over_budget(max));
        }
        let work = PathBuf::from(&req.workdir);
        let mut tiles = Vec::with_capacity(wanted.len());
        let mut kind = None;
        let mut missing = 0;
        for &(z, x, y) in &wanted {
            let f = work.join("tile");
            let url = tile_url(&req.model, z, x, y);
            match get(&url, &f)? {
                200 => {
                    let b = std::fs::read(&f).map_err(|e| e.to_string())?;
                    kind.get_or_insert(kind_of(&b));
                    tiles.push(((z, x, y), b));
                }
                404 | 204 => missing += 1,
                code => return Err(format!("{url}: HTTP {code}")),
            }
        }
        let (tt, comp) = kind.unwrap_or((TileType::Png, 1));
        let archive = pmtiles::write(&tiles, tt, comp, &serde_json::json!({ "source": req.model }));
        std::fs::write(&req.output, archive).map_err(|e| e.to_string())?;
        let mut notes = vec![format!("{} tiles fetched", tiles.len())];
        if missing > 0 {
            notes.push(format!("{missing} tiles missing on the service"));
        }
        Ok(Response { ok: true, version: Some("tiles 1".into()), notes, ..Default::default() })
    }
}
