//! Machine-readable codes through rxing's encoders: QR, Data Matrix,
//! PDF417, EAN-13, UPC-A and Code 128, drawn as merged module rectangles.

use rxing::{BarcodeFormat, EncodeHints, MultiFormatWriter, Writer};
use sr_vector::scene::{FillRule, Paint};
use sr_vector::shapes;

use crate::glyph::Drawing;

/// Encodes `data` and draws it into a `size` box with a quiet zone of `quiet` modules.
#[allow(clippy::too_many_arguments)]
pub fn draw(
    kind: &str,
    data: &str,
    size: [f64; 2],
    fg: Paint,
    bg: Paint,
    ecc: &str,
    quiet: u32,
    tol: f64,
) -> Result<Drawing, String> {
    let format = match kind {
        "qr" => BarcodeFormat::QR_CODE,
        "datamatrix" => BarcodeFormat::DATA_MATRIX,
        "pdf417" => BarcodeFormat::PDF_417,
        "ean13" => BarcodeFormat::EAN_13,
        "upc-a" => BarcodeFormat::UPC_A,
        "code128" => BarcodeFormat::CODE_128,
        other => return Err(format!("unknown code kind {other}")),
    };
    let hints = EncodeHints {
        ErrorCorrection: match kind {
            "qr" => Some(ecc.to_string()),
            "pdf417" => Some(
                match ecc {
                    "L" => "2",
                    "M" => "4",
                    "Q" => "6",
                    _ => "8",
                }
                .to_string(),
            ),
            _ => None,
        },
        Margin: Some("0".into()),
        ..Default::default()
    };
    let m = MultiFormatWriter.encode_with_hints(data, &format, 0, 0, &hints).map_err(|e| format!("{kind}: {e}"))?;
    let (mw, mh) = (m.getWidth(), m.getHeight());
    let linear = matches!(kind, "ean13" | "upc-a" | "code128");
    let q = quiet as f64;
    let (cols, rows) = (mw as f64 + 2.0 * q, if linear { 1.0 } else { mh as f64 + 2.0 * q });
    // square modules for 2D codes, centred; linear codes fill the height
    let (sx, sy) = if linear {
        (size[0] / cols, size[1])
    } else {
        let s = (size[0] / cols).min(size[1] / rows);
        (s, s)
    };
    let (ox, oy) = ((size[0] - cols * sx) * 0.5, if linear { 0.0 } else { (size[1] - rows * sy) * 0.5 });
    let mut d = Drawing::default();
    d.scene.fill(&shapes::rect(0.0, 0.0, size[0], size[1], [0.0; 4]), FillRule::NonZero, bg, 1.0, tol);
    let mut path = sr_vector::Path::default();
    let y_rows = if linear { 1 } else { mh };
    for y in 0..y_rows {
        let mut x = 0;
        while x < mw {
            if !m.get(x, y) {
                x += 1;
                continue;
            }
            let x0 = x;
            while x < mw && m.get(x, y) {
                x += 1;
            }
            let (rx, ry) = (ox + (q + x0 as f64) * sx, oy + if linear { 0.0 } else { (q + y as f64) * sy });
            path.extend(&shapes::rect(rx, ry, (x - x0) as f64 * sx, if linear { size[1] } else { sy }, [0.0; 4]));
        }
    }
    d.scene.fill(&path, FillRule::NonZero, fg, 1.0, tol);
    Ok(d)
}
