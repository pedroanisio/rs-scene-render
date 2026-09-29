#!/usr/bin/env python3
"""Writes crates/sr-gpu/tests/fixtures/fonts/wght-test.ttf: a minimal variable TrueType font
for the text-animator variation test, so the test needs no installed font.

Family "SR Wght Test", one axis wght 100..900 (default 400), glyphs .notdef, space and "I".
"I" is a bar 600 units tall whose stem is 100 units wide at the default and widens by 150
units at wght 900 (700, normalised to 0.6, gives +90).

Needs fontTools (pip install fonttools).
"""
import os

from fontTools.fontBuilder import FontBuilder
from fontTools.pens.ttGlyphPen import TTGlyphPen
from fontTools.ttLib.tables.TupleVariation import TupleVariation

HERE = os.path.dirname(os.path.abspath(__file__))
OUT = os.path.join(HERE, "..", "..", "crates", "sr-gpu", "tests", "fixtures", "fonts", "wght-test.ttf")


def rect(x0, y0, x1, y1):
    pen = TTGlyphPen(None)
    pen.moveTo((x0, y0))
    pen.lineTo((x0, y1))
    pen.lineTo((x1, y1))
    pen.lineTo((x1, y0))
    pen.closePath()
    return pen.glyph()


def main():
    fb = FontBuilder(1000, isTTF=True)
    order = [".notdef", "space", "I"]
    fb.setupGlyphOrder(order)
    fb.setupCharacterMap({0x20: "space", 0x49: "I"})
    fb.setupGlyf({".notdef": rect(50, 0, 450, 700), "space": TTGlyphPen(None).glyph(), "I": rect(100, 0, 200, 600)})
    fb.setupHorizontalMetrics({".notdef": (500, 50), "space": (300, 0), "I": (500, 100)})
    fb.setupHorizontalHeader(ascent=800, descent=-200)
    # fontdb skips faces without a PostScript name
    fb.setupNameTable(
        {
            "familyName": "SR Wght Test",
            "styleName": "Regular",
            "uniqueFontIdentifier": "SR Wght Test Regular",
            "fullName": "SR Wght Test Regular",
            "psName": "SRWghtTest-Regular",
            "version": "Version 1.000",
        }
    )
    fb.setupOS2(sTypoAscender=800, sTypoDescender=-200, usWinAscent=800, usWinDescent=200)
    fb.setupPost()
    fb.setupFvar(axes=[("wght", 100, 400, 900, "Weight")], instances=[])
    # "I" points in outline order: (x0,y0) (x0,y1) (x1,y1) (x1,y0), then 4 phantom points;
    # at wght 900 the right edge moves out by 150 and the advance grows with it
    grow = [(0, 0), (0, 0), (150, 0), (150, 0), (0, 0), (150, 0), (0, 0), (0, 0)]
    fb.setupGvar({"I": [TupleVariation({"wght": (0, 1.0, 1.0)}, grow)]})
    os.makedirs(os.path.dirname(OUT), exist_ok=True)
    fb.save(OUT)
    print("wrote", os.path.normpath(OUT))


if __name__ == "__main__":
    main()
