//! Captions: subtitle parsers (SRT, WebVTT, ASS, TTML, ITT, SCC),
//! transcription caches, word timing, paging, profanity filtering,
//! sidecar writers and the per-word effects of the 13 presets.

use sr_vector::geom::Xf;
use sr_vector::Paint;

use crate::glyph::GlyphFx;
use crate::layout::Layout;

/// A timed word.
#[derive(Debug, Clone, PartialEq)]
pub struct Word {
    pub start: f64,
    pub end: f64,
    pub text: String,
    pub emphasis: bool,
}

/// A caption cue.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Cue {
    pub start: f64,
    pub end: f64,
    pub text: String,
    pub speaker: Option<String>,
    pub style: Option<String>,
    pub position: Option<String>,
    /// Word timings when the source has them.
    pub words: Vec<Word>,
}

/// A parse error.
#[derive(Debug, Clone, PartialEq, thiserror::Error)]
#[error("{format} line {line}: {message}")]
pub struct CaptionError {
    pub format: &'static str,
    pub line: usize,
    pub message: String,
}

fn err(format: &'static str, line: usize, m: impl Into<String>) -> CaptionError {
    CaptionError { format, line, message: m.into() }
}

/// `HH:MM:SS,mmm`, `HH:MM:SS.mmm`, `MM:SS.mmm` or `H:MM:SS.cc`.
fn clock(s: &str) -> Option<f64> {
    let s = s.trim().replace(',', ".");
    let parts: Vec<&str> = s.split(':').collect();
    let nums: Vec<f64> = parts.iter().map(|x| x.parse::<f64>().ok()).collect::<Option<Vec<_>>>()?;
    Some(match nums.len() {
        3 => nums[0] * 3600.0 + nums[1] * 60.0 + nums[2],
        2 => nums[0] * 60.0 + nums[1],
        1 => nums[0],
        _ => return None,
    })
}

fn strip_tags(s: &str) -> String {
    let mut out = String::new();
    let mut depth = 0;
    for c in s.chars() {
        match c {
            '<' => depth += 1,
            '>' if depth > 0 => depth -= 1,
            _ if depth == 0 => out.push(c),
            _ => {}
        }
    }
    out.replace("&amp;", "&").replace("&lt;", "<").replace("&gt;", ">").replace("&nbsp;", " ")
}

/// Parses a subtitle file.
pub fn parse(text: &str, format: &str) -> Result<Vec<Cue>, CaptionError> {
    let text = text.trim_start_matches('\u{feff}').replace("\r\n", "\n");
    match format {
        "srt" => parse_srt(&text),
        "vtt" => parse_vtt(&text),
        "ass" => parse_ass(&text),
        "ttml" | "itt" => parse_ttml(&text),
        "scc" => parse_scc(&text),
        other => Err(err("captions", 0, format!("unknown format {other}"))),
    }
}

fn parse_srt(text: &str) -> Result<Vec<Cue>, CaptionError> {
    let mut out = Vec::new();
    let lines: Vec<&str> = text.lines().collect();
    let mut i = 0;
    while i < lines.len() {
        if !lines[i].contains("-->") {
            i += 1;
            continue;
        }
        let (a, b) = lines[i].split_once("-->").unwrap();
        let (Some(s), Some(e)) = (clock(a), clock(b.split_whitespace().next().unwrap_or(""))) else {
            return Err(err("SRT", i + 1, "bad timing line"));
        };
        let mut body = Vec::new();
        i += 1;
        while i < lines.len() && !lines[i].trim().is_empty() {
            body.push(strip_tags(lines[i]));
            i += 1;
        }
        out.push(Cue { start: s, end: e, text: body.join("\n"), ..Default::default() });
    }
    Ok(out)
}

fn parse_vtt(text: &str) -> Result<Vec<Cue>, CaptionError> {
    if !text.trim_start().starts_with("WEBVTT") {
        return Err(err("WebVTT", 1, "missing WEBVTT header"));
    }
    let mut out = Vec::new();
    let lines: Vec<&str> = text.lines().collect();
    let mut i = 0;
    while i < lines.len() {
        if !lines[i].contains("-->") {
            i += 1;
            continue;
        }
        let (a, b) = lines[i].split_once("-->").unwrap();
        let mut rest = b.split_whitespace();
        let (Some(s), Some(e)) = (clock(a), rest.next().and_then(clock)) else {
            return Err(err("WebVTT", i + 1, "bad timing line"));
        };
        let position = {
            let r: Vec<&str> = rest.collect();
            (!r.is_empty()).then(|| r.join(" "))
        };
        let mut raw = Vec::new();
        i += 1;
        while i < lines.len() && !lines[i].trim().is_empty() {
            raw.push(lines[i]);
            i += 1;
        }
        let body = raw.join("\n");
        let speaker = body.find("<v ").and_then(|k| body[k + 3..].split('>').next()).map(|x| x.trim().to_string());
        // inline timestamps give word timing (karaoke)
        let mut words = Vec::new();
        let mut t = s;
        let mut rest = body.as_str();
        let has_ts = body.contains("<0") || body.contains("<1") || body.contains("<2");
        if has_ts {
            while !rest.is_empty() {
                let (chunk, next) = match rest.find('<') {
                    Some(k) => (&rest[..k], &rest[k..]),
                    None => (rest, ""),
                };
                for w in strip_tags(chunk).split_whitespace() {
                    words.push(Word { start: t, end: t, text: w.to_string(), emphasis: false });
                }
                if next.is_empty() {
                    break;
                }
                // an unclosed tag runs to the end of the cue
                let close = next.find('>').unwrap_or(next.len());
                if let Some(ts) = clock(&next[1..close]) {
                    t = ts;
                }
                rest = next.get(close + 1..).unwrap_or("");
            }
            for k in 0..words.len() {
                words[k].end = words.get(k + 1).map(|w| w.start).unwrap_or(e).max(words[k].start);
            }
        }
        out.push(Cue { start: s, end: e, text: strip_tags(&body), speaker, position, words, ..Default::default() });
    }
    Ok(out)
}

fn parse_ass(text: &str) -> Result<Vec<Cue>, CaptionError> {
    let mut fields: Vec<String> = Vec::new();
    let mut out = Vec::new();
    let mut in_events = false;
    for (ln, l) in text.lines().enumerate() {
        let t = l.trim();
        if t.starts_with('[') {
            in_events = t.eq_ignore_ascii_case("[events]");
            continue;
        }
        if !in_events {
            continue;
        }
        if let Some(f) = t.strip_prefix("Format:") {
            fields = f.split(',').map(|x| x.trim().to_ascii_lowercase()).collect();
            continue;
        }
        let Some(d) = t.strip_prefix("Dialogue:") else { continue };
        if fields.is_empty() {
            return Err(err("ASS", ln + 1, "Dialogue before Format"));
        }
        let parts: Vec<&str> = d.splitn(fields.len(), ',').collect();
        let get = |name: &str| fields.iter().position(|f| f == name).and_then(|k| parts.get(k)).map(|x| x.trim());
        let (Some(s), Some(e)) = (get("start").and_then(clock), get("end").and_then(clock)) else {
            return Err(err("ASS", ln + 1, "bad times"));
        };
        let raw = get("text").unwrap_or("");
        // {\k<cs>} karaoke timings
        let mut words = Vec::new();
        let mut t = s;
        let mut plain = String::new();
        let mut k = 0;
        let rb = raw.as_bytes();
        let mut pending: Option<f64> = None;
        let mut cur = String::new();
        let flush = |cur: &mut String, t: &mut f64, dur: Option<f64>, words: &mut Vec<Word>| {
            let txt = cur.trim().to_string();
            if let Some(d) = dur {
                if !txt.is_empty() {
                    words.push(Word { start: *t, end: *t + d, text: txt, emphasis: false });
                }
                *t += d;
            }
            cur.clear();
        };
        while k < rb.len() {
            if rb[k] == b'{' {
                let close = raw[k..].find('}').map(|c| k + c).unwrap_or(rb.len());
                let tag = &raw[k + 1..close];
                for part in tag.split('\\').filter(|p| !p.is_empty()) {
                    let lower = part.to_ascii_lowercase();
                    let num = lower.trim_start_matches(|c: char| c.is_alphabetic());
                    if (lower.starts_with("kf") || lower.starts_with("ko") || lower.starts_with('k'))
                        && num.parse::<f64>().is_ok()
                    {
                        flush(&mut cur, &mut t, pending.take(), &mut words);
                        pending = num.parse::<f64>().ok().map(|cs| cs / 100.0);
                    }
                }
                k = close + 1;
                continue;
            }
            if raw[k..].starts_with("\\N") || raw[k..].starts_with("\\n") {
                plain.push('\n');
                cur.push(' ');
                k += 2;
                continue;
            }
            let ch = raw[k..].chars().next().unwrap();
            plain.push(ch);
            cur.push(ch);
            k += ch.len_utf8();
        }
        flush(&mut cur, &mut t, pending, &mut words);
        out.push(Cue {
            start: s,
            end: e,
            text: plain.trim().to_string(),
            speaker: get("name").filter(|x| !x.is_empty()).map(str::to_string),
            style: get("style").map(str::to_string),
            words,
            ..Default::default()
        });
    }
    Ok(out)
}

/// TTML and ITT (TTML with frame timing).
fn parse_ttml(text: &str) -> Result<Vec<Cue>, CaptionError> {
    let doc = roxmltree::Document::parse(text).map_err(|e| err("TTML", e.pos().row as usize, e.to_string()))?;
    let root = doc.root_element();
    let attr =
        |n: roxmltree::Node, name: &str| n.attributes().find(|a| a.name() == name).map(|a| a.value().to_string());
    let fps = attr(root, "frameRate").and_then(|v| v.parse::<f64>().ok()).unwrap_or(30.0);
    let mult = attr(root, "frameRateMultiplier").and_then(|v| {
        let mut it = v.split_whitespace().filter_map(|x| x.parse::<f64>().ok());
        Some(it.next()? / it.next()?)
    });
    let fps = fps * mult.unwrap_or(1.0);
    let tick = attr(root, "tickRate").and_then(|v| v.parse::<f64>().ok()).unwrap_or(1.0);
    let time = |s: &str| -> Option<f64> {
        let s = s.trim();
        for (suf, k) in [("ms", 0.001), ("h", 3600.0), ("m", 60.0), ("s", 1.0), ("f", 1.0 / fps), ("t", 1.0 / tick)] {
            if let Some(v) = s.strip_suffix(suf) {
                if let Ok(x) = v.parse::<f64>() {
                    return Some(x * k);
                }
            }
        }
        let parts: Vec<&str> = s.split(':').collect();
        if parts.len() == 4 {
            let n: Vec<f64> = parts.iter().filter_map(|x| x.parse().ok()).collect();
            return (n.len() == 4).then(|| n[0] * 3600.0 + n[1] * 60.0 + n[2] + n[3] / fps);
        }
        clock(s)
    };
    let mut out = Vec::new();
    for p in doc.descendants().filter(|n| n.has_tag_name("p") || n.tag_name().name() == "p") {
        let (Some(s), end) = (attr(p, "begin").and_then(|v| time(&v)), attr(p, "end").and_then(|v| time(&v))) else {
            continue;
        };
        let e = end.or_else(|| attr(p, "dur").and_then(|v| time(&v)).map(|d| s + d)).unwrap_or(s + 2.0);
        let mut body = String::new();
        let mut words = Vec::new();
        for n in p.descendants() {
            if n.is_text() {
                body.push_str(n.text().unwrap_or(""));
            } else if n.tag_name().name() == "br" {
                body.push('\n');
            }
            if n.tag_name().name() == "span" {
                if let Some(ws) = attr(n, "begin").and_then(|v| time(&v)) {
                    let we = attr(n, "end").and_then(|v| time(&v)).map(|x| x + s).unwrap_or(e);
                    let txt: String = n.descendants().filter(|x| x.is_text()).filter_map(|x| x.text()).collect();
                    words.push(Word { start: s + ws, end: we, text: txt.trim().to_string(), emphasis: false });
                }
            }
        }
        out.push(Cue {
            start: s,
            end: e,
            text: body
                .split('\n')
                .map(|l| l.split_whitespace().collect::<Vec<_>>().join(" "))
                .filter(|l| !l.is_empty())
                .collect::<Vec<_>>()
                .join("\n"),
            speaker: attr(p, "agent"),
            style: attr(p, "style"),
            words,
            ..Default::default()
        });
    }
    if out.is_empty() && !text.contains("<p") {
        return Err(err("TTML", 1, "no <p> cues"));
    }
    Ok(out)
}

/// SCC (CEA-608 in Scenarist format), pop-on, roll-up and paint-on.
fn parse_scc(text: &str) -> Result<Vec<Cue>, CaptionError> {
    if !text.trim_start().starts_with("Scenarist_SCC") {
        return Err(err("SCC", 1, "missing Scenarist_SCC header"));
    }
    const SPECIAL: [&str; 16] = ["®", "°", "½", "¿", "™", "¢", "£", "♪", "à", " ", "è", "â", "ê", "î", "ô", "û"];
    let basic = |b: u8| -> char {
        match b {
            0x2A => 'á',
            0x5C => 'é',
            0x5E => 'í',
            0x5F => 'ó',
            0x60 => 'ú',
            0x7B => 'ç',
            0x7C => '÷',
            0x7D => 'Ñ',
            0x7E => 'ñ',
            0x7F => '█',
            b => b as char,
        }
    };
    let fps = 30000.0 / 1001.0;
    let mut out = Vec::new();
    let (mut buffer, mut shown) = (String::new(), String::new());
    let mut shown_at: Option<f64> = None;
    let mut direct = false;
    let mut last_ctrl: Option<(u8, u8)> = None;
    for (ln, l) in text.lines().enumerate().skip(1) {
        let Some((tc, data)) = l.split_once('\t').or_else(|| l.split_once(' ')) else { continue };
        let drop = tc.contains(';');
        let n: Vec<f64> = tc.split([':', ';']).filter_map(|x| x.parse().ok()).collect();
        if n.len() != 4 {
            continue;
        }
        let frames = (n[0] * 3600.0 + n[1] * 60.0 + n[2]) * 30.0 + n[3];
        let mut t = if drop {
            (frames - 2.0 * ((frames / 1800.0).floor() - (frames / 18000.0).floor())) / fps
        } else {
            frames / fps
        };
        for word in data.split_whitespace() {
            let v = u16::from_str_radix(word, 16).map_err(|_| err("SCC", ln + 1, format!("bad byte pair {word}")))?;
            let (b1, b2) = (((v >> 8) as u8) & 0x7F, (v as u8) & 0x7F);
            t += 1.0 / fps;
            let target = if direct { &mut shown } else { &mut buffer };
            if (0x10..=0x1F).contains(&b1) {
                if last_ctrl == Some((b1, b2)) {
                    last_ctrl = None;
                    continue; // control codes are sent twice
                }
                last_ctrl = Some((b1, b2));
                let b1c = b1 & 0x17;
                if b1c == 0x14 {
                    match b2 {
                        0x20 => direct = false,
                        0x25..=0x27 | 0x29 => direct = true,
                        0x2C => {
                            if let Some(s) = shown_at.take() {
                                if !shown.trim().is_empty() {
                                    out.push(Cue {
                                        start: s,
                                        end: t,
                                        text: shown.trim().to_string(),
                                        ..Default::default()
                                    });
                                }
                            }
                            shown.clear();
                        }
                        0x2D => {
                            if direct {
                                if let Some(s) = shown_at.take() {
                                    if !shown.trim().is_empty() {
                                        out.push(Cue {
                                            start: s,
                                            end: t,
                                            text: shown.trim().to_string(),
                                            ..Default::default()
                                        });
                                    }
                                }
                                shown.clear();
                            } else {
                                target.push('\n');
                            }
                        }
                        0x2E => buffer.clear(),
                        0x2F => {
                            if let Some(s) = shown_at.take() {
                                if !shown.trim().is_empty() {
                                    out.push(Cue {
                                        start: s,
                                        end: t,
                                        text: shown.trim().to_string(),
                                        ..Default::default()
                                    });
                                }
                            }
                            shown = std::mem::take(&mut buffer);
                            shown_at = Some(t);
                        }
                        _ => {}
                    }
                } else if b1c == 0x11 && (0x30..=0x3F).contains(&b2) {
                    target.push_str(SPECIAL[(b2 - 0x30) as usize]);
                } else if (0x40..=0x7F).contains(&b2) && !target.is_empty() && !target.ends_with('\n') {
                    // a preamble address starts a new row
                    target.push('\n');
                }
                if direct && shown_at.is_none() {
                    shown_at = Some(t);
                }
                continue;
            }
            last_ctrl = None;
            for b in [b1, b2] {
                if b >= 0x20 {
                    target.push(basic(b));
                }
            }
            if direct && shown_at.is_none() {
                shown_at = Some(t);
            }
        }
    }
    if let Some(s) = shown_at {
        if !shown.trim().is_empty() {
            out.push(Cue { start: s, end: s + 3.0, text: shown.trim().to_string(), ..Default::default() });
        }
    }
    Ok(out)
}

/// Reads a transcription cache: `{"words": [{"start", "end", "text"|"word"}]}` or
/// `{"segments": [{"start", "end", "text", "words": [...]}]}` (Whisper-style).
pub fn from_transcript(json: &str) -> Result<Vec<Cue>, CaptionError> {
    let v: serde_json::Value = serde_json::from_str(json).map_err(|e| err("transcript", e.line(), e.to_string()))?;
    let word = |w: &serde_json::Value| -> Option<Word> {
        Some(Word {
            start: w["start"].as_f64()?,
            end: w["end"].as_f64()?,
            text: w["text"].as_str().or(w["word"].as_str())?.trim().to_string(),
            emphasis: w["emphasis"].as_bool().unwrap_or(false),
        })
    };
    if let Some(segs) = v["segments"].as_array() {
        return Ok(segs
            .iter()
            .filter_map(|s| {
                let words: Vec<Word> =
                    s["words"].as_array().map(|a| a.iter().filter_map(word).collect()).unwrap_or_default();
                Some(Cue {
                    start: s["start"].as_f64()?,
                    end: s["end"].as_f64()?,
                    text: s["text"].as_str().unwrap_or("").trim().to_string(),
                    speaker: s["speaker"].as_str().map(str::to_string),
                    words,
                    ..Default::default()
                })
            })
            .collect());
    }
    let words: Vec<Word> = v["words"]
        .as_array()
        .ok_or_else(|| err("transcript", 1, "needs \"segments\" or \"words\""))?
        .iter()
        .filter_map(word)
        .collect();
    // one cue per sentence-ish group of up to 12 words
    let mut out = Vec::new();
    for chunk in words.chunks(12) {
        out.push(Cue {
            start: chunk[0].start,
            end: chunk[chunk.len() - 1].end,
            text: chunk.iter().map(|w| w.text.as_str()).collect::<Vec<_>>().join(" "),
            words: chunk.to_vec(),
            ..Default::default()
        });
    }
    Ok(out)
}

/// Words of a cue, timed from the source or spread over the cue by length.
pub fn timed_words(c: &Cue) -> Vec<Word> {
    if !c.words.is_empty() {
        return c.words.clone();
    }
    let ws: Vec<&str> = c.text.split_whitespace().collect();
    let total: usize = ws.iter().map(|w| w.chars().count() + 1).sum::<usize>().max(1);
    let mut t = c.start;
    ws.iter()
        .map(|w| {
            let d = (c.end - c.start) * (w.chars().count() + 1) as f64 / total as f64;
            let o = Word { start: t, end: t + d, text: w.to_string(), emphasis: false };
            t += d;
            o
        })
        .collect()
}

/// A page shown on screen: lines of words.
#[derive(Debug, Clone, PartialEq)]
pub struct Page {
    pub start: f64,
    pub end: f64,
    pub lines: Vec<Vec<Word>>,
    pub cue: usize,
}

impl Page {
    /// Text with one line per row.
    pub fn text(&self) -> String {
        self.lines
            .iter()
            .map(|l| l.iter().map(|w| w.text.as_str()).collect::<Vec<_>>().join(" "))
            .collect::<Vec<_>>()
            .join("\n")
    }
    /// All words in order.
    pub fn words(&self) -> Vec<&Word> {
        self.lines.iter().flatten().collect()
    }
}

/// Splits cues into pages of at most `max_lines` lines of `max_chars` characters and `max_words`
/// words: the first page starts with its
/// cue, later ones with their first word, and each lasts until the next page starts (the
/// last until the cue ends). `one_word` gives every word its own page (the one-word preset).
pub fn paginate(
    cues: &[Cue],
    max_words: Option<usize>,
    max_chars: usize,
    max_lines: usize,
    one_word: bool,
) -> Vec<Page> {
    let mut pages = Vec::new();
    for (ci, c) in cues.iter().enumerate() {
        let words: Vec<Word> = timed_words(c).into_iter().filter(|w| !w.text.is_empty()).collect();
        if one_word {
            for (i, w) in words.iter().enumerate() {
                let start = if i == 0 { c.start } else { c.start.max(w.start) };
                let end = words.get(i + 1).map(|n| n.start).unwrap_or(c.end);
                pages.push(Page { start, end, lines: vec![vec![w.clone()]], cue: ci });
            }
            continue;
        }
        let mut lines: Vec<Vec<Word>> = Vec::new();
        let mut cur: Vec<Word> = Vec::new();
        let mut len = 0;
        for w in words {
            let wl = w.text.chars().count();
            let over =
                (!cur.is_empty() && len + 1 + wl > max_chars.max(1)) || max_words.is_some_and(|m| cur.len() >= m);
            if over {
                lines.push(std::mem::take(&mut cur));
                len = 0;
            }
            len += wl + (!cur.is_empty()) as usize;
            cur.push(w);
        }
        if !cur.is_empty() {
            lines.push(cur);
        }
        let groups: Vec<&[Vec<Word>]> = lines.chunks(max_lines.max(1)).collect();
        for (i, g) in groups.iter().enumerate() {
            let first = |g: &[Vec<Word>]| g.first().and_then(|l| l.first()).map(|w| w.start).unwrap_or(c.start);
            let start = if i == 0 { c.start } else { first(g) };
            let end = groups.get(i + 1).map(|n| first(n)).unwrap_or(c.end);
            pages.push(Page { start, end: end.max(start), lines: g.to_vec(), cue: ci });
        }
    }
    pages
}

/// Masks common profanity as the first letter followed by asterisks.
pub fn profanity(s: &str) -> String {
    const WORDS: [&str; 12] = [
        "fuck",
        "fucking",
        "shit",
        "bitch",
        "bastard",
        "asshole",
        "cunt",
        "dick",
        "piss",
        "damn",
        "crap",
        "motherfucker",
    ];
    s.split(' ')
        .map(|w| {
            let core: String = w.chars().filter(|c| c.is_alphabetic()).collect::<String>().to_lowercase();
            if WORDS.contains(&core.as_str()) {
                w.chars().enumerate().map(|(i, c)| if i > 0 && c.is_alphabetic() { '*' } else { c }).collect()
            } else {
                w.to_string()
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

fn ts(t: f64, sep: char) -> String {
    let ms = (t.max(0.0) * 1000.0).round() as u64;
    format!("{:02}:{:02}:{:02}{sep}{:03}", ms / 3_600_000, ms / 60_000 % 60, ms / 1000 % 60, ms % 1000)
}

/// SRT sidecar text.
pub fn to_srt(cues: &[Cue]) -> String {
    cues.iter()
        .enumerate()
        .map(|(i, c)| format!("{}\n{} --> {}\n{}\n\n", i + 1, ts(c.start, ','), ts(c.end, ','), c.text))
        .collect()
}

/// WebVTT sidecar text (speakers as voice tags).
pub fn to_vtt(cues: &[Cue]) -> String {
    let mut s = String::from("WEBVTT\n\n");
    for c in cues {
        let body = match &c.speaker {
            Some(sp) => format!("<v {sp}>{}", c.text),
            None => c.text.clone(),
        };
        s.push_str(&format!("{} --> {}\n{}\n\n", ts(c.start, '.'), ts(c.end, '.'), body));
    }
    s
}

/// Caption presets.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Preset {
    Classic,
    BoxedLine,
    BoxedWord,
    OneWord,
    Karaoke,
    Highlight,
    Pop,
    Fade,
    Bounce,
    Slide,
    Typewriter,
    Enlarge,
    None,
}

impl Preset {
    /// Parses a preset name (unknown names are classic).
    pub fn parse(s: &str) -> Preset {
        use Preset::*;
        match s {
            "boxed-line" => BoxedLine,
            "boxed-word" => BoxedWord,
            "one-word" => OneWord,
            "karaoke" => Karaoke,
            "highlight" => Highlight,
            "pop" => Pop,
            "fade" => Fade,
            "bounce" => Bounce,
            "slide" => Slide,
            "typewriter" => Typewriter,
            "enlarge" => Enlarge,
            "none" => None,
            _ => Classic,
        }
    }
}

/// Page-level motion of the fade and slide presets at time `t`: (opacity, downward offset in em).
pub fn page_motion(preset: Preset, page: &Page, t: f64) -> (f64, f64) {
    match preset {
        Preset::Fade => (((t - page.start) / 0.15).min((page.end - t) / 0.15).min(1.0), 0.0),
        Preset::Slide => {
            let u = ((t - page.start) / 0.25).min(1.0);
            let e = 1.0 - (1.0 - u).powi(3);
            (e, 0.4 * (1.0 - e))
        }
        _ => (1.0, 0.0),
    }
}

fn bounce_out(x: f64) -> f64 {
    let (n1, d1) = (7.5625, 2.75);
    if x < 1.0 / d1 {
        n1 * x * x
    } else if x < 2.0 / d1 {
        let x = x - 1.5 / d1;
        n1 * x * x + 0.75
    } else if x < 2.5 / d1 {
        let x = x - 2.25 / d1;
        n1 * x * x + 0.9375
    } else {
        let x = x - 2.625 / d1;
        n1 * x * x + 0.984375
    }
}

/// Per-glyph effects of a caption page at time `t`, for a layout of `page.text()`
/// (caption presets). The active word is the last
/// word that has started (it stays active through the gap before the next). Karaoke lights
/// each word as a whole: from its start its fill cross-fades to `active` over its duration.
pub fn effects(preset: Preset, lay: &Layout, page: &Page, t: f64, active: &Paint) -> Vec<GlyphFx> {
    let words = page.words();
    let em = lay.styles.first().map(|s| s.size).unwrap_or(40.0);
    let current = if preset == Preset::OneWord { Some(0) } else { words.iter().rposition(|w| w.start <= t + 1e-9) };
    let (alpha, dy) = page_motion(preset, page, t);
    // word boxes on their lines: scale pivots (centre of the box, middle of the line box)
    let mut boxes: std::collections::HashMap<(usize, usize), (f64, f64, f64)> = Default::default();
    for g in &lay.glyphs {
        let lr = lay.lines.get(g.line).map(|l| l.rect).unwrap_or([0.0; 4]);
        let b = boxes.entry((g.line, g.word)).or_insert((f64::INFINITY, f64::NEG_INFINITY, lr[1] + lr[3] * 0.5));
        b.0 = b.0.min(g.x.min(g.x + g.advance));
        b.1 = b.1.max(g.x.max(g.x + g.advance));
    }
    lay.glyphs
        .iter()
        .map(|g| {
            let mut fx = GlyphFx { opacity: alpha, xf: Xf::translate(0.0, dy * em), ..Default::default() };
            let Some(w) = words.get(g.word) else { return fx };
            let wi = g.word;
            let started = t >= w.start - 1e-9;
            let is_cur = Some(wi) == current;
            let q = ((t - w.start) / (w.end - w.start).max(1e-3)).clamp(0.0, 1.0);
            let pivot = boxes.get(&(g.line, wi)).map(|b| ((b.0 + b.1) * 0.5, b.2)).unwrap_or((g.x, g.y));
            let scale_about = |s: f64| {
                Xf::translate(pivot.0, pivot.1 + dy * em).mul(&Xf::scale(s, s)).mul(&Xf::translate(-pivot.0, -pivot.1))
            };
            let light = |fx: &mut GlyphFx, k: f64| {
                fx.fill = Some(active.clone());
                fx.fill_mix = k;
            };
            match preset {
                Preset::Karaoke if started => light(&mut fx, if t >= w.end { 1.0 } else { q }),
                Preset::Highlight if is_cur => light(&mut fx, 1.0),
                Preset::BoxedWord if is_cur && !lay.chars.get(g.ch).is_some_and(|c| c.is_whitespace()) => {
                    fx.highlight = Some(crate::glyph::Highlight {
                        paint: active.clone(),
                        fraction: 1.0,
                        unit: wi,
                        pad: 0.12 * em,
                        radius: 0.18 * em,
                    })
                }
                Preset::Pop if is_cur => {
                    light(&mut fx, 1.0);
                    let u = ((t - w.start) / 0.25).clamp(0.0, 1.0);
                    fx.xf = scale_about(1.0 + 0.15 * libm::sin(std::f64::consts::PI * u));
                }
                Preset::Enlarge if is_cur => {
                    light(&mut fx, 1.0);
                    let u = ((t - w.start) / 0.1).min(1.0);
                    fx.xf = scale_about(1.0 + 0.2 * (1.0 - (1.0 - u).powi(3)));
                }
                Preset::Typewriter if !started => fx.opacity = 0.0,
                Preset::Bounce => {
                    if !started {
                        fx.opacity = 0.0;
                    } else {
                        let u = ((t - w.start) / 0.35).min(1.0);
                        if u < 1.0 {
                            fx.xf = Xf::translate(0.0, dy * em - 0.3 * em * (1.0 - bounce_out(u)));
                        }
                    }
                }
                _ => {}
            }
            fx
        })
        .collect()
}
