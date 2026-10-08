//! Every attribute of the shared effect bag that a kernel reads while rendering is declared for its type
//! (`sr_model::effect_attrs`), so the evaluator's warning for an attribute its type does not read (E19) is never wrong.

use super::common;
use common::*;

/// A valid sample value for each attribute of the bag, so that every branch that depends on one being set runs.
fn sample(name: &str) -> Option<&'static str> {
    Some(match name {
        "amount" | "intensity" | "radius" | "threshold" | "saturation" | "contrast" | "size" | "frequency"
        | "speed" | "temperature" | "tint" | "exposure" | "hue" | "relief" => "0.5",
        "brightness" | "inputBlack" | "outputBlack" | "centerX" | "centerY" => "0.1",
        "inputWhite" | "outputWhite" => "0.9",
        "offsetX" | "offsetY" => "3",
        "angle" => "30",
        "samples" | "levels" => "6",
        "seed" => "7",
        "tolerance" | "softness" | "spill" => "0.3",
        "color" | "paint" | "keyColor" => "#CC6633",
        "lift" | "gamma" | "gain" | "slope" | "offset" | "power" => "1.1 1 0.9",
        "curve" => "0,0 0.5,0.6 1,1",
        "channel" => "rgb",
        "falloff" => "smooth",
        "position" => "outside",
        "compositeOriginal" => "behind",
        "tonemapper" => "aces",
        "space" => "srgb",
        "source" => "other",
        "lights" => "lamp",
        "src" => "missing.cube",
        _ => return None,
    })
}

fn bag() -> Vec<&'static str> {
    // the attributes of effectType, read from the schema's declarations
    sr_model::xsd::COMPLEX_TYPES
        .iter()
        .find(|t| t.name == "effectType")
        .expect("effectType")
        .attrs
        .iter()
        .map(|a| a.name)
        .collect()
}

#[test]
fn kernels_read_only_declared_attributes() {
    let Some(_) = gpu() else { return };
    let names = bag();
    let mut failures = Vec::new();
    for (ty, _) in sr_model::effect_attrs::DECLARED {
        let attrs: String = names
            .iter()
            .filter(|n| !matches!(**n, "id" | "type" | "enabled" | "mix"))
            .filter_map(|n| sample(n).map(|v| format!(r#" {n}="{v}""#)))
            .collect();
        let xml = format!(
            r##"<scene version="1.1"><project width="32" height="32" fps="10" duration="1" background="#202020"/>
            <composition><shape id="other" shape="rect" x="2" y="2" width="12" height="12" fill="#FFFFFF"/>
            <shape id="n" shape="rect" x="8" y="8" width="16" height="16" fill="#FF8800" effects="fx"/></composition>
            <lights><light id="lamp" type="directional" intensity="1"/></lights>
            <effects><effect id="fx" type="{ty}"{attrs}/></effects></scene>"##
        );
        let doc = match sr_model::load_str(&xml, &sr_model::LoadOptions::without_assets()) {
            Ok(d) => d,
            Err(e) => {
                failures.push(format!("{ty}: the probe document is invalid: {e:?}"));
                continue;
            }
        };
        let (_, reads) = sr_gpu::vector::record_reads(|| render(&doc));
        for (element, name) in reads {
            if !matches!(element.as_str(), "effect" | "effectType") {
                continue;
            }
            if !names.contains(&name.as_str()) {
                continue;
            }
            if !sr_model::effect_attrs::reads(ty, &name) {
                failures.push(format!("{ty} reads {name}, which it does not declare"));
            }
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

/// The source of the kernels, read as text: a kernel arm that uses a value read up front for every type (`amount`, `sz`, ...)
/// must declare the attribute, which the recorder cannot see because the read happens before the arm.
const KERNELS: &str = include_str!("../../src/fx.rs");

/// The identifiers a kernel arm can use, and the attributes they stand for.
const UPFRONT: &[(&str, &[&str])] = &[
    ("r", &["radius"]),
    ("big", &["radius"]),
    ("intensity", &["intensity"]),
    ("threshold", &["threshold"]),
    ("amount", &["amount"]),
    ("amount_px", &["amount"]),
    ("sz", &["size"]),
    ("angle", &["angle"]),
    ("seed", &["seed"]),
    ("es", &["seed"]),
    ("speed", &["speed"]),
    ("center", &["centerX", "centerY"]),
];

/// `text` without string literals and `//` comments, as identifier tokens.
fn identifiers(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let (mut cur, mut in_str) = (String::new(), false);
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        if in_str {
            if c == '\\' {
                chars.next();
            } else if c == '"' {
                in_str = false;
            }
            continue;
        }
        if c == '"' {
            in_str = true;
        } else if c == '/' && chars.peek() == Some(&'/') {
            for d in chars.by_ref() {
                if d == '\n' {
                    break;
                }
            }
        } else if c.is_alphanumeric() || c == '_' {
            cur.push(c);
            continue;
        }
        if !cur.is_empty() {
            out.push(std::mem::take(&mut cur));
        }
    }
    out
}

/// The effect types an arm head names: a line starting with a quoted name or `| "name"`, continued until the line with `=>`.
fn arm_head(lines: &[&str], i: usize) -> Option<(Vec<String>, usize)> {
    let first = lines[i].trim_start();
    if !(first.starts_with('"') || first.starts_with("| \"")) {
        return None;
    }
    let mut head = String::new();
    let mut j = i;
    while j < lines.len() && j < i + 40 {
        head.push_str(lines[j]);
        head.push(' ');
        if lines[j].contains("=>") {
            let before = head.split("=>").next().unwrap();
            // a guard (`if ...`) ends the list of names
            let before = before.split(" if ").next().unwrap();
            let names: Vec<String> =
                before.split('|').map(|s| s.trim().trim_matches('"').to_string()).filter(|s| !s.is_empty()).collect();
            let all_effects = !names.is_empty()
                && names.iter().all(|n| {
                    !n.contains([' ', '(', ')', '{'])
                        && (sr_model::effect_attrs::declared(n).is_some() || n == "shader")
                });
            return all_effects.then_some((names, j));
        }
        j += 1;
    }
    None
}

#[test]
fn arms_declare_the_values_they_use_from_the_up_front_reads() {
    let lines: Vec<&str> = KERNELS.lines().collect();
    let indent = |l: &str| l.len() - l.trim_start().len();
    let mut failures = Vec::new();
    let mut covered = std::collections::BTreeSet::new();
    let mut i = 0;
    while i < lines.len() {
        let Some((names, head_end)) = arm_head(&lines, i) else {
            i += 1;
            continue;
        };
        let ind = indent(lines[i]);
        let mut body = lines[i..=head_end].to_vec();
        let mut j = head_end + 1;
        while j < lines.len() {
            let l = lines[j];
            let starts_arm = arm_head(&lines, j).is_some() && indent(l) <= ind;
            if starts_arm || (!l.trim().is_empty() && indent(l) < ind) {
                break;
            }
            body.push(l);
            j += 1;
        }
        // an arm that holds nested arms of its own (a shared head with a second `match` inside) answers only for the lines
        // before them: each nested arm answers for itself
        if let Some(k) = (head_end + 1..j).find(|k| arm_head(&lines, *k).is_some() && indent(lines[*k]) > ind) {
            body.truncate(k - i);
        }
        let tokens = identifiers(&body.join("\n"));
        for (ident, attrs) in UPFRONT {
            if tokens.iter().any(|t| t == ident) {
                for ty in &names {
                    for attr in *attrs {
                        if sr_model::effect_attrs::declared(ty).is_some_and(|d| !d.contains(attr)) {
                            failures.push(format!("{ty} uses `{ident}` (attribute {attr}) but does not declare it"));
                        }
                    }
                }
            }
        }
        covered.extend(names);
        i = j;
    }
    // every declared type has an arm in the kernels, except those the render passes handle themselves (their reads are
    // explicit, so the recorder sees them)
    let outside = ["displacement-map", "echo", "pixel-motion-blur", "posterize-time"];
    let missing: Vec<&str> = sr_model::effect_attrs::DECLARED
        .iter()
        .map(|(t, _)| *t)
        .filter(|t| !covered.contains(*t) && !outside.contains(t))
        .collect();
    assert!(missing.is_empty(), "the scan did not find arms for: {missing:?}");
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}
