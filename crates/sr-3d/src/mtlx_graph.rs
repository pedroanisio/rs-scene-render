//! MaterialX pattern graphs compiled into bounded, reusable expression trees.
use roxmltree::Node;
use std::{collections::HashMap, path::Path, sync::Arc};

#[derive(Clone)]
pub(crate) struct Expr {
    op: String,
    inputs: HashMap<String, Arc<Expr>>,
    value: [f32; 4],
    image: Option<Arc<image::Rgba32FImage>>,
    channels: String,
    pub varying: bool,
    pub size: [u32; 2],
}

impl Expr {
    fn constant(value: [f32; 4]) -> Arc<Self> {
        Arc::new(Self {
            op: "constant".into(),
            inputs: HashMap::new(),
            value,
            image: None,
            channels: String::new(),
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
        let b = input("in2", 0.0);
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
                let p = if self.inputs.contains_key("texcoord") {
                    input("texcoord", 0.0)
                } else {
                    [uv[0], uv[1], 0.0, 1.0]
                };
                let (w, h) = image.dimensions();
                let (px, py) = (p[0].rem_euclid(1.0) * w as f32 - 0.5, p[1].rem_euclid(1.0) * h as f32 - 0.5);
                let (x, y) = (px.floor() as i64, py.floor() as i64);
                let get =
                    |x: i64, y: i64| image.get_pixel(x.rem_euclid(w as i64) as u32, y.rem_euclid(h as i64) as u32).0;
                let (a, b, c, d) = (get(x, y), get(x + 1, y), get(x, y + 1), get(x + 1, y + 1));
                let (u, v) = (px - px.floor(), py - py.floor());
                std::array::from_fn(|i| (a[i] * (1.0 - u) + b[i] * u) * (1.0 - v) + (c[i] * (1.0 - u) + d[i] * u) * v)
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
                std::array::from_fn(|i| outlo[i] + (x[i] - lo[i]) / (hi[i] - lo[i]).max(1e-20) * (outhi[i] - outlo[i]))
            }
            "smoothstep" => {
                let lo = input("low", 0.0);
                let hi = input("high", 1.0);
                std::array::from_fn(|i| {
                    let t = ((x[i] - lo[i]) / (hi[i] - lo[i]).max(1e-20)).clamp(0.0, 1.0);
                    t * t * (3.0 - 2.0 * t)
                })
            }
            "normalize" => {
                let length = x[..3].iter().map(|v| v * v).sum::<f32>().sqrt().max(1e-20);
                [x[0] / length, x[1] / length, x[2] / length, x[3]]
            }
            "magnitude" => [x[..3].iter().map(|v| v * v).sum::<f32>().sqrt(); 4],
            "dotproduct" => [a[..3].iter().zip(&b).map(|(a, b)| a * b).sum(); 4],
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
            let values: Vec<f32> = value
                .split(|c: char| c == ',' || c.is_whitespace())
                .filter(|s| !s.is_empty())
                .map(|s| s.parse().map_err(|_| format!("invalid MaterialX number {s}")))
                .collect::<Result<_, _>>()?;
            if values.is_empty() {
                return Err("empty MaterialX value".into());
            }
            return Ok(Expr::constant(std::array::from_fn(|i| {
                values.get(i).copied().unwrap_or(if values.len() == 1 {
                    values[0]
                } else if i == 3 {
                    1.0
                } else {
                    0.0
                })
            })));
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
            inputs: HashMap::new(),
            value: [0.0; 4],
            image: None,
            channels: String::new(),
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
                let mut image = image::ImageReader::open(&path)
                    .map_err(|e| format!("{}: {e}", path.display()))?
                    .decode()
                    .map_err(|e| e.to_string())?
                    .to_rgba32f();
                if image.width() > 4096 || image.height() > 4096 {
                    image = image::imageops::thumbnail(&image, 4096, 4096);
                }
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
