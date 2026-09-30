//! `scene-render` — validate and inspect scene-render 1.1 documents.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use anstream::{AutoStream, ColorChoice};
use anstyle::{AnsiColor, Style};
use clap::{CommandFactory, Parser, Subcommand, ValueEnum};
use sr_model::diag::LineIndex;
use sr_model::{codes, Diagnostic, LoadError, LoadOptions, Report, Severity};

const EXAMPLES: &str = "\
Examples:
  scene-render validate promo.scene.xml
  scene-render validate scenes/*.xml --format json > report.json
  scene-render validate promo.scene.xml --no-assets --deny-warnings
  scene-render inspect promo.scene.xml
  scene-render eval promo.scene.xml --time 2.5 --variant dark --param headline=Hi
  scene-render eval promo.scene.xml --bench
  scene-render explain C21

Exit status: 0 valid, 1 invalid (or warnings with --deny-warnings; render/encode: anything\nnot rendered as authored with --strict), 2 usage or I/O error.";

/// Validate and inspect scene-render 1.1 documents.
///
/// Validation checks the XSD structure, the Schematron co-occurrence and
/// reference rules, and every referenced file with its SHA-256 digest.
#[derive(Parser, Debug)]
#[command(name = "scene-render", version, propagate_version = true, after_help = EXAMPLES, arg_required_else_help = true)]
struct Cli {
    /// When to use colour: auto detects a terminal and honours NO_COLOR.
    #[arg(long, value_enum, default_value_t = Color::Auto, global = true, env = "SCENE_RENDER_COLOR")]
    color: Color,

    /// Worker threads for the CPU stages (vector tiling, audio analysis); default: every core.
    #[arg(long, global = true, env = "SR_THREADS", value_name = "N")]
    threads: Option<usize>,

    #[command(subcommand)]
    command: Command,
}

#[derive(Copy, Clone, Debug, ValueEnum)]
enum Color {
    Auto,
    Always,
    Never,
}

#[derive(Copy, Clone, Debug, PartialEq, Eq, ValueEnum)]
enum Format {
    /// Diagnostics with source excerpts.
    Human,
    /// One JSON document on stdout.
    Json,
}

#[derive(Copy, Clone, Debug, PartialEq, Eq, ValueEnum)]
enum EvalFormat {
    /// One line per node.
    Summary,
    /// The full FrameGraph as JSON.
    Json,
}

fn parse_parallel(s: &str) -> Result<sr_deliver::Parallel, String> {
    sr_deliver::Parallel::parse(s).ok_or_else(|| format!("expected auto or a positive count, got {s:?}"))
}

fn parse_param(s: &str) -> Result<(String, String), String> {
    s.split_once('=')
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .ok_or_else(|| format!("expected ID=VALUE, got {s:?}"))
}

#[derive(Subcommand, Debug)]
enum Command {
    /// Validate documents and report every problem found.
    Validate {
        /// Scene documents to validate.
        #[arg(required = true, value_name = "FILE")]
        files: Vec<PathBuf>,
        /// Output format.
        #[arg(long, value_enum, default_value_t = Format::Human)]
        format: Format,
        /// Skip file existence and SHA-256 checks.
        #[arg(long)]
        no_assets: bool,
        /// Directory relative URIs resolve against (default: each document's directory).
        #[arg(long, value_name = "DIR")]
        base_dir: Option<PathBuf>,
        /// Treat warnings as errors for the exit status.
        #[arg(long)]
        deny_warnings: bool,
        /// Print only the per-file summary lines.
        #[arg(short, long)]
        quiet: bool,
    },
    /// Load a document and summarise the typed model.
    Inspect {
        /// Scene document.
        #[arg(value_name = "FILE")]
        file: PathBuf,
        /// Print the full typed model as JSON.
        #[arg(long)]
        json: bool,
        /// Skip file existence and SHA-256 checks.
        #[arg(long)]
        no_assets: bool,
    },
    /// Evaluate one frame and print its FrameGraph.
    Eval {
        /// Scene document.
        #[arg(value_name = "FILE")]
        file: PathBuf,
        /// Composition time in seconds.
        #[arg(long, short, conflicts_with = "frame", allow_negative_numbers = true)]
        time: Option<f64>,
        /// Frame index at the project frame rate.
        #[arg(long, short)]
        frame: Option<u64>,
        /// Variant to apply.
        #[arg(long)]
        variant: Option<String>,
        /// Layout to render.
        #[arg(long)]
        layout: Option<String>,
        /// Parameter value, repeatable: --param id=value.
        #[arg(long = "param", value_name = "ID=VALUE", value_parser = parse_param)]
        params: Vec<(String, String)>,
        /// Data row for batch rendering.
        #[arg(long)]
        row: Option<usize>,
        /// Data source of --row (default: the first one).
        #[arg(long, requires = "row")]
        data: Option<String>,
        /// Output format.
        #[arg(long, value_enum, default_value_t = EvalFormat::Summary)]
        format: EvalFormat,
        /// Evaluate every frame and report timing instead of printing a frame.
        #[arg(long)]
        bench: bool,
        /// Skip file existence and SHA-256 checks.
        #[arg(long)]
        no_assets: bool,
    },
    /// Render frames on the GPU and write PNG files.
    Render {
        /// Scene document.
        #[arg(value_name = "FILE")]
        file: PathBuf,
        /// Composition time in seconds.
        #[arg(long, short, conflicts_with_all = ["frame", "frames"], allow_negative_numbers = true)]
        time: Option<f64>,
        /// Frame index at the project frame rate.
        #[arg(long, short, conflicts_with = "frames")]
        frame: Option<u64>,
        /// Frame range START..END (end exclusive) or START..=END.
        #[arg(long, value_name = "RANGE", value_parser = parse_range)]
        frames: Option<(u64, u64)>,
        /// Output PNG; for several frames include %04d or {frame} in the name.
        #[arg(long, short, value_name = "PATH", required_unless_present = "bench")]
        output: Option<PathBuf>,
        /// Bits per channel of the PNG.
        #[arg(long, value_parser = ["8", "16"], default_value = "8")]
        bit_depth: String,
        /// Variant to apply.
        #[arg(long)]
        variant: Option<String>,
        /// Layout to render.
        #[arg(long)]
        layout: Option<String>,
        /// Parameter value, repeatable: --param id=value.
        #[arg(long = "param", value_name = "ID=VALUE", value_parser = parse_param)]
        params: Vec<(String, String)>,
        /// Data row for batch rendering.
        #[arg(long)]
        row: Option<usize>,
        /// Data source of --row (default: the first one).
        #[arg(long, requires = "row")]
        data: Option<String>,
        /// Render the frames into GPU memory and report timing instead of writing files.
        #[arg(long)]
        bench: bool,
        /// Print per-frame renderer statistics as JSON lines on stderr.
        #[arg(long)]
        stats: bool,
        /// With --bench, keep one frame in flight: wait for the previous frame's submission
        /// instead of draining the GPU after every frame, so the figure is throughput.
        #[arg(long, requires = "bench")]
        pipelined: bool,
        /// Exit 1 if anything was not rendered as authored: unsupported content, shader fallbacks
        /// (pass-through effects, crossfaded transitions), evaluator warnings or, when encoding,
        /// accessibility findings. Output files are still written.
        #[arg(long)]
        strict: bool,
    },
    /// Render the document's outputs (or one ad-hoc output) to finished files.
    Encode {
        /// Scene document.
        #[arg(value_name = "FILE")]
        file: PathBuf,
        /// Render only these outputs (by id); all outputs by default.
        #[arg(long = "output", value_name = "ID")]
        outputs: Vec<String>,
        /// Ad-hoc output path instead of the document's outputs.
        #[arg(short = 'o', long = "path", value_name = "PATH")]
        path: Option<String>,
        /// Codec of the ad-hoc output (default from the extension).
        #[arg(long, requires = "path")]
        codec: Option<String>,
        /// Directory relative output paths resolve against (default: the document's).
        #[arg(long, value_name = "DIR")]
        out_dir: Option<PathBuf>,
        /// Preferred asset representation (e.g. proxy).
        #[arg(long)]
        representation: Option<String>,
        /// Encoder family for H.264, H.265 and AV1.
        #[arg(long = "hw", value_parser = ["auto", "software", "nvenc", "videotoolbox", "vaapi", "qsv", "amf"], default_value = "auto")]
        hardware: String,
        /// Start time override, seconds.
        #[arg(long)]
        start: Option<f64>,
        /// End time override, seconds.
        #[arg(long)]
        end: Option<f64>,
        /// Skip destinations.
        #[arg(long)]
        no_upload: bool,
        /// Time segments of a video output rendered and encoded at once, then joined without
        /// re-encoding: `auto` (3 with a hardware encoder, else half the cores up to 4, fewer for
        /// short ranges) or a count; 1 renders serially.
        #[arg(long, value_name = "auto|N", value_parser = parse_parallel, default_value = "auto")]
        parallel: sr_deliver::Parallel,
        /// Parameter value, repeatable: --param id=value.
        #[arg(long = "param", value_name = "ID=VALUE", value_parser = parse_param)]
        params: Vec<(String, String)>,
        /// Data row for batch rendering.
        #[arg(long)]
        row: Option<usize>,
        /// Data source of --row (default: the first one).
        #[arg(long, requires = "row")]
        data: Option<String>,
        /// Print each report as JSON.
        #[arg(long)]
        json: bool,
        /// Exit 1 if anything was not rendered as authored: unsupported content, shader fallbacks
        /// (pass-through effects, crossfaded transitions), evaluator warnings or, when encoding,
        /// accessibility findings. Output files are still written.
        #[arg(long)]
        strict: bool,
    },
    /// Simulate the document's physics and write its cache file (physics@cache).
    Simulate {
        /// Scene document.
        file: PathBuf,
        /// Cache file to write (default: the document's physics@cache).
        #[arg(short, long)]
        output: Option<PathBuf>,
    },
    /// Fill the caches of generated media and transcribed captions through their providers,
    /// and pin their SHA-256 in the document.
    ///
    /// Providers: whisper (whisper.cpp), piper, audioforge, the cloud services openai and
    /// elevenlabs (with --allow-cloud), or any `scene-render-provider-NAME` program speaking
    /// the JSON protocol. Unchanged targets are never made again.
    Resolve {
        /// Scene document (its cacheSha256 attributes are rewritten in place).
        file: PathBuf,
        /// Report what is stale and change nothing; exit 1 if anything is.
        #[arg(long)]
        check: bool,
        /// Make every target again.
        #[arg(long)]
        force: bool,
        /// Only these ids (repeatable).
        #[arg(long, value_name = "ID")]
        only: Vec<String>,
        /// Allow providers that send prompts to a cloud service.
        #[arg(long)]
        allow_cloud: bool,
        /// Do not read or write the shared result store.
        #[arg(long)]
        no_store: bool,
        /// Print the results as JSON.
        #[arg(long)]
        json: bool,
    },
    /// Describe a diagnostic code, or list every code.
    Explain {
        /// Code such as S06, C21 or R24-fill; omit to list all codes.
        code: Option<String>,
    },
    /// Print a shell completion script.
    Completions {
        /// Target shell.
        #[arg(value_enum)]
        shell: clap_complete::Shell,
    },
}

struct Out {
    w: AutoStream<std::io::Stdout>,
}

fn style(c: AnsiColor, bold: bool) -> Style {
    let s = Style::new().fg_color(Some(c.into()));
    if bold {
        s.bold()
    } else {
        s
    }
}

impl Out {
    fn new(color: Color) -> Self {
        let choice = match color {
            Color::Auto => ColorChoice::Auto,
            Color::Always => ColorChoice::Always,
            Color::Never => ColorChoice::Never,
        };
        Out { w: AutoStream::new(std::io::stdout(), choice) }
    }

    fn diagnostic(&mut self, file: &Path, lines: &LineIndex, d: &Diagnostic) -> std::io::Result<()> {
        let (label, color) = match d.severity {
            Severity::Error => ("error", AnsiColor::Red),
            Severity::Warning => ("warning", AnsiColor::Yellow),
        };
        let head = style(color, true);
        let bold = Style::new().bold();
        let blue = style(AnsiColor::Blue, true);
        writeln!(self.w, "{head}{label}[{}]{head:#}{bold}: {}{bold:#}", d.code, d.message)?;
        let gutter = d.loc.line.max(1).to_string().len();
        let pad = " ".repeat(gutter);
        writeln!(self.w, "{pad}{blue}-->{blue:#} {}:{}:{}", file.display(), d.loc.line, d.loc.column)?;
        let text = lines.line_text(d.loc.line);
        if d.loc.line > 0 && !text.is_empty() {
            let shown: String = text.chars().map(|c| if c == '\t' { ' ' } else { c }).collect();
            writeln!(self.w, "{pad} {blue}|{blue:#}")?;
            writeln!(self.w, "{blue}{}{blue:#} {blue}|{blue:#} {shown}", d.loc.line)?;
            let col = d.loc.column.max(1) as usize - 1;
            let span =
                shown.chars().skip(col).take_while(|c| !c.is_whitespace() && *c != '>' && *c != '/').count().max(1);
            writeln!(self.w, "{pad} {blue}|{blue:#} {}{head}{}{head:#}", " ".repeat(col), "^".repeat(span))?;
        }
        if let Some(h) = &d.help {
            writeln!(self.w, "{pad} {blue}={blue:#} {bold}help{bold:#}: {h}")?;
        }
        writeln!(self.w, "{pad} {blue}={blue:#} at {}", d.path)?;
        writeln!(self.w)
    }

    fn summary(&mut self, file: &Path, report: &Report, deny_warnings: bool) -> std::io::Result<()> {
        let (e, w) = (report.error_count(), report.warning_count());
        let ok = e == 0 && !(deny_warnings && w > 0);
        let (mark, st) =
            if ok { ("valid", style(AnsiColor::Green, true)) } else { ("invalid", style(AnsiColor::Red, true)) };
        writeln!(self.w, "{st}{mark}{st:#} {}: {e} error(s), {w} warning(s)", file.display())
    }
}

#[derive(serde::Serialize)]
struct FileReport<'a> {
    file: String,
    valid: bool,
    errors: usize,
    warnings: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    io_error: Option<String>,
    diagnostics: &'a [Diagnostic],
}

fn validate(
    files: &[PathBuf],
    format: Format,
    no_assets: bool,
    base_dir: Option<PathBuf>,
    deny_warnings: bool,
    quiet: bool,
    out: &mut Out,
) -> std::io::Result<ExitCode> {
    let opts = LoadOptions { verify_assets: !no_assets, base_dir };
    let mut worst = 0u8;
    let mut json_files = Vec::new();
    let mut reports = Vec::new();
    for file in files {
        let text = match std::fs::read(file).map(String::from_utf8) {
            Ok(Ok(t)) => t,
            Ok(Err(e)) => {
                reports.push((file, None, Some(format!("not UTF-8: {e}"))));
                continue;
            }
            Err(e) => {
                reports.push((file, None, Some(e.to_string())));
                continue;
            }
        };
        let mut o = opts.clone();
        if o.base_dir.is_none() {
            o.base_dir = Some(match file.parent() {
                Some(p) if !p.as_os_str().is_empty() => p.to_path_buf(),
                _ => PathBuf::from("."),
            });
        }
        let report = sr_model::validate_str(&text, &o);
        reports.push((file, Some((text, report)), None));
    }
    for (file, result, io) in &reports {
        match (result, io) {
            (_, Some(err)) => {
                worst = 2;
                if format == Format::Json {
                    json_files.push(FileReport {
                        file: file.display().to_string(),
                        valid: false,
                        errors: 0,
                        warnings: 0,
                        io_error: Some(err.clone()),
                        diagnostics: &[],
                    });
                } else {
                    let st = style(AnsiColor::Red, true);
                    let mut e = AutoStream::auto(std::io::stderr());
                    writeln!(e, "{st}error{st:#}: cannot read {}: {err}", file.display())?;
                }
            }
            (Some((text, report)), None) => {
                let ok = !report.has_errors() && !(deny_warnings && report.warning_count() > 0);
                if !ok {
                    worst = worst.max(1);
                }
                if format == Format::Json {
                    json_files.push(FileReport {
                        file: file.display().to_string(),
                        valid: ok,
                        errors: report.error_count(),
                        warnings: report.warning_count(),
                        io_error: None,
                        diagnostics: &report.diagnostics,
                    });
                } else {
                    if !quiet {
                        let lines = LineIndex::new(text);
                        for d in &report.diagnostics {
                            out.diagnostic(file, &lines, d)?;
                        }
                    }
                    out.summary(file, report, deny_warnings)?;
                }
            }
            (None, None) => unreachable!(),
        }
    }
    if format == Format::Json {
        let doc = serde_json::json!({ "valid": worst == 0, "files": json_files });
        writeln!(out.w, "{}", serde_json::to_string_pretty(&doc).expect("serialisable"))?;
    }
    Ok(ExitCode::from(worst))
}

fn simulate(file: &Path, output: Option<PathBuf>, out: &mut Out) -> std::io::Result<ExitCode> {
    use sha2::Digest;
    // the cache being regenerated may no longer match its recorded digest, so assets are not verified
    let doc = match sr_model::load_file(file, &LoadOptions::without_assets()) {
        Ok(d) => d,
        Err(e) => {
            eprintln!("error: {}: {e}", file.display());
            return Ok(ExitCode::from(1));
        }
    };
    let dir = file.parent().map(Path::to_path_buf).unwrap_or_default();
    let target = match output.or_else(|| doc.scene.physics.as_ref().and_then(|p| p.cache.as_ref()).map(|c| dir.join(c)))
    {
        Some(t) => t,
        None => {
            eprintln!("error: {} has no physics@cache; name the cache file with -o", file.display());
            return Ok(ExitCode::from(2));
        }
    };
    let ev = match sr_eval::Evaluator::new(&doc, &sr_eval::EvalOptions::default()) {
        Ok(ev) => ev,
        Err(report) => {
            for d in &report.diagnostics {
                eprintln!("error[{}]: {}", d.code, d.message);
            }
            return Ok(ExitCode::from(1));
        }
    };
    let bytes = match ev.physics_cache() {
        Ok(b) => b,
        Err(e) => {
            eprintln!("error: {}: {e}", file.display());
            return Ok(ExitCode::from(1));
        }
    };
    std::fs::write(&target, &bytes)?;
    let sha: String = sha2::Sha256::digest(&bytes).iter().map(|b| format!("{b:02x}")).collect();
    writeln!(out.w, "wrote {} ({} bytes)", target.display(), bytes.len())?;
    writeln!(out.w, "cacheSha256=\"{sha}\"")?;
    Ok(ExitCode::SUCCESS)
}

fn resolve(file: &Path, o: &sr_resolve::Options, json: bool, out: &mut Out) -> std::io::Result<ExitCode> {
    use sr_resolve::Status;
    let rows = match sr_resolve::resolve(file, o) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("error: {e}");
            return Ok(ExitCode::from(2));
        }
    };
    if json {
        writeln!(out.w, "{}", serde_json::to_string_pretty(&rows).unwrap_or_default())?;
    } else {
        for r in &rows {
            let (label, color) = match r.status {
                Status::UpToDate => ("up to date", AnsiColor::Green),
                Status::Pinned => ("pinned", AnsiColor::Green),
                Status::Restored => ("restored", AnsiColor::Green),
                Status::Made => ("made", AnsiColor::Green),
                Status::Stale => ("stale", AnsiColor::Yellow),
                Status::Error => ("error", AnsiColor::Red),
            };
            let st = style(color, true);
            let sha = r.sha256.as_deref().map(|s| &s[..12.min(s.len())]).unwrap_or("");
            write!(out.w, "{st}{label:>10}{st:#}  {} ({}, {})  {}", r.id, r.element, r.provider, r.cache)?;
            if !sha.is_empty() {
                write!(out.w, "  {sha}…")?;
            }
            writeln!(out.w)?;
            if !r.message.is_empty() {
                writeln!(out.w, "            {}", r.message)?;
            }
            for n in &r.notes {
                writeln!(out.w, "            note: {n}")?;
            }
        }
        if rows.is_empty() {
            writeln!(out.w, "nothing to resolve: no <generated> assets or transcribed caption tracks")?;
        }
    }
    let failed = rows.iter().any(|r| r.status == Status::Error || (o.check && r.status == Status::Stale));
    Ok(if failed { ExitCode::from(1) } else { ExitCode::SUCCESS })
}

fn inspect(file: &Path, json: bool, no_assets: bool, out: &mut Out) -> std::io::Result<ExitCode> {
    let opts = if no_assets { LoadOptions::without_assets() } else { LoadOptions::default() };
    let doc = match sr_model::load_file(file, &opts) {
        Ok(d) => d,
        Err(LoadError::Io { path, source }) => {
            eprintln!("error: cannot read {}: {source}", path.display());
            return Ok(ExitCode::from(2));
        }
        Err(LoadError::Invalid(report)) => {
            let text = std::fs::read_to_string(file).unwrap_or_default();
            let lines = LineIndex::new(&text);
            for d in &report.diagnostics {
                out.diagnostic(file, &lines, d)?;
            }
            out.summary(file, &report, false)?;
            return Ok(ExitCode::from(1));
        }
    };
    if json {
        writeln!(out.w, "{}", serde_json::to_string_pretty(&doc.scene).expect("serialisable"))?;
        return Ok(ExitCode::SUCCESS);
    }
    let s = &doc.scene;
    let (w, h) = doc.frame_size();
    let bold = Style::new().bold();
    writeln!(out.w, "{bold}{}{bold:#}", file.display())?;
    writeln!(out.w, "  version      {}", doc.version())?;
    writeln!(out.w, "  frame        {w}x{h} @ {} fps", doc.fps())?;
    writeln!(out.w, "  duration     {} s = {} frames", doc.duration(), doc.frame_count())?;
    writeln!(out.w, "  background   {}", s.project.background)?;
    writeln!(
        out.w,
        "  color space  {}{}",
        s.project.working_color_space,
        if s.project.linear_light { " (linear light)" } else { "" }
    )?;
    let mut kinds: std::collections::BTreeMap<&str, usize> = Default::default();
    if let Some(a) = &s.assets {
        for c in &a.children {
            *kinds.entry(c.element_name()).or_default() += 1;
        }
    }
    let kinds: Vec<String> = kinds.iter().map(|(k, v)| format!("{v} {k}")).collect();
    writeln!(out.w, "  assets       {}", if kinds.is_empty() { "none".into() } else { kinds.join(", ") })?;
    let nodes = doc.composition_nodes();
    let depth = nodes.iter().map(|(d, _)| d + 1).max().unwrap_or(0);
    writeln!(out.w, "  nodes        {} in the composition, depth {depth}", nodes.len())?;
    writeln!(out.w, "  symbols      {}", s.symbols.as_ref().map_or(0, |x| x.symbols.len()))?;
    writeln!(out.w, "  effects      {}", s.effects.as_ref().map_or(0, |x| x.effects.len()))?;
    writeln!(out.w, "  outputs      {}", s.outputs.len())?;
    for o in &s.outputs {
        writeln!(out.w, "    {} ({})", o.path, o.codec)?;
    }
    writeln!(out.w, "  ids          {}", doc.index().len())?;
    if !doc.warnings().is_empty() {
        writeln!(out.w, "  warnings     {}", doc.warnings().len())?;
    }
    Ok(ExitCode::SUCCESS)
}

fn explain(code: Option<&str>, out: &mut Out) -> std::io::Result<ExitCode> {
    let bold = Style::new().bold();
    match code {
        None => {
            for c in codes::all() {
                writeln!(out.w, "{bold}{:<16}{bold:#} {:<9} {}", c.code, c.stage, c.summary)?;
            }
            for (c, summary) in sr_eval::codes::CODES {
                writeln!(out.w, "{bold}{c:<16}{bold:#} {:<9} {summary}", "eval")?;
            }
            Ok(ExitCode::SUCCESS)
        }
        Some(code) if sr_eval::codes::CODES.iter().any(|(c, _)| c.eq_ignore_ascii_case(code)) => {
            let (c, summary) = sr_eval::codes::CODES.iter().find(|(c, _)| c.eq_ignore_ascii_case(code)).unwrap();
            writeln!(out.w, "{bold}{c}{bold:#} (eval)\n{summary}")?;
            Ok(ExitCode::SUCCESS)
        }
        Some(code) => match codes::lookup(code) {
            Some(c) => {
                writeln!(out.w, "{bold}{}{bold:#} ({})", c.code, c.stage)?;
                writeln!(out.w, "{}", c.summary)?;
                if let Some((ctx, test)) = c.rule {
                    writeln!(out.w, "\n  context: {ctx}\n  test:    {test}")?;
                }
                Ok(ExitCode::SUCCESS)
            }
            None => {
                let all: Vec<&str> = codes::all().iter().map(|c| c.code).collect();
                eprintln!("error: unknown code {code:?}; `scene-render explain` lists all {} codes", all.len());
                Ok(ExitCode::from(2))
            }
        },
    }
}

#[allow(clippy::too_many_arguments)]
fn eval(
    file: &Path,
    time: Option<f64>,
    frame: Option<u64>,
    opts: sr_eval::EvalOptions,
    format: EvalFormat,
    bench: bool,
    no_assets: bool,
    out: &mut Out,
) -> std::io::Result<ExitCode> {
    let lopts = if no_assets { LoadOptions::without_assets() } else { LoadOptions::default() };
    let text = match std::fs::read_to_string(file) {
        Ok(t) => t,
        Err(e) => {
            eprintln!("error: cannot read {}: {e}", file.display());
            return Ok(ExitCode::from(2));
        }
    };
    let lines = LineIndex::new(&text);
    let report_errors = |out: &mut Out, report: &Report| -> std::io::Result<ExitCode> {
        for d in &report.diagnostics {
            out.diagnostic(file, &lines, d)?;
        }
        out.summary(file, report, false)?;
        Ok(ExitCode::from(1))
    };
    let doc = match sr_model::load_file(file, &lopts) {
        Ok(d) => d,
        Err(LoadError::Io { path, source }) => {
            eprintln!("error: cannot read {}: {source}", path.display());
            return Ok(ExitCode::from(2));
        }
        Err(LoadError::Invalid(r)) => return report_errors(out, &r),
    };
    let ev = match sr_eval::Evaluator::new(&doc, &opts) {
        Ok(e) => e,
        Err(r) => return report_errors(out, &r),
    };
    for w in ev.warnings() {
        out.diagnostic(file, &lines, w)?;
    }
    if bench {
        let n = ev.frame_count().max(1);
        let mut times: Vec<f64> = (0..n)
            .map(|i| {
                let t = std::time::Instant::now();
                std::hint::black_box(ev.evaluate_frame(i));
                t.elapsed().as_secs_f64() * 1e3
            })
            .collect();
        times.sort_by(|a, b| a.partial_cmp(b).unwrap());
        let (nodes, slots, channels, exprs) = ev.stats();
        writeln!(
            out.w,
            "{}: {nodes} nodes, {slots} properties, {channels} keyframe channels, {exprs} expressions",
            file.display()
        )?;
        writeln!(
            out.w,
            "{n} frames: median {:.3} ms, p95 {:.3} ms, max {:.3} ms per frame on one core",
            times[times.len() / 2],
            times[(times.len() * 95 / 100).min(times.len() - 1)],
            times[times.len() - 1]
        )?;
        return Ok(ExitCode::SUCCESS);
    }
    let g = match (time, frame) {
        (Some(t), _) => ev.evaluate(t),
        (None, Some(f)) => ev.evaluate_frame(f),
        (None, None) => ev.evaluate(0.0),
    };
    if format == EvalFormat::Json {
        writeln!(out.w, "{}", serde_json::to_string_pretty(&g).expect("serialisable"))?;
        return Ok(ExitCode::SUCCESS);
    }
    let bold = Style::new().bold();
    let dim = Style::new().dimmed();
    writeln!(
        out.w,
        "{bold}t = {:.4} s{bold:#}  frame {}  {}x{}  {} nodes",
        g.time,
        g.frame,
        g.size[0],
        g.size[1],
        g.nodes.len()
    )?;
    for n in &g.nodes {
        let o = n.world.apply([0.0, 0.0]);
        let rot = n.world.0[1].atan2(n.world.0[0]).to_degrees();
        let indent = "  ".repeat(n.depth as usize);
        let mut extra = String::new();
        if let Some(s) = n.source_time {
            extra.push_str(&format!(" src={s:.3}"));
        }
        if let Some(t) = &n.text {
            extra.push_str(&format!(" text={t:?}"));
        }
        if n.is_matte {
            extra.push_str(" matte-source");
        }
        for (k, v) in &n.props.0 {
            extra.push_str(&format!(" {k}={}", serde_json::to_string(v).unwrap_or_default()));
        }
        let hidden = if n.draw { "" } else { " (hidden)" };
        writeln!(
            out.w,
            "{indent}{bold}{}{bold:#} {dim}{}{hidden}{dim:#}  at ({:.1}, {:.1}) rot {:.1}° opacity {:.3} local {:.3}{extra}",
            n.id, n.kind, o[0], o[1], rot, n.world_opacity, n.local_time
        )?;
    }
    for t in &g.transitions {
        let name = |i: Option<u32>| i.map(|i| g.nodes[i as usize].id.to_string()).unwrap_or_else(|| "-".into());
        writeln!(out.w, "transition {} {} → {} progress {:.3}", t.kind, name(t.from), name(t.to), t.progress)?;
    }
    if let Some(c) = g.camera {
        writeln!(out.w, "camera {}", g.nodes[c as usize].id)?;
    }
    for e in &g.elements {
        writeln!(out.w, "{} <{}> {}", e.key, e.element, serde_json::to_string(&e.props).unwrap_or_default())?;
    }
    Ok(ExitCode::SUCCESS)
}

fn parse_range(s: &str) -> Result<(u64, u64), String> {
    let (a, b, inclusive) = if let Some((a, b)) = s.split_once("..=") {
        (a, b, true)
    } else if let Some((a, b)) = s.split_once("..") {
        (a, b, false)
    } else {
        return Err("expected START..END or START..=END".into());
    };
    let a: u64 = a.trim().parse().map_err(|_| format!("bad start {a:?}"))?;
    let b: u64 = b.trim().parse().map_err(|_| format!("bad end {b:?}"))?;
    let end = if inclusive { b + 1 } else { b };
    if end <= a {
        return Err("empty frame range".into());
    }
    Ok((a, end))
}

fn frame_path(pattern: &Path, frame: u64, several: bool) -> Result<PathBuf, String> {
    let s = pattern.to_string_lossy();
    if s.contains("{frame}") {
        return Ok(PathBuf::from(s.replace("{frame}", &frame.to_string())));
    }
    if let Some(i) = s.find('%') {
        let rest = &s[i + 1..];
        let digits: String = rest.chars().take_while(|c| c.is_ascii_digit()).collect();
        if rest[digits.len()..].starts_with('d') {
            let width: usize = digits.trim_start_matches('0').parse().unwrap_or(0);
            let num = format!("{frame:0width$}");
            return Ok(PathBuf::from(format!("{}{num}{}", &s[..i], &rest[digits.len() + 1..])));
        }
    }
    if several {
        Err(format!("{} names one file; use %04d or {{frame}} to write several frames", pattern.display()))
    } else {
        Ok(pattern.to_path_buf())
    }
}

#[allow(clippy::too_many_arguments)]
fn render(
    file: &Path,
    frames: Vec<u64>,
    time: Option<f64>,
    opts: sr_eval::EvalOptions,
    output: Option<PathBuf>,
    bit_depth: u8,
    bench: bool,
    stats: bool,
    pipelined: bool,
    strict: bool,
    out: &mut Out,
) -> std::io::Result<ExitCode> {
    let text = match std::fs::read_to_string(file) {
        Ok(t) => t,
        Err(e) => {
            eprintln!("error: cannot read {}: {e}", file.display());
            return Ok(ExitCode::from(2));
        }
    };
    let lines = LineIndex::new(&text);
    let report_errors = |out: &mut Out, report: &Report| -> std::io::Result<ExitCode> {
        for d in &report.diagnostics {
            out.diagnostic(file, &lines, d)?;
        }
        out.summary(file, report, false)?;
        Ok(ExitCode::from(1))
    };
    let doc = match sr_model::load_file(file, &LoadOptions::default()) {
        Ok(d) => d,
        Err(LoadError::Io { path, source }) => {
            eprintln!("error: cannot read {}: {source}", path.display());
            return Ok(ExitCode::from(2));
        }
        Err(LoadError::Invalid(r)) => return report_errors(out, &r),
    };
    let ev = match sr_eval::Evaluator::new(&doc, &opts) {
        Ok(e) => e,
        Err(r) => return report_errors(out, &r),
    };
    for w in ev.warnings() {
        out.diagnostic(file, &lines, w)?;
    }
    let eval_warnings = ev.warnings().len();
    let gpu = match sr_gpu::Gpu::new() {
        Ok(g) => g,
        Err(e) => {
            eprintln!("error: {e}");
            return Ok(ExitCode::from(2));
        }
    };
    let adapter = format!("{} ({:?}, {:?})", gpu.info.name, gpu.info.backend, gpu.info.device_type);
    let mut r = sr_gpu::Renderer::new(gpu, ev.program());
    let graphs = |f: u64| ev.evaluate_frame(f);
    let mut unsupported = std::collections::BTreeSet::new();
    if bench {
        let frames = if frames.is_empty() { (0..ev.frame_count().max(1)).collect() } else { frames };
        let warm = frames.len().min(2);
        for &f in &frames[..warm] {
            let mut sub = |st: f64| ev.evaluate(st);
            r.render_with(&graphs(f), ev.program(), Some(&mut sub));
            r.gpu().wait();
        }
        let mut times = Vec::new();
        let mut eval_ms = Vec::new();
        let mut submit_ms = Vec::new();
        let mut last = sr_gpu::RenderStats::default();
        let mut prev: Option<sr_gpu::SubmissionIndex> = None;
        for &f in &frames {
            let t0 = std::time::Instant::now();
            let g = graphs(f);
            let t1 = std::time::Instant::now();
            let mut sub = |st: f64| ev.evaluate(st);
            last = r.render_with(&g, ev.program(), Some(&mut sub)).stats;
            submit_ms.push(t1.elapsed().as_secs_f64() * 1e3);
            if pipelined {
                if let Some(p) = prev.take() {
                    r.gpu().wait_for(p);
                }
                prev = r.last_submission();
            } else {
                r.gpu().wait();
            }
            eval_ms.push((t1 - t0).as_secs_f64() * 1e3);
            times.push(t1.elapsed().as_secs_f64() * 1e3);
            unsupported.extend(last.unsupported.iter().cloned());
            if stats {
                eprintln!("{}", serde_json::json!({ "frame": f, "stats": &last }));
            }
        }
        if let Some(p) = prev.take() {
            r.gpu().wait_for(p);
        }
        let sorted = |mut v: Vec<f64>| {
            v.sort_by(|a, b| a.partial_cmp(b).unwrap());
            v
        };
        let (times, eval_ms, submit_ms) = (sorted(times), sorted(eval_ms), sorted(submit_ms));
        let med = times[times.len() / 2];
        writeln!(
            out.w,
            "{}: {} frames on {adapter}{}",
            file.display(),
            frames.len(),
            if pipelined { ", pipelined" } else { "" }
        )?;
        writeln!(
            out.w,
            "render: median {med:.3} ms ({:.1} fps), p95 {:.3} ms; of which CPU planning and submission {:.3} ms; evaluation {:.3} ms",
            1e3 / med,
            times[(times.len() * 95 / 100).min(times.len() - 1)],
            submit_ms[submit_ms.len() / 2],
            eval_ms[eval_ms.len() / 2]
        )?;
        writeln!(
            out.w,
            "last frame: {} draws, {} targets, {} cache hits, {} restored, {} backdrop copies, {} effect passes, {} sub-frames",
            last.draws, last.targets, last.cache_hits, last.prefix_restored, last.backdrop_copies, last.fx_passes, last.subframes
        )?;
        if last.objects3d > 0 || last.splats > 0 {
            writeln!(out.w, "3d: {} mesh draws, {} triangles, {} splats", last.objects3d, last.triangles, last.splats)?;
        }
    } else {
        let output = output.expect("clap requires --output");
        let list: Vec<(Option<u64>, f64)> = match (time, frames.is_empty()) {
            (Some(t), _) => vec![(None, t)],
            (None, true) => vec![(Some(0), 0.0)],
            (None, false) => frames.iter().map(|&f| (Some(f), ev.program().fps.frame_time(f))).collect(),
        };
        let several = list.len() > 1;
        for (f, t) in list {
            let path = match frame_path(&output, f.unwrap_or(0), several) {
                Ok(p) => p,
                Err(e) => {
                    eprintln!("error: {e}");
                    return Ok(ExitCode::from(2));
                }
            };
            let g = ev.evaluate(t);
            let mut sub = |st: f64| ev.evaluate(st);
            let frame = r.render_with(&g, ev.program(), Some(&mut sub));
            unsupported.extend(frame.stats.unsupported.iter().cloned());
            if let Some(e) = frame.stats.errors.first() {
                eprintln!("error: frame at {:.3} s: {e}", g.time);
                return Ok(ExitCode::from(1));
            }
            if stats {
                eprintln!("{}", serde_json::json!({ "frame": g.frame, "stats": &frame.stats }));
            }
            let px = r.read(&frame.texture);
            let [w, h] = frame.texture.size;
            let res = if bit_depth == 16 {
                let working = r.working();
                let data: Vec<u16> = px
                    .iter()
                    .flat_map(|p| {
                        let a = p[3].clamp(0.0, 1.0) as f64;
                        let c = if a > 0.0 { [p[0] as f64 / a, p[1] as f64 / a, p[2] as f64 / a] } else { [0.0; 3] };
                        let d = working.to_display_srgb(c);
                        [d[0], d[1], d[2], a].map(|v| (v * 65535.0).round() as u16)
                    })
                    .collect();
                image::ImageBuffer::<image::Rgba<u16>, _>::from_raw(w, h, data).expect("sized").save(&path)
            } else {
                image::RgbaImage::from_raw(w, h, r.to_srgb8(&px)).expect("sized").save(&path)
            };
            if let Err(e) = res {
                eprintln!("error: cannot write {}: {e}", path.display());
                return Ok(ExitCode::from(2));
            }
            writeln!(out.w, "wrote {} (t = {:.4} s, {} draws)", path.display(), g.time, frame.stats.draws)?;
        }
    }
    let problems = unsupported.len() + eval_warnings;
    for u in unsupported {
        writeln!(out.w, "note: not rendered yet: {u}")?;
    }
    if strict && problems > 0 {
        eprintln!("error: --strict: {problems} item(s) not rendered as authored (see the notes and warnings above)");
        return Ok(ExitCode::from(1));
    }
    Ok(ExitCode::SUCCESS)
}

fn codec_for_path(p: &str) -> &'static str {
    let lower = p.to_ascii_lowercase();
    let ext = lower.rsplit('.').next().unwrap_or("");
    let seq = lower.contains('%');
    match ext {
        "mov" => "prores",
        "mxf" => "dnxhr",
        "mkv" => "ffv1",
        "webm" => "vp9",
        "gif" => "gif",
        "webp" => "webp",
        "png" if seq => "png-sequence",
        "png" => "apng",
        "jpg" | "jpeg" => "jpeg-sequence",
        "exr" => "exr-sequence",
        "tif" | "tiff" => "tiff-sequence",
        "wav" | "m4a" | "mp3" => "audio-only",
        _ => "h264",
    }
}

#[allow(clippy::too_many_arguments)]
fn encode(
    file: &Path,
    ids: &[String],
    path: Option<String>,
    codec: Option<String>,
    opts: sr_deliver::Options,
    json: bool,
    strict: bool,
    out: &mut Out,
) -> std::io::Result<ExitCode> {
    let text = match std::fs::read_to_string(file) {
        Ok(t) => t,
        Err(e) => {
            eprintln!("error: cannot read {}: {e}", file.display());
            return Ok(ExitCode::from(2));
        }
    };
    let lines = LineIndex::new(&text);
    let doc = match sr_model::load_file(file, &LoadOptions::default()) {
        Ok(d) => d,
        Err(LoadError::Io { path, source }) => {
            eprintln!("error: cannot read {}: {source}", path.display());
            return Ok(ExitCode::from(2));
        }
        Err(LoadError::Invalid(r)) => {
            for d in &r.diagnostics {
                out.diagnostic(file, &lines, d)?;
            }
            out.summary(file, &r, false)?;
            return Ok(ExitCode::from(1));
        }
    };
    let outputs: Vec<sr_model::model::Output> = match path {
        Some(p) => {
            let c = codec.unwrap_or_else(|| codec_for_path(&p).to_string());
            match sr_deliver::adhoc_output(&p, &c) {
                Ok(o) => vec![o],
                Err(e) => {
                    eprintln!("error: {e}");
                    return Ok(ExitCode::from(2));
                }
            }
        }
        None => doc
            .scene
            .outputs
            .iter()
            .filter(|o| ids.is_empty() || o.id.as_ref().is_some_and(|i| ids.contains(i)))
            .cloned()
            .collect(),
    };
    if outputs.is_empty() {
        eprintln!("error: {} defines no matching <output>; pass -o PATH for an ad-hoc output", file.display());
        return Ok(ExitCode::from(2));
    }
    let gpu = outputs.iter().any(|o| o.codec.as_str() != "audio-only").then(sr_gpu::Gpu::new).transpose();
    let gpu = match gpu {
        Ok(g) => g,
        Err(e) => {
            eprintln!("error: {e}");
            return Ok(ExitCode::from(2));
        }
    };
    let tty = std::io::IsTerminal::is_terminal(&std::io::stderr());
    let mut failed = false;
    for o in &outputs {
        let mut progress = |done: u64, total: u64| {
            if tty && (done % 10 == 0 || done == total) {
                eprint!("\r{}: frame {done}/{total}", o.path);
                if done == total {
                    eprintln!();
                }
            }
        };
        match sr_deliver::deliver(&doc, o, gpu.as_ref(), &opts, &mut progress) {
            Ok(r) => {
                let problems = r.unsupported.len() + r.accessibility.len();
                if strict && problems > 0 {
                    eprintln!(
                        "error: --strict: {}: {problems} item(s) not delivered as authored (unsupported content or accessibility findings)",
                        o.path
                    );
                    failed = true;
                }
                if json {
                    writeln!(out.w, "{}", serde_json::to_string(&r).expect("serialisable"))?;
                    continue;
                }
                let bold = Style::new().bold();
                writeln!(out.w, "{bold}wrote{bold:#} {} ({} file(s), {})", r.path.display(), r.files.len(), r.encoder)?;
                if r.frames > 0 {
                    let [e, s, w, b] = r.stage_seconds;
                    writeln!(
                        out.w,
                        "  {} frames {}x{} at {} fps in {:.2} s: {:.1} frames/s (evaluate {:.2} s, render {:.2} s of which decode wait {:.2} s and vector geometry {:.2} s, GPU and readback wait {:.2} s, encoder blocked {:.2} s), {} pass(es){}",
                        r.frames, r.size[0], r.size[1], r.fps, r.seconds, r.render_fps, e, s, r.decode_wait_seconds, r.vector_seconds, w, b, r.passes,
                        if r.segments > 1 { format!(", {} segments at once (stage times summed over them)", r.segments) } else { String::new() }
                    )?;
                }
                if let Some(tp) = r.true_peak {
                    let l = r
                        .loudness
                        .map(|l| format!("{l:.1} LUFS integrated"))
                        .unwrap_or_else(|| "too short for integrated loudness".into());
                    writeln!(out.w, "  audio: {l}, {tp:.1} dBTP true peak, mixed in {:.2} s", r.audio_seconds)?;
                }
                for u in &r.uploads {
                    writeln!(out.w, "  delivered to {u}")?;
                }
                for u in &r.unsupported {
                    writeln!(out.w, "  note: not rendered yet: {u}")?;
                }
                for a in &r.accessibility {
                    writeln!(out.w, "  accessibility: {a}")?;
                }
            }
            Err(e) => {
                eprintln!("error: {}: {e}", o.path);
                failed = true;
            }
        }
    }
    Ok(if failed { ExitCode::from(1) } else { ExitCode::SUCCESS })
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    if let Some(n) = cli.threads.filter(|&n| n > 0) {
        sr_vector::set_threads(n);
        if std::env::var_os("RAYON_NUM_THREADS").is_none() {
            std::env::set_var("RAYON_NUM_THREADS", n.to_string());
        }
    }
    let mut out = Out::new(cli.color);
    let res = match cli.command {
        Command::Validate { files, format, no_assets, base_dir, deny_warnings, quiet } => {
            validate(&files, format, no_assets, base_dir, deny_warnings, quiet, &mut out)
        }
        Command::Inspect { file, json, no_assets } => inspect(&file, json, no_assets, &mut out),
        Command::Eval { file, time, frame, variant, layout, params, row, data, format, bench, no_assets } => {
            let opts =
                sr_eval::EvalOptions { variant, layout, params, row: row.map(|r| (data, r)), ..Default::default() };
            eval(&file, time, frame, opts, format, bench, no_assets, &mut out)
        }
        Command::Render {
            file,
            time,
            frame,
            frames,
            output,
            bit_depth,
            variant,
            layout,
            params,
            row,
            data,
            bench,
            stats,
            pipelined,
            strict,
        } => {
            let opts =
                sr_eval::EvalOptions { variant, layout, params, row: row.map(|r| (data, r)), ..Default::default() };
            let list: Vec<u64> = match (frame, frames) {
                (Some(f), _) => vec![f],
                (None, Some((a, b))) => (a..b).collect(),
                (None, None) => Vec::new(),
            };
            render(
                &file,
                list,
                time,
                opts,
                output,
                bit_depth.parse().unwrap_or(8),
                bench,
                stats,
                pipelined,
                strict,
                &mut out,
            )
        }
        Command::Encode {
            file,
            outputs,
            path,
            codec,
            out_dir,
            representation,
            hardware,
            start,
            end,
            no_upload,
            parallel,
            params,
            row,
            data,
            json,
            strict,
        } => {
            let opts = sr_deliver::Options {
                out_dir,
                representation,
                hardware: sr_media::encode::Hardware::parse(&hardware).unwrap_or(sr_media::encode::Hardware::Auto),
                start,
                end,
                upload: !no_upload,
                params,
                row: row.map(|r| (data, r)),
                parallel,
            };
            encode(&file, &outputs, path, codec, opts, json, strict, &mut out)
        }
        Command::Simulate { file, output } => simulate(&file, output, &mut out),
        Command::Resolve { file, check, force, only, allow_cloud, no_store, json } => {
            let o = sr_resolve::Options { check, force, only, allow_cloud, store: no_store.then(PathBuf::new) };
            resolve(&file, &o, json, &mut out)
        }
        Command::Explain { code } => explain(code.as_deref(), &mut out),
        Command::Completions { shell } => {
            clap_complete::generate(shell, &mut Cli::command(), "scene-render", &mut std::io::stdout());
            Ok(ExitCode::SUCCESS)
        }
    };
    match res {
        Ok(code) => code,
        Err(e) if e.kind() == std::io::ErrorKind::BrokenPipe => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("error: {e}");
            ExitCode::from(2)
        }
    }
}
