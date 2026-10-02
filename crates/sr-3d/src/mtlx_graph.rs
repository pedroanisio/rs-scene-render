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

#[derive(Clone, Copy, Default)]
enum Address {
    #[default]
    Periodic,
    Clamp,
    Mirror,
    Constant,
}
impl Address {
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
#[derive(Clone, Copy, Default)]
enum Filter {
    Closest,
    #[default]
    Linear,
    Cubic,
}
#[derive(Clone, Copy, Default)]
struct Sampler {
    address: [Address; 2],
    filter: Filter,
}
impl Sampler {
    fn from_node(node: Node<'_, '_>) -> Result<Self, String> {
        let value = |name, default| -> Result<&str, String> {
            node.children().find(|n| n.attribute("name") == Some(name)).map_or(Ok(default), |n| {
                n.attribute("value").ok_or_else(|| format!("MaterialX {name} must be a uniform value"))
            })
        };
        let address = |name| -> Result<Address, String> {
            match value(name, "periodic")? {
                "periodic" => Ok(Address::Periodic),
                "clamp" => Ok(Address::Clamp),
                "mirror" => Ok(Address::Mirror),
                "constant" => Ok(Address::Constant),
                other => Err(format!("unknown MaterialX {name}: {other}")),
            }
        };
        let filter = match value("filtertype", "linear")? {
            "closest" => Filter::Closest,
            "linear" => Filter::Linear,
            "cubic" => Filter::Cubic,
            other => return Err(format!("unknown MaterialX filtertype: {other}")),
        };
        Ok(Self { address: [address("uaddressmode")?, address("vaddressmode")?], filter })
    }
    fn sample(&self, image: &image::Rgba32FImage, uv: [f32; 2], default: [f32; 4]) -> [f32; 4] {
        if uv.iter().any(|v| !v.is_finite()) {
            return default;
        }
        let (w, h) = image.dimensions();
        let get = |x, y| match (self.address[0].index(x, w), self.address[1].index(y, h)) {
            (Some(x), Some(y)) => image.get_pixel(x, y).0,
            _ => default,
        };
        // Reduce large coordinates before conversion so neighbour offsets cannot overflow.
        let coord = |v: f32, mode: Address| match mode {
            Address::Periodic => v.rem_euclid(1.0),
            Address::Mirror => v.rem_euclid(2.0),
            Address::Clamp => v.clamp(0.0, 1.0),
            Address::Constant => v.clamp(-1.0, 2.0),
        };
        let px = coord(uv[0], self.address[0]) * w as f32 - 0.5;
        let py = coord(uv[1], self.address[1]) * h as f32 - 0.5;
        let (x, y) = (px.floor() as i64, py.floor() as i64);
        let (u, v) = (px - px.floor(), py - py.floor());
        match self.filter {
            Filter::Closest => get((px + 0.5).floor() as i64, (py + 0.5).floor() as i64),
            Filter::Linear => {
                let (a, b, c, d) = (get(x, y), get(x + 1, y), get(x, y + 1), get(x + 1, y + 1));
                std::array::from_fn(|i| (a[i] * (1.0 - u) + b[i] * u) * (1.0 - v) + (c[i] * (1.0 - u) + d[i] * u) * v)
            }
            Filter::Cubic => {
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
                self.sampler.sample(image, [p[0], p[1]], input("default", 0.0))
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
    pub fn bake(&self, normal: bool) -> (crate::Texture, [f32; 4]) {
        let [w, h] = self.size;
        let mut values = Vec::with_capacity((w * h) as usize);
        let mut factor = [1.0f32; 4];
        for y in 0..h {
            for x in 0..w {
                let mut v = self.eval([(x as f32 + 0.5) / w as f32, (y as f32 + 0.5) / h as f32]);
                if normal {
                    for channel in &mut v[..3] {
                        *channel = *channel * 0.5 + 0.5;
                    }
                }
                for i in 0..4 {
                    factor[i] = factor[i].max(v[i]);
                }
                values.push(v);
            }
        }
        let rgba = values
            .into_iter()
            .flat_map(|v| {
                std::array::from_fn::<_, 4, _>(|i| (v[i] / factor[i]).clamp(0.0, 1.0).mul_add(255.0, 0.5) as u8)
            })
            .collect();
        (crate::Texture { width: w, height: h, rgba, srgb: false }, factor)
    }
}

fn load_image(path: &Path) -> Result<image::Rgba32FImage, String> {
    let error = |e| format!("{}: {e}", path.display());
    let mut reader = image::ImageReader::open(path).map_err(error)?;
    let mut limits = image::Limits::default();
    limits.max_alloc = Some(256 * 1024 * 1024);
    limits.max_image_width = Some(32768);
    limits.max_image_height = Some(32768);
    reader.limits(limits);
    let mut image = reader.decode().map_err(|e| format!("{}: {e}", path.display()))?;
    // Resize in the decoded representation first. Expanding an 8-bit source to
    // four float channels before this step can allocate gigabytes unnecessarily.
    if image.width() > 4096 || image.height() > 4096 {
        image = image.thumbnail(4096, 4096);
    }
    Ok(image.into_rgba32f())
}

pub(crate) struct Compiler<'a, 'input> {
    root: Node<'a, 'input>,
    base: &'a Path,
    cache: HashMap<roxmltree::NodeId, Arc<Expr>>,
    visiting: Vec<roxmltree::NodeId>,
}
impl<'a, 'input> Compiler<'a, 'input> {
    pub fn new(root: Node<'a, 'input>, base: &'a Path) -> Self {
        Self { root, base, cache: HashMap::new(), visiting: Vec::new() }
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
                let mut image = load_image(&path)?;
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
