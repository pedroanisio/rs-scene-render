#!/usr/bin/env python3
"""Build a deterministic, authored square-ring O for solid geometry tests.

Generated with Codex. Requires fontTools only when regenerating the fixture.
The glyph has outer bounds 600 by 800 and a 200 by 400 counter at (200, 200).
"""
from pathlib import Path

from fontTools.fontBuilder import FontBuilder
from fontTools.pens.ttGlyphPen import TTGlyphPen


def glyph(contours):
    pen = TTGlyphPen(None)
    for points in contours:
        pen.moveTo(points[0])
        for point in points[1:]:
            pen.lineTo(point)
        pen.closePath()
    return pen.glyph()


font = FontBuilder(1000, isTTF=True)
font.setupGlyphOrder([".notdef", "space", "O"])
font.setupCharacterMap({32: "space", 79: "O"})
font.setupGlyf({
    ".notdef": glyph([]),
    "space": glyph([]),
    "O": glyph([
        [(0, 0), (0, 800), (600, 800), (600, 0)],
        [(200, 200), (400, 200), (400, 600), (200, 600)],
    ]),
})
font.setupHorizontalMetrics({name: (700, 0) for name in [".notdef", "space", "O"]})
font.setupHorizontalHeader(ascent=800, descent=-200)
font.setupNameTable({"familyName": "SR Solid Test", "styleName": "Regular",
                    "uniqueFontIdentifier": "SR Solid Test Regular", "fullName": "SR Solid Test Regular",
                    "psName": "SRSolidTest-Regular"})
font.setupOS2(sTypoAscender=800, sTypoDescender=-200, usWinAscent=800, usWinDescent=200)
font.setupPost()
font.setupMaxp()
font.font["head"].created = font.font["head"].modified = 2082844800
font.font.recalcTimestamp = False
output = Path(__file__).resolve().parents[2] / "crates/sr-eval/tests/fixtures/solid.ttf"
output.parent.mkdir(parents=True, exist_ok=True)
font.save(output)
