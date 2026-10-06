# Third-party notices

scene-render is MIT-licensed (`LICENSE`). This file records third-party code and data that the binaries carry and
whose licences ask for a notice. Crates linked as ordinary dependencies are listed with their licences by
`cargo metadata`.

## PDF support (sr-resolve feature `pdf`, SREP 17)

- **hayro, hayro-interpret, hayro-syntax** 0.8 (Apache-2.0 OR MIT), by Laurenz Stampfl and contributors:
  PDF parsing and rendering in the resolve step of `pdf` assets.
- **unicode-normalization** 0.1 (MIT OR Apache-2.0): NFKC of region phrases.
- **zlib-rs** and **libz-rs-sys** (Zlib licence): hayro-syntax turns on flate2's `zlib-rs` backend, and Cargo
  feature unification makes it the inflate backend of the whole build (sr-geo and sr-volume decode with it too).
- **Foxit standard fonts** (FoxitSans, FoxitSerif, FoxitFixed with their bold and italic styles, FoxitSymbol,
  FoxitDingbats), extracted from PDFium and embedded by hayro-interpret as substitutes for the 14 standard PDF
  fonts. BSD-3-Clause:

  ```text
  Copyright 2014 PDFium Authors. All rights reserved.

  Redistribution and use in source and binary forms, with or without
  modification, are permitted provided that the following conditions are
  met:

     * Redistributions of source code must retain the above copyright
  notice, this list of conditions and the following disclaimer.
     * Redistributions in binary form must reproduce the above
  copyright notice, this list of conditions and the following disclaimer
  in the documentation and/or other materials provided with the
  distribution.
     * Neither the name of Google Inc. nor the names of its
  contributors may be used to endorse or promote products derived from
  this software without specific prior written permission.

  THIS SOFTWARE IS PROVIDED BY THE COPYRIGHT HOLDERS AND CONTRIBUTORS
  "AS IS" AND ANY EXPRESS OR IMPLIED WARRANTIES, INCLUDING, BUT NOT
  LIMITED TO, THE IMPLIED WARRANTIES OF MERCHANTABILITY AND FITNESS FOR
  A PARTICULAR PURPOSE ARE DISCLAIMED. IN NO EVENT SHALL THE COPYRIGHT
  OWNER OR CONTRIBUTORS BE LIABLE FOR ANY DIRECT, INDIRECT, INCIDENTAL,
  SPECIAL, EXEMPLARY, OR CONSEQUENTIAL DAMAGES (INCLUDING, BUT NOT
  LIMITED TO, PROCUREMENT OF SUBSTITUTE GOODS OR SERVICES; LOSS OF USE,
  DATA, OR PROFITS; OR BUSINESS INTERRUPTION) HOWEVER CAUSED AND ON ANY
  THEORY OF LIABILITY, WHETHER IN CONTRACT, STRICT LIABILITY, OR TORT
  (INCLUDING NEGLIGENCE OR OTHERWISE) ARISING IN ANY WAY OUT OF THE USE
  OF THIS SOFTWARE, EVEN IF ADVISED OF THE POSSIBILITY OF SUCH DAMAGE.
  ```

  Original code copyright 2014 Foxit Software Inc. (http://www.foxitsoftware.com).
- **CGATS001Compat-v2-micro.icc** (CC0-1.0), from https://github.com/saucecontrol/Compact-ICC-Profiles, embedded by
  hayro-interpret.
- **LAB.icc**, generated with LCMS2, embedded by hayro-interpret.
