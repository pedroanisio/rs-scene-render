//! MaterialX pattern graphs compiled into bounded, reusable expression trees.
use roxmltree::Node;
use std::{collections::HashMap, path::Path, sync::Arc};

// A collapsed input range maps to its lower output endpoint. Preserve the
// denominator sign for descending ranges instead of replacing it with epsilon.
fn unit_range(value: f32, low: f32, high: f32) -> f32 {
    if high == low {
        0.0
    } else {
        (value - low) / (high - low)
    }
}

fn value_width(ty: Option<&str>, fallback: usize) -> usize {
    match ty {
        Some("float" | "integer" | "boolean") => 1,
        Some("vector2") => 2,
        Some("vector3" | "color3") => 3,
        Some("vector4" | "color4") => 4,
        _ => fallback.clamp(1, 4),
    }
}

use crate::sampling::{AddressMode, TextureFilter, TextureSampler as Sampler};
impl Sampler {
    fn from_node(node: Node<'_, '_>) -> Result<Self, String> {
        let value = |name, default| -> Result<&str, String> {
            node.children().find(|n| n.attribute("name") == Some(name)).map_or(Ok(default), |n| {
                n.attribute("value").ok_or_else(|| format!("MaterialX {name} must be a uniform value"))
            })
        };
        let address = |name| -> Result<AddressMode, String> {
            match value(name, "periodic")? {
                "periodic" => Ok(AddressMode::Periodic),
                "clamp" => Ok(AddressMode::Clamp),
                "mirror" => Ok(AddressMode::Mirror),
                "constant" => Ok(AddressMode::Constant),
                other => Err(format!("unknown MaterialX {name}: {other}")),
            }
        };
        let filter = match value("filtertype", "linear")? {
            "closest" => TextureFilter::Closest,
            "linear" => TextureFilter::Linear,
            "cubic" => TextureFilter::Cubic,
            other => return Err(format!("unknown MaterialX filtertype: {other}")),
        };
        Ok(Self { address: [address("uaddressmode")?, address("vaddressmode")?], filter, border: [0.0; 4] })
    }
}

#[derive(Clone)]
pub(crate) struct Expr {
    op: String,
    width: usize,
    inputs: HashMap<String, Arc<Expr>>,
    value: [f32; 4],
    image: Option<Arc<image::Rgba32FImage>>,
    channels: String,
    sampler: Sampler,
    pub varying: bool,
    pub size: [u32; 2],
}

impl Expr {
    fn constant(value: [f32; 4], width: usize) -> Arc<Self> {
        Arc::new(Self {
            op: "constant".into(),
            width,
            inputs: HashMap::new(),
            value,
            image: None,
            channels: String::new(),
            sampler: Sampler::default(),
            varying: false,
            size: [1; 2],
        })
    }
    pub fn eval(&self, uv: [f32; 2]) -> [f32; 4] {
        self.eval_cached(uv, &mut HashMap::new())
    }
    fn eval_cached(&self, uv: [f32; 2], cache: &mut HashMap<*const Expr, [f32; 4]>) -> [f32; 4] {
        let key = self as *const Expr;
        if let Some(value) = cache.get(&key) {
            return *value;
        }
        let mut input =
            |name: &str, default: f32| self.inputs.get(name).map_or([default; 4], |v| v.eval_cached(uv, cache));
        let a = input("in1", 0.0);
        let b = input(
            "in2",
            if matches!(self.op.as_str(), "multiply" | "divide" | "modulo" | "power") { 1.0 } else { 0.0 },
        );
        let width = self.inputs.get("in").map_or(self.width, |v| v.width);
        let pair_width = self.inputs.get("in1").map_or(self.width, |v| v.width);
        let x = input("in", 0.0);
        let unary = |f: fn(f32) -> f32| x.map(f);
        let value = match self.op.as_str() {
            "constant" => {
                if self.inputs.contains_key("value") {
                    input("value", 0.0)
                } else {
                    self.value
                }
            }
            "texcoord" => [uv[0], uv[1], 0.0, 1.0],
            "image" | "tiledimage" => {
                let image = self.image.as_ref().expect("compiled image");
                let mut p = if self.inputs.contains_key("texcoord") {
                    input("texcoord", 0.0)
                } else {
                    [uv[0], uv[1], 0.0, 1.0]
                };
                if self.op == "tiledimage" {
                    let tiling = input("uvtiling", 1.0);
                    let offset = input("uvoffset", 0.0);
                    let image_size = input("realworldimagesize", 1.0);
                    let tile_size = input("realworldtilesize", 1.0);
                    for i in 0..2 {
                        p[i] = if image_size[i] == 0.0 {
                            0.0
                        } else {
                            (p[i] * tiling[i] - offset[i]) / image_size[i] * tile_size[i]
                        };
                    }
                }
                Sampler { border: input("default", 0.0), ..self.sampler }.sample(
                    [image.width(), image.height()],
                    [p[0], p[1]],
                    |x, y| image.get_pixel(x, y).0,
                )
            }
            "add" => std::array::from_fn(|i| a[i] + b[i]),
            "subtract" => std::array::from_fn(|i| a[i] - b[i]),
            "multiply" => std::array::from_fn(|i| a[i] * b[i]),
            "divide" => std::array::from_fn(|i| if b[i] == 0.0 { 0.0 } else { a[i] / b[i] }),
            "modulo" => std::array::from_fn(|i| if b[i] == 0.0 { 0.0 } else { a[i].rem_euclid(b[i]) }),
            "power" => std::array::from_fn(|i| a[i].max(0.0).powf(b[i])),
            "min" => std::array::from_fn(|i| a[i].min(b[i])),
            "max" => std::array::from_fn(|i| a[i].max(b[i])),
            "absval" => unary(f32::abs),
            "floor" => unary(f32::floor),
            "ceil" => unary(f32::ceil),
            "sqrt" => x.map(|v| v.max(0.0).sqrt()),
            "sin" => unary(f32::sin),
            "cos" => unary(f32::cos),
            "tan" => unary(f32::tan),
            "exp" => unary(f32::exp),
            "ln" => x.map(|v| v.max(1e-20).ln()),
            "sign" => unary(f32::signum),
            "round" => unary(f32::round),
            "invert" => {
                let amount = input("amount", 1.0);
                std::array::from_fn(|i| amount[i] - x[i])
            }
            "clamp" => {
                let lo = input("low", 0.0);
                let hi = input("high", 1.0);
                std::array::from_fn(|i| x[i].max(lo[i]).min(hi[i]))
            }
            "mix" => {
                let fg = input("fg", 0.0);
                let bg = input("bg", 0.0);
                let m = input("mix", 0.5);
                std::array::from_fn(|i| bg[i] * (1.0 - m[i]) + fg[i] * m[i])
            }
            "remap" | "range" => {
                let lo = input("inlow", 0.0);
                let hi = input("inhigh", 1.0);
                let outlo = input("outlow", 0.0);
                let outhi = input("outhigh", 1.0);
                let gamma = input("gamma", 1.0);
                let clamp = self.op == "range" && input("doclamp", 0.0)[0] != 0.0;
                std::array::from_fn(|i| {
                    let mut t = unit_range(x[i], lo[i], hi[i]);
                    if self.op == "range" && gamma[i] != 1.0 && t != 0.0 {
                        let exponent = if gamma[i] == 0.0 { 0.0 } else { 1.0 / gamma[i] };
                        t = t.abs().powf(exponent).copysign(t);
                    }
                    let value = outlo[i] + t * (outhi[i] - outlo[i]);
                    if clamp {
                        value.clamp(outlo[i].min(outhi[i]), outlo[i].max(outhi[i]))
                    } else {
                        value
                    }
                })
            }
            "smoothstep" => {
                let lo = input("low", 0.0);
                let hi = input("high", 1.0);
                std::array::from_fn(|i| {
                    let t = (unit_range(x[i], lo[i], hi[i])).clamp(0.0, 1.0);
                    t * t * (3.0 - 2.0 * t)
                })
            }
            "normalize" => {
                let length = x[..width].iter().map(|v| v * v).sum::<f32>().sqrt().max(1e-20);
                std::array::from_fn(|i| if i < width { x[i] / length } else { x[i] })
            }
            "magnitude" => [x[..width].iter().map(|v| v * v).sum::<f32>().sqrt(); 4],
            "dotproduct" => [a[..pair_width].iter().zip(&b).map(|(a, b)| a * b).sum(); 4],
            "crossproduct" => [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0], 1.0],
            "combine2" | "combine3" | "combine4" => {
                std::array::from_fn(|i| input(&format!("in{}", i + 1), if i == 3 { 1.0 } else { 0.0 })[0])
            }
            "swizzle" | "extract" => {
                let index = input("index", 0.0)[0].clamp(0.0, 3.0) as usize;
                if self.op == "extract" {
                    return [x[index]; 4];
                }
                let chars: Vec<char> = self.channels.chars().collect();
                std::array::from_fn(|i| match chars.get(i).copied().unwrap_or(if i == 3 { '1' } else { '0' }) {
                    'r' | 'x' => x[0],
                    'g' | 'y' => x[1],
                    'b' | 'z' => x[2],
                    'a' | 'w' => x[3],
                    '1' => 1.0,
                    _ => 0.0,
                })
            }
            "convert" => x,
            "normalmap" => {
                let scale = input("scale", 1.0)[0];
                let v = [(x[0] * 2.0 - 1.0) * scale, (x[1] * 2.0 - 1.0) * scale, x[2] * 2.0 - 1.0];
                let length = v.iter().map(|x| x * x).sum::<f32>().sqrt().max(1e-20);
                [v[0] / length, v[1] / length, v[2] / length, 1.0]
            }
            _ => unreachable!("operator validated at compilation"),
        };
        cache.insert(key, value);
        value
    }
    fn output_sampler(&self, factor: [f32; 4], normal: bool) -> Option<Sampler> {
        let mut sampler = self.sampler_cached(&mut HashMap::new(), &mut HashMap::new())?;
        for (i, f) in factor.iter().enumerate() {
            if normal && i < 3 {
                sampler.border[i] = sampler.border[i] * 0.5 + 0.5;
            }
            sampler.border[i] /= f;
        }
        Some(sampler)
    }

    fn sampler_cached(
        &self,
        cache: &mut HashMap<*const Expr, Option<Sampler>>,
        border_values: &mut HashMap<*const Expr, [f32; 4]>,
    ) -> Option<Sampler> {
        let key = self as *const Expr;
        if let Some(sampler) = cache.get(&key) {
            return *sampler;
        }
        // Cache absence too: mixed/coordinate graphs can share subgraphs just
        // as heavily as graphs whose image samplers are compatible.
        let result = if self.image.is_some()
            && self.inputs.get("texcoord").is_none_or(|v| v.op == "texcoord")
            && !self.inputs.contains_key("uvtiling")
            && !self.inputs.contains_key("uvoffset")
        {
            let mut sampler = self.sampler;
            sampler.border = self.inputs.get("default").map_or([0.0; 4], |v| v.eval([0.0; 2]));
            Some(sampler)
        } else if self.image.is_none() {
            let mut samplers = self.inputs.values().filter_map(|v| v.sampler_cached(cache, border_values));
            let common = samplers.next().filter(|first| samplers.all(|s| s == *first));
            common.map(|mut sampler| {
                sampler.border = self.eval_cached([-1.0; 2], border_values);
                sampler
            })
        } else {
            None
        };
        cache.insert(key, result);
        result
    }

    pub fn bake(&self, normal: bool) -> (crate::Texture, [f32; 4]) {
        let [w, h] = self.size;
        let value = |x, y| {
            let mut v = self.eval([(x as f32 + 0.5) / w as f32, (y as f32 + 0.5) / h as f32]);
            if normal {
                for channel in &mut v[..3] {
                    *channel = *channel * 0.5 + 0.5;
                }
            }
            v
        };
        // Two passes avoid retaining a second, float-sized copy of the output.
        let mut factor = [1.0f32; 4];
        for y in 0..h {
            for x in 0..w {
                let v = value(x, y);
                for i in 0..4 {
                    factor[i] = factor[i].max(v[i]);
                }
            }
        }
        let mut rgba = Vec::with_capacity(w as usize * h as usize * 4);
        for y in 0..h {
            for x in 0..w {
                let v = value(x, y);
                rgba.extend(std::array::from_fn::<_, 4, _>(|i| {
                    (v[i] / factor[i]).clamp(0.0, 1.0).mul_add(255.0, 0.5) as u8
                }));
            }
        }
        (
            crate::Texture { width: w, height: h, rgba, srgb: false, sampler: self.output_sampler(factor, normal) },
            factor,
        )
    }
}

const GRAPH_MEMORY_BYTES: u64 = 512 * 1024 * 1024;

#[cfg(test)]
fn load_image(path: &Path) -> Result<image::Rgba32FImage, String> {
    let mut remaining = GRAPH_MEMORY_BYTES;
    load_image_with_budget(path, &mut remaining)
}

fn load_image_with_budget(path: &Path, remaining: &mut u64) -> Result<image::Rgba32FImage, String> {
    use image::ImageDecoder;
    let error = |e| format!("{}: {e}", path.display());
    let mut reader = image::ImageReader::open(path).map_err(error)?;
    let mut limits = image::Limits::default();
    limits.max_alloc = Some((*remaining).min(256 * 1024 * 1024));
    limits.max_image_width = Some(32768);
    limits.max_image_height = Some(32768);
    reader.limits(limits);
    let decoder = reader.into_decoder().map_err(|e| format!("{}: {e}", path.display()))?;
    let (w, h) = decoder.dimensions();
    // Conservative upper bound also accounts for the float conversion's overlap
    // with decoded pixels. Reject before decoding or allocating output pixels.
    let float_bytes = u64::from(w.min(4096)) * u64::from(h.min(4096)) * 16;
    if decoder.total_bytes() > 256 * 1024 * 1024 {
        return Err("MaterialX image exceeds its 256 MiB decode budget".into());
    }
    if float_bytes.saturating_add(decoder.total_bytes()) > *remaining {
        return Err("MaterialX graph exceeds its 512 MiB aggregate image/bake memory budget".into());
    }
    let mut image = image::DynamicImage::from_decoder(decoder).map_err(|e| format!("{}: {e}", path.display()))?;
    // Resize in the decoded representation first. Expanding an 8-bit source to
    // four float channels before this step can allocate gigabytes unnecessarily.
    if image.width() > 4096 || image.height() > 4096 {
        image = image.thumbnail(4096, 4096);
    }
    let image = image.into_rgba32f();
    *remaining -= u64::from(image.width()) * u64::from(image.height()) * 16;
    Ok(image)
}

pub(crate) struct Compiler<'a, 'input> {
    root: Node<'a, 'input>,
    base: &'a Path,
    cache: HashMap<roxmltree::NodeId, Arc<Expr>>,
    visiting: Vec<roxmltree::NodeId>,
    remaining: u64,
}
impl<'a, 'input> Compiler<'a, 'input> {
    pub fn new(root: Node<'a, 'input>, base: &'a Path) -> Self {
        Self { root, base, cache: HashMap::new(), visiting: Vec::new(), remaining: GRAPH_MEMORY_BYTES }
    }
    #[cfg(test)]
    fn with_budget(root: Node<'a, 'input>, base: &'a Path, bytes: u64) -> Self {
        Self { remaining: bytes, ..Self::new(root, base) }
    }
    pub fn reserve_bake(&mut self, size: [u32; 2]) -> Result<(), String> {
        let bytes = u64::from(size[0]) * u64::from(size[1]) * 4;
        self.remaining = self
            .remaining
            .checked_sub(bytes)
            .ok_or("MaterialX graph exceeds its 512 MiB aggregate image/bake memory budget")?;
        Ok(())
    }
    fn named(&self, from: Node<'a, 'input>, name: &str) -> Option<Node<'a, 'input>> {
        from.ancestors().skip(1).find_map(|scope| scope.children().find(|n| n.attribute("name") == Some(name)))
    }
    pub fn input(&mut self, input: Node<'a, 'input>) -> Result<Arc<Expr>, String> {
        if let Some(value) = input.attribute("value") {
            if input.attribute("type") == Some("boolean") {
                return match value {
                    "true" | "1" => Ok(Expr::constant([1.0; 4], 1)),
                    "false" | "0" => Ok(Expr::constant([0.0; 4], 1)),
                    _ => Err(format!("invalid MaterialX boolean {value}")),
                };
            }
            let values: Vec<f32> = value
                .split(|c: char| c == ',' || c.is_whitespace())
                .filter(|s| !s.is_empty())
                .map(|s| s.parse().map_err(|_| format!("invalid MaterialX number {s}")))
                .collect::<Result<_, _>>()?;
            if values.is_empty() {
                return Err("empty MaterialX value".into());
            }
            return Ok(Expr::constant(
                std::array::from_fn(|i| {
                    values.get(i).copied().unwrap_or(if values.len() == 1 {
                        values[0]
                    } else if i == 3 {
                        1.0
                    } else {
                        0.0
                    })
                }),
                value_width(input.attribute("type"), values.len()),
            ));
        }
        if let Some(graph) = input.attribute("nodegraph") {
            let graph = self.named(input, graph).ok_or_else(|| format!("missing nodegraph {graph}"))?;
            let output = input.attribute("output");
            let output = graph
                .children()
                .find(|n| n.has_tag_name("output") && output.is_none_or(|name| n.attribute("name") == Some(name)))
                .ok_or("missing nodegraph output")?;
            return self.connected(output);
        }
        self.connected(input)
    }
    fn connected(&mut self, input: Node<'a, 'input>) -> Result<Arc<Expr>, String> {
        let name = input.attribute("nodename").ok_or("MaterialX connection has no value or node")?;
        let node = self.named(input, name).ok_or_else(|| format!("missing MaterialX node {name}"))?;
        self.node(node)
    }
    fn node(&mut self, node: Node<'a, 'input>) -> Result<Arc<Expr>, String> {
        if let Some(expr) = self.cache.get(&node.id()) {
            return Ok(expr.clone());
        }
        if self.cache.len() + self.visiting.len() >= 1024 {
            return Err("MaterialX graph exceeds 1024 connected nodes".into());
        }
        if self.visiting.contains(&node.id()) {
            return Err("MaterialX graph contains a cycle".into());
        }
        if self.visiting.len() >= 64 {
            return Err("MaterialX graph exceeds 64 nested nodes".into());
        }
        self.visiting.push(node.id());
        let op = node.tag_name().name();
        let mut expression = Expr {
            op: op.into(),
            width: value_width(node.attribute("type"), 3),
            inputs: HashMap::new(),
            value: [0.0; 4],
            image: None,
            channels: String::new(),
            sampler: Sampler::default(),
            varying: false,
            size: [1; 2],
        };
        for input in node.children().filter(|n| n.has_tag_name("input")) {
            let name = input.attribute("name").unwrap_or("");
            if matches!(input.attribute("type"), Some("filename" | "string")) || name == "file" || name == "channels" {
                continue;
            }
            let value = self.input(input)?;
            expression.varying |= value.varying;
            for i in 0..2 {
                expression.size[i] = expression.size[i].max(value.size[i]);
            }
            expression.inputs.insert(name.into(), value);
        }
        match op {
            "constant" => {
                expression.value = expression.inputs.get("value").ok_or("constant without value")?.eval([0.0; 2])
            }
            "image" | "tiledimage" => {
                expression.sampler = Sampler::from_node(node)?;
                let file = node
                    .children()
                    .find(|n| n.attribute("name") == Some("file"))
                    .and_then(|n| n.attribute("value"))
                    .ok_or("image without filename")?;
                let prefix = node
                    .ancestors()
                    .filter_map(|n| n.attribute("fileprefix"))
                    .collect::<Vec<_>>()
                    .into_iter()
                    .rev()
                    .collect::<String>();
                let path = self.base.join(format!("{prefix}{file}"));
                let mut image = load_image_with_budget(&path, &mut self.remaining)?;
                let srgb = node
                    .attribute("colorspace")
                    .or_else(|| self.root.attribute("colorspace"))
                    .map_or(node.attribute("type").is_some_and(|t| t.starts_with("color")), |s| {
                        matches!(s, "srgb_texture" | "srgb")
                    });
                if srgb {
                    for p in image.pixels_mut() {
                        for c in &mut p.0[..3] {
                            *c = crate::material::srgb_to_linear(*c);
                        }
                    }
                }
                expression.size = [image.width(), image.height()];
                expression.image = Some(Arc::new(image));
                expression.varying = true;
            }
            "texcoord" => {
                expression.varying = true;
                expression.size = [256; 2];
            }
            "add" | "subtract" | "multiply" | "divide" | "modulo" | "power" | "min" | "max" | "absval" | "floor"
            | "ceil" | "sqrt" | "sin" | "cos" | "tan" | "exp" | "ln" | "sign" | "round" | "invert" | "clamp"
            | "mix" | "remap" | "range" | "smoothstep" | "normalize" | "magnitude" | "dotproduct" | "crossproduct"
            | "combine2" | "combine3" | "combine4" | "extract" | "convert" | "normalmap" => {}
            "swizzle" => {
                expression.channels = node
                    .children()
                    .find(|n| n.attribute("name") == Some("channels"))
                    .and_then(|n| n.attribute("value"))
                    .unwrap_or("rgba")
                    .into()
            }
            _ => return Err(format!("unknown MaterialX pattern node <{op}>")),
        }
        self.visiting.pop();
        let expression = Arc::new(expression);
        self.cache.insert(node.id(), expression.clone());
        Ok(expression)
    }
}

#[cfg(all(test, target_os = "linux"))]
mod resource_tests {
    use super::*;
    #[test]
    fn large_images_resize_within_memory_budget() {
        const CHILD_INPUT: &str = "SR_MATERIALX_MEMORY_TEST_INPUT";
        if let Some(path) = std::env::var_os(CHILD_INPUT) {
            let image = load_image(Path::new(&path)).unwrap();
            assert_eq!(image.dimensions(), (4096, 4096));
            assert_eq!(image.get_pixel(0, 0).0, [1.0; 4]);
            return;
        }
        use std::io::Write;
        let path = std::env::temp_dir().join(format!("sr-mtlx-memory-{}.ppm", std::process::id()));
        let mut file = std::fs::File::create(&path).unwrap();
        write!(file, "P6\n8192 8192\n255\n").unwrap();
        let row = vec![255u8; 8192 * 3];
        for _ in 0..8192 {
            file.write_all(&row).unwrap();
        }
        drop(file);
        // Old code allocates a 1 GiB float buffer before shrinking; the bounded
        // decode plus integer thumbnail fits this 900 MiB address-space limit.
        let output = std::process::Command::new("sh")
            .args(["-c", "ulimit -c 0; ulimit -v 921600; exec \"$@\"", "memory-test"])
            .arg(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "mtlx_graph::resource_tests::large_images_resize_within_memory_budget",
                "--test-threads=1",
            ])
            .env(CHILD_INPUT, &path)
            .output()
            .unwrap();
        std::fs::remove_file(path).unwrap();
        assert!(
            output.status.success(),
            "{}\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    }
}

#[cfg(test)]
mod budget_tests {
    use super::*;
    #[test]
    fn shared_dag_sampler_planning_is_bounded() {
        const CHILD: &str = "SR_MTLX_DAG_PROBE";
        if std::env::var_os(CHILD).is_some() {
            let base = std::env::temp_dir().join(format!("sr-mtlx-dag-{}", std::process::id()));
            std::fs::create_dir_all(&base).unwrap();
            std::fs::write(base.join("a.ppm"), b"P6\n1 1\n255\n\xff\0\0").unwrap();
            let mut text = String::from(
                r#"<materialx><image name="n0" type="color3"><input name="file" type="filename" value="a.ppm"/><input name="filtertype" type="string" value="closest"/></image>"#,
            );
            for i in 1..=28 {
                text.push_str(&format!(r#"<add name="n{i}" type="color3"><input name="in1" nodename="n{}"/><input name="in2" nodename="n{}"/></add>"#, i-1, i-1));
            }
            text.push_str(r#"<input nodename="n28"/></materialx>"#);
            let doc = roxmltree::Document::parse(&text).unwrap();
            let root = doc.root_element();
            let expression =
                Compiler::new(root, &base).input(root.children().find(|n| n.has_tag_name("input")).unwrap()).unwrap();
            let (texture, factor) = expression.bake(false);
            assert_eq!(texture.sampler.unwrap().filter, TextureFilter::Closest);
            assert_eq!(factor[0], 268435456.0);
            std::fs::remove_dir_all(base).unwrap();
            return;
        }
        let mut child = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "mtlx_graph::budget_tests::shared_dag_sampler_planning_is_bounded"])
            .env(CHILD, "1")
            .spawn()
            .unwrap();
        let start = std::time::Instant::now();
        loop {
            if let Some(status) = child.try_wait().unwrap() {
                assert!(status.success());
                break;
            }
            if start.elapsed() > std::time::Duration::from_secs(5) {
                child.kill().unwrap();
                child.wait().unwrap();
                panic!("sampler planning revisits shared DAG nodes");
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
    }

    #[test]
    fn graph_rejects_images_exceeding_aggregate_budget() {
        let base = std::env::temp_dir().join(format!("sr-mtlx-budget-{}", std::process::id()));
        std::fs::create_dir_all(&base).unwrap();
        std::fs::write(base.join("a.ppm"), b"P6\n2 1\n255\n\xff\0\0\0\xff\0").unwrap();
        std::fs::write(base.join("b.ppm"), b"P6\n2 1\n255\n\0\0\xff\xff\xff\0").unwrap();
        let xml = roxmltree::Document::parse(r#"<materialx><image name="a"><input name="file" type="filename" value="a.ppm"/></image><image name="b"><input name="file" type="filename" value="b.ppm"/></image><input nodename="a"/><input nodename="b"/></materialx>"#).unwrap();
        let root = xml.root_element();
        let mut graph = Compiler::with_budget(root, &base, 64);
        let inputs: Vec<_> = root.children().filter(|n| n.has_tag_name("input")).collect();
        assert!(graph.input(inputs[0]).is_ok());
        // The same DAG node is shared, not charged repeatedly.
        assert!(graph.input(inputs[0]).is_ok());
        let error = graph.input(inputs[1]).err().expect("aggregate budget must reject the second decode");
        assert!(error.contains("budget"), "{error}");
        std::fs::remove_dir_all(base).unwrap();
    }
}
