# sr-resolve

The resolve step of scene-render: it fills the caches that rendering reads and pins their SHA-256 digests in the
document. Rendering never runs it. It makes:

- `<generated>` media, through providers (see the repository README, "Generated media and transcription");
- `<captionTrack transcribe="…">` transcriptions;
- online map tiles for `<tiles url="…">`;
- with the `pdf` feature (on by default), the page images and text regions of `<pdf>` assets (SREP 17), in
  `src/pdf.rs`.

## Features

| Feature | Default | What it adds | Toolchain |
|---|---|---|---|
| `pdf` | off | the resolve step of `pdf` assets: rendering one page and finding phrases on it | Rust 1.92 (hayro's minimum) |

The feature is opt-in (`cargo build -p scene-render --features pdf`) so that the default build keeps the workspace's
minimum, Rust 1.90; without it `resolve` reports every `pdf` asset as an error.
The renderer does not need the feature: it draws a `pdf` asset from its pinned cache image.

## Third-party code of the `pdf` feature

| Crate or data | Version | Licence | Use |
|---|---|---|---|
| [hayro](https://crates.io/crates/hayro) (with hayro-interpret, hayro-syntax) | 0.8 | Apache-2.0 OR MIT | PDF parsing, page rendering, glyph positions and Unicode |
| Foxit standard-font substitutes, bundled by hayro-interpret (`embed-fonts`) | from PDFium | BSD-3-Clause | the 14 standard fonts when a PDF does not embed them |
| CGATS001Compat-v2-micro.icc, bundled by hayro-interpret | | CC0-1.0 | CMYK conversion |
| LAB.icc, bundled by hayro-interpret | generated with LCMS2 | (generated profile) | Lab conversion |
| [unicode-normalization](https://crates.io/crates/unicode-normalization) | 0.1 | MIT OR Apache-2.0 | NFKC of page text and phrases |
| zlib-rs, libz-rs-sys (flate2's `zlib-rs` backend, turned on by hayro-syntax) | | Zlib | PDF stream inflate; through Cargo feature unification also the inflate backend of sr-geo and sr-volume |

Their transitive dependencies (kurbo, vello_cpu, skrifa, moxcms and others) are listed with their licences by
`cargo metadata`; the PDFium notice that the BSD-3-Clause licence asks binaries to carry is in the repository's
`THIRD-PARTY-NOTICES.md`.
