#!/usr/bin/env python3
"""Builds tests/corpus from tools/kitchen_sink.xml.in.

* valid/kitchen-sink.scene.xml — the template with SHA-256 digests filled in;
* valid/*.scene.xml — small documents for version and edge semantics;
* invalid/<case>.scene.xml — one mutation of the kitchen sink per assert id
  and per structural/asset code, headed by `<!-- expect: CODES -->`;
* manifest.json — file -> expected codes.

Every document's XSD and Schematron verdict is checked against lxml
(tools/oracle.py). Asset codes (A01–A06) are outside the oracle's scope.
"""
import hashlib, json, re, sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).parent))
from oracle import verdict

TOOLS = Path(__file__).parent
CORPUS = TOOLS.parent / "tests" / "corpus"
MEDIA = CORPUS / "media"

base = re.sub(r"\{sha:([^}]+)\}", lambda m: hashlib.sha256((MEDIA / m.group(1)).read_bytes()).hexdigest(),
              (TOOLS / "kitchen_sink.xml.in").read_text())
MINIMAL = '<scene version="1.1"><project width="1920" height="1080" fps="30" duration="5"/><composition/></scene>\n'

def sub(old, new, count=1):
    def f(t):
        assert old in t, f"mutation anchor not found: {old!r}"
        return t.replace(old, new, count)
    return f

def chain(*fs):
    def f(t):
        for g in fs:
            t = g(t)
        return t
    return f

def ins_comp(xml):  # insert nodes at the start of <composition>
    return sub("<composition>\n", "<composition>\n    " + xml + "\n")

def v10(body, extra_sections=""):
    return f'<scene version="1.0"><project width="640" height="360" fps="25" duration="2"/>{extra_sections}<composition>{body}</composition></scene>\n'

# (name, expected codes, transform-or-document, asset codes the oracle cannot see)
CASES = [
    # ---- structure
    ("s01-root", ["S01"], lambda t: MINIMAL.replace("scene", "movie")),
    ("s02-unknown-element", ["S02"], ins_comp('<lyer id="z" asset="logo"/>')),
    ("s02-out-of-order", ["S02"], sub("  <composition>", "  <project width=\"1\" height=\"1\" fps=\"1\" duration=\"1\"/>\n  <composition>")),
    ("s03-missing-child", ["S03"], lambda t: '<scene version="1.1"><project width="1" height="1" fps="1" duration="1"/></scene>\n'),
    ("s03-empty-animate", ["S03"], ins_comp('<layer id="z" asset="logo"><animate property="x"/></layer>')),
    ("s04-unknown-attribute", ["S04"], sub('<bus id="fx"/>', '<bus id="fx" gian="3"/>')),
    ("s05-missing-attribute", ["S05"], sub('<light id="key" type="directional"/>', '<light id="key"/>')),
    ("s06-enum", ["S06"], sub('codec="h264"', 'codec="h266"')),
    ("s06-bound", ["S06"], sub('<layer id="shot-a" asset="logo" end="4"/>', '<layer id="shot-a" asset="logo" end="4" opacity="1.5"/>')),
    ("s06-pattern-fps", ["S06"], sub('fps="12"', 'fps="12/0"')),
    ("s06-colour", ["S06"], sub('<stop offset="1" color="#220044"/>', '<stop offset="1" color="1.2,0,0"/>')),
    ("s06-length", ["S06"], sub('group id="hero" x="50%"', 'group id="hero" x="50px"')),
    ("s06-double", ["S06"], sub('<marker id="drop" time="4.2"', '<marker id="drop" time="4,2"')),
    ("s06-integer", ["S06"], sub('seed="7"', 'seed="-7"')),
    ("s06-idrefs-empty", ["S06"], sub('<adjustment id="grade-all" effects="grade"/>', '<adjustment id="grade-all" effects=" "/>')),
    ("s07-text", ["S07"], sub("<master/>", "<master>loud</master>")),
    ("s09-duplicate-id", ["S09"], sub('<bus id="fx"/>', '<bus id="fx"/>\n    <bus id="fx"/>')),
    ("s10-dangling-idref", ["S10"], sub('<audiogram id="wave" source="music"', '<audiogram id="wave" source="musik"')),
    # ---- version check
    ("v1-sections", ["V1"], lambda t: v10("", '<markers><marker time="1"/></markers>')),
    ("v2-outputs", ["V2"], lambda t: '<scene version="1.0"><project width="640" height="360" fps="25" duration="2"/><output path="a.mp4" codec="h264"/><output path="b.mp4" codec="h264"/><composition/></scene>\n'),
    ("v3-elements", ["V3"], lambda t: v10('<sequence id="s"/>')),
    ("v4-assets", ["V4"], lambda t: '<scene version="1.0"><project width="640" height="360" fps="25" duration="2"/><assets><formula id="f" tex="x" width="10" height="10"/></assets><composition/></scene>\n'),
    # ---- co-occurrence
    ("c1", ["C1"], sub('<vector id="icon" shape="path" path="M0 0 L10 0 L5 10 Z"', '<vector id="icon" shape="path"')),
    ("c2", ["C2"], sub('<vector id="icon" shape="path" path="M0 0 L10 0 L5 10 Z"', '<vector id="icon" shape="svg"')),
    ("c3", ["C3"], sub('<shape id="dot" shape="ellipse"', '<shape id="dot" shape="path"')),
    ("c4", ["C4"], sub('<mask type="ellipse" width="800" height="800"/>', '<mask type="path"/>')),
    ("c5", ["C5"], sub('<mask type="ellipse" width="800" height="800"/>', '<mask type="ellipse" width="800"/>')),
    ("c6", ["C6"], sub('primitive="mesh" mesh="robot"', 'primitive="mesh"')),
    ("c7", ["C7"], sub('primitive="mesh" mesh="robot"', 'primitive="text"')),
    ("c8", ["C8"], sub('primitive="mesh" mesh="robot"', 'primitive="extrude"')),
    ("c9", ["C9"], sub('<constraint id="pin1" type="pin" a="sparks"', '<constraint id="pin1" type="pin" a="sparks" b="bot"')),
    ("c10", ["C10"], sub('a="sparks" x="0" y="0"', 'a="sparks" x="0"')),
    ("c11", ["C11"], sub('<constraint id="pin1" type="pin" a="sparks" x="0" y="0"/>', '<constraint id="pin1" type="spring" a="sparks"/>')),
    ("c12", ["C12"], sub('<text id="rich" width="800" height="100" size="40">', '<text id="rich" width="800" height="100" size="40" text="both">')),
    ("c13", ["C13"], sub('minSize="24" maxSize="96"', 'minSize="120" maxSize="96"')),
    ("c13-exponent", ["C13"], sub('minSize="24" maxSize="96"', 'minSize="1.2e2" maxSize="96"')),
    ("c14", ["C14"], sub('letterSpacing="2"', 'letterSpacing="400"')),
    ("c15", ["C15"], sub('<layer id="spinner" asset="spin">', '<layer id="spinner" asset="spin" speed="2">')),
    ("c15-reverse-literal", ["C15"], sub('<layer id="spinner" asset="spin">', '<layer id="spinner" asset="spin" reverse="0">')),
    ("c16", ["C16"], sub('fit="contain" boxWidth="400" boxHeight="300"', 'fit="contain" boxWidth="400"')),
    ("c17", ["C17"], sub('<repeat id="grid" count="3">', '<repeat id="grid">')),
    ("c18", ["C18"], sub('<repeat id="list" over="rows">', '<repeat id="list" over="headline">')),
    ("c19", ["C19"], sub('<effect id="grade" type="lut" src="../media/grade.cube"/>', '<effect id="grade" type="lut"/>')),
    ("c20", ["C20"], sub('<transition id="xf" type="crossfade" from="shot-a" to="shot-b"/>', '<transition id="xf" type="crossfade"/>')),
    ("c21", ["C21"], sub('type="crossfade" from="shot-a"', 'type="shader" from="shot-a"')),
    ("c22", ["C22"], sub('type="crossfade" from="shot-a"', 'type="luma" from="shot-a"')),
    ("c23", ["C23"], sub('from="shot-a" to="shot-b"', 'from="hero" to="shot-b"')),
    ("c24", ["C24"], sub('from="shot-a" to="shot-b"', 'from="shot-a" to="bot"')),
    ("c25", ["C25"], ins_comp('<layer id="z" asset="logo"><deform><modifier type="skin"/></deform></layer>')),
    ("c26", ["C26"], ins_comp('<layer id="z" asset="logo"><deform><modifier type="corner-pin" corners="0 0 1 0 1 1 0"/></deform></layer>')),
    ("c27", ["C27"], ins_comp('<layer id="z" asset="logo"><transformConstraint type="follow-path"/></layer>')),
    ("c28", ["C28"], ins_comp('<layer id="z" asset="logo"><transformConstraint type="look-at"/></layer>')),
    ("c29", ["C29"], sub('<transformConstraint type="track" target="face"/>', '<transformConstraint type="track" target="hero"/>')),
    ("c30", ["C30"], sub('preset="custom" top="0.05"', 'preset="custom"')),
    ("c31", ["C31"], sub('<captionTrack id="en" language="en-US" safeArea="broadcast">', '<captionTrack id="en" language="en-US" safeArea="broadcast" src="../media/transcript.vtt">')),
    ("c32", ["C32"], lambda t: re.sub(r'(transcribe="vo-track" cache="\.\./media/transcript\.vtt") cacheSha256="[0-9a-f]+"', r'\1', t)),
    ("c33", ["C33"], sub('transcribe="vo-track"', 'transcribe="music"')),
    ("c34", ["C34"], sub("<master/>", "<master/>\n    <master/>")),
    ("c35", ["C35"], lambda t: MINIMAL.replace("<composition/>", '<composition/><audioMix><bus id="b"/></audioMix>')),
    ("c36", ["C36"], sub('<stop offset="0" color="#ffffff"/>\n      <stop offset="1" color="#ffffff00"/>', '<stop offset="0" color="#ffffff"/>')),
    ("c37", ["C37"], sub('<stop offset="0.5" color="var(--brand)"/>', '<stop offset="0.5" color="var(--brand)"/>\n      <stop offset="0.2" color="#000000"/>')),
    ("c38", ["C38"], sub('<key time="1" value="0.4"/>', '<key time="0.5" value="0.4"/>\n          <key time="0.25" value="0.5"/>')),
    ("c39", ["C39"], sub('interpolation="steps" steps="4"', 'interpolation="steps"')),
    ("c40", ["C40"], sub('interpolation="cubic-bezier" bezier="0.25,0.1,0.25,1"', 'interpolation="cubic-bezier"')),
    ("c41", ["C41"], sub('kind="speech" provider="acme-tts" model="v2" prompt="Welcome"', 'kind="music" provider="acme-tts" model="v2"')),
    ("c42", ["C42"], sub('codec="h264" layout="square"', 'codec="h264" proresProfile="hq" layout="square"')),
    ("c43", ["C43"], sub('codec="h264" layout="square"', 'codec="h264" alpha="true" layout="square"')),
    ("c44", ["C44"], sub('start="0" end="12"', 'start="5" end="4"')),
    # ---- references
    ("r1", ["R1"], sub('<layer id="shot-a" asset="logo"', '<layer id="shot-a" asset="rows"')),
    ("r2", ["R2"], sub('<layer id="title-layer" asset="title"', '<layer id="title-layer" asset="logo"')),
    ("r3", ["R3"], sub('audioBus="fx"', 'audioBus="music-track"')),
    ("r4", ["R4"], sub('material="chrome"', 'material="logo"')),
    ("r5", ["R5"], sub('mesh="robot" material', 'mesh="logo" material')),
    ("r6", ["R6"], sub('effects="glow-fx grade"', 'effects="glow-fx key"')),
    ("r6-duplicate", ["R6"], sub('effects="glow-fx grade"', 'effects="grade grade"')),
    ("r7", ["R7"], sub('lights="key dome"', 'lights="key sparks"')),
    ("c38-exponent", ["C38"], sub('<key time="1" value="0.4"/>', '<key time="1" value="0.4"/>\n          <key time="5e-1" value="0.5"/>')),
    ("r8", ["R8"], sub('matte="bg"', 'matte="logo"')),
    ("r9", ["R9"], sub('matte="bg"', 'matte="title-layer"')),
    ("r10", ["R10"], sub('matte="bg" parent="hero"', 'matte="bg" parent="title-layer"')),
    ("r11", ["R11"], sub('symbol="card"', 'symbol="hero"')),
    ("r12", ["R12"], sub('layout="square"', 'layout="broadcast"')),
    ("r13", ["R13"], sub('variant="dark"', 'variant="rows"')),
    ("r14", ["R14"], sub('burnCaptions="en"', 'burnCaptions="vo-track"')),
    ("r15", ["R15"], sub('<audioTrack id="clip-audio" asset="clip"/>', '<audioTrack id="clip-audio" asset="logo"/>')),
    ("r15-video-without-audio", ["R15"], sub('hasAudio="true"', 'hasAudio="false"')),
    ("r16", ["R16"], sub('bus="fx" duckUnder', 'bus="vo-track" duckUnder')),
    ("r17", ["R17"], sub('duckUnder="vo-track"', 'duckUnder="vo-track music-track"')),
    ("r18", ["R18"], sub('<bind param="headline"', '<bind param="rows"')),
    ("r19", ["R19"], sub('<layout id="square" width="1080" height="1080" safeArea="broadcast">', '<layout id="square" width="1080" height="1080" safeArea="en">')),
    ("r20", ["R20"], sub('focusTarget="hero" target="hero"', 'focusTarget="hero" target="card-bg"')),
    ("r21", ["R21"], sub('startMarker="drop"', 'startMarker="hero"')),
    ("r21-key", ["R21"], sub('marker="drop"', 'marker="hero"')),
    ("r22", ["R22"], sub('basedOn="heading"', 'basedOn="title"')),
    ("r22-style", ["R22"], sub('size="72" style="heading"', 'size="72" style="logo"')),
    ("r23", ["R23"], sub('fontAsset="inter"', 'fontAsset="logo"')),
    ("r36", ["R36"], sub('<geoLayer geo="places"', '<geoLayer geo="logo"')),
    ("r37", ["R37"], sub('<route geo="places"', '<route geo="logo"')),
    ("r26", ["R26"], sub('fit="places"', 'fit="places logo"')),
    ("c45", ["C45"], sub('<route points="0,0 20,20"', '<route')),
    # ---- schema 1.1.3
    ("v6-elements", ["V6"], lambda t: v10('<particleEmitter id="p"><burst time="1" count="2"/></particleEmitter>')),
    ("v7-primitives", ["V7"], lambda t: v10('<object3D id="o" primitive="torus"/>')),
    ("r10-any-node", ["R10"], ins_comp('<flock id="fk" width="10" height="10" parent="fk"/>')),
    ("r30-outline", ["R30-outline"], sub('outline="#FFFFFF"', 'outline="url(#nope)"')),
    ("r31-baseColor", ["R31-baseColor"], sub('<material id="chrome" metallic="1"', '<material id="chrome" baseColor="var(--nope)" metallic="1"')),
    ("c50", ["C50"], sub('<particleEmitter id="sparks" color="#ffcc00">', '<particleEmitter id="sparks" color="#ffcc00" shape="sprite">')),
    ("r32", ["R32"], sub('<particleEmitter id="sparks" color="#ffcc00">', '<particleEmitter id="sparks" color="#ffcc00" sprite="music">')),
    ("r33", ["R33"], ins_comp('<erosion id="er" width="10" height="10" heightmap="music"/>')),
    ("r34", ["R34"], sub('<particleEmitter id="sparks" color="#ffcc00">', '<particleEmitter id="sparks" color="#ffcc00" forceFields="gravity music">')),
    ("r35", ["R35"], sub('<pin lon="5" lat="5" label="Here"/>', '<pin lon="5" lat="5" label="Here" textStyle="logo"/>')),
    ("c51", ["C51"], ins_comp('<object3D id="cl" primitive="clay"/>')),
    ("c52", ["C52"], ins_comp('<fluid id="fl" width="10" height="10"><fluidSource start="2" end="1"/></fluid>')),
    ("c53", ["C53"], sub('palette="#F7FBFF #08306B"', 'palette="#F7FBFF #08306B" domain="3 1"')),
    ("s06-latitude", ["S06"], sub('<pin lon="5" lat="5"', '<pin lon="5" lat="95"')),
    ("r27", ["R27"], sub('<basemap tiles="streets"', '<basemap tiles="places"')),
    ("c47", ["C47"], sub('<object3D id="earth" primitive="globe" map="atlas"', '<object3D id="earth" primitive="globe"')),
    ("r28", ["R28"], sub('primitive="globe" map="atlas"', 'primitive="globe" map="streets"')),
    ("r29", ["R29"], sub('terrain="streets"', 'terrain="atlas"')),
    ("c46", ["C46"], sub('<tiles id="streets" src="../media/streets.pmtiles"', '<tiles id="streets"')),
]
for attr, anchor, repl in [
    ("fill", 'shape="rounded-rect" width="400" height="300" fill="var(--bg)"', 'shape="rounded-rect" width="400" height="300" fill="url(#logo)"'),
    ("stroke", 'stroke="url(#glow)"', 'stroke="url(#nope)"'),
    ("color", '<span color="url(#sunset)"', '<span color="url(#sun)"'),
    ("background", '<symbol id="card" width="400" height="300">', '<symbol id="card" width="400" height="300" background="url(#x)">'),
    ("paint", '<effect id="glow-fx" type="glow"', '<effect id="glow-fx" type="glow" paint="url(#nope)"'),
    ("activeColor", '<captionTrack id="en" language="en-US"', '<captionTrack id="en" language="en-US" activeColor="url(#nope)"'),
    ("highlight", '<text id="title" text', '<text id="title" highlight="url(#nope)" text'),
    ("strokeColor", '<text id="title" text', '<text id="title" strokeColor="url(#nope)" text'),
]:
    CASES.append((f"r24-{attr}", [f"R24-{attr}"], sub(anchor, repl)))
for attr, anchor, repl in [
    ("fill", 'fill="var(--bg)"', 'fill="var(--missing)"'),
    ("stroke", 'stroke="var(--brand)"', 'stroke="var(--brnd)"'),
    ("color", 'color="var(--brand)" weight', 'color="var(--none)" weight'),
    ("background", 'background="var(--bg)"', 'background="var(--bgg)"'),
    ("paint", '<generator id="noise" kind="fractal-noise"', '<generator id="noise" paint="var(--nope)" kind="fractal-noise"'),
    ("activeColor", '<captionTrack id="en" language="en-US"', '<captionTrack id="en" language="en-US" activeColor="var(--nope)"'),
    ("highlight", '<text id="title" text', '<text id="title" highlight="var(--nope)" text'),
    ("strokeColor", '<span color="#ffffff">', '<span color="#ffffff" strokeColor="var(--nope)">'),
]:
    CASES.append((f"r25-{attr}", [f"R25-{attr}"], sub(anchor, repl)))

# the paint and colour attributes 1.1.3 added to the url(#) and var(--) checks: one insertion point each
EXTRA_REF_SITES = {
    "paint2": '<generator id="noise" kind="fractal-noise"',
    "foreground": '<code id="qr" kind="qr"',
    "noData": '<geoLayer geo="places"',
    "headFill": '<route points="0,0 20,20"',
    "attenuationColor": '<material id="chrome"',
    "baseColor": '<material id="chrome"',
    "emissive": '<material id="chrome"',
    "sheenColor": '<material id="chrome"',
    "specularColor": '<material id="chrome"',
    "colorEnd": '<particleEmitter id="sparks"',
    "keyColor": '<effect id="glow-fx"',
    "shadowColor": '<span color="#ffffff"',
}

def with_attr(attr, value):
    if attr in ("colorHigh", "colorLow"):
        return ins_comp(f'<erosion id="er" width="10" height="10" {attr}="{value}"/>')
    if attr == "outline":  # the map already has an outline: replace it
        return sub('outline="#FFFFFF"', f'outline="{value}"')
    site = EXTRA_REF_SITES[attr]
    return sub(site, f'{site} {attr}="{value}"')

for attr in ["colorEnd", "colorHigh", "colorLow", "headFill", "noData", "outline", "paint2"]:
    if attr != "outline":
        CASES.append((f"r30-{attr}", [f"R30-{attr}"], with_attr(attr, "url(#nope)")))
for attr in ["attenuationColor", "baseColor", "colorEnd", "colorHigh", "colorLow", "emissive", "foreground", "headFill",
             "keyColor", "noData", "outline", "paint2", "shadowColor", "sheenColor", "specularColor"]:
    if attr != "baseColor":
        CASES.append((f"r31-{attr}", [f"R31-{attr}"], with_attr(attr, "var(--nope)")))

# Asset cases: the oracle sees a valid document; the Rust verifier must not.
ASSET_CASES = [
    ("a01-missing-file", ["A01"], sub('src="../media/clip.mp4"', 'src="../media/clip-missing.mp4"')),
    ("a02-hash-mismatch", ["A02"], lambda t: re.sub(r'(src="\.\./media/music\.wav" sha256=")[0-9a-f]{4}', r'\g<1>0000', t)),
    ("a02-generated-cache", ["A02"], lambda t: re.sub(r'(cache="\.\./media/voice\.wav" cacheSha256=")[0-9a-f]{4}', r'\g<1>ffff', t)),
    ("a04-sequence-frames", ["A04"], sub('first="1" last="5"', 'first="1" last="9"')),
]
WARN_CASES = [
    ("a03-remote", ["A03"], sub('src="../media/clip.mp4"', 'src="https://cdn.example.com/clip.mp4"')),
    ("a04-sequence-hold", ["A04"], sub('first="1" last="5"', 'first="1" last="9" missingFrame="hold"')),
    ("w01-non-finite", ["W01"], sub('<marker id="drop" time="4.2"', '<marker id="drop" time="4.2" duration="1"/>\n    <marker id="late" time="INF"')),
]

VALID = {
    "kitchen-sink": base,
    "minimal": MINIMAL,
    "version-1.0": v10('<layer id="l" asset="img" x="10" y="10"/>').replace("<composition>", '<assets><image id="img" src="../media/logo.png" width="16" height="16"/></assets><composition>'),
}

# libxml2 (the oracle) skips two XSD 1.0 checks that sr-model enforces:
# IDREF resolution (Part 1, Validation Rule: Validation Root Valid (ID/IDREF
# Table)) and minLength 1 on whitespace-only IDREFS lists.
ORACLE_BLIND = {"s06-idrefs-empty", "s10-dangling-idref"}

def write(path, text, expect):
    header = f"<!-- expect: {' '.join(expect) if expect else 'valid'} -->\n"
    body = text.split("\n", 1)[1] if text.startswith("<?xml") else text
    decl = '<?xml version="1.0" encoding="UTF-8"?>\n'
    path.write_text(decl + header + body)
    return (decl + header + body).encode()

def main():
    manifest = {"valid": {}, "invalid": {}, "warnings": {}}
    for d in ("valid", "invalid"):
        for f in (CORPUS / d).glob("*.xml"):
            f.unlink()
    ok = True
    for name, text in VALID.items():
        data = write(CORPUS / "valid" / f"{name}.scene.xml", text, [])
        xsd, sch = verdict(data)
        if xsd or sch:
            ok = False
            print(f"valid/{name}: oracle disagrees: {xsd} {sch}")
        manifest["valid"][f"{name}.scene.xml"] = []
    for name, expect, fn in CASES:
        text = fn(base)
        data = write(CORPUS / "invalid" / f"{name}.scene.xml", text, expect)
        xsd, sch = verdict(data)
        got = sorted(set(i for i, _ in sch) | ({"XSD"} if xsd else set()))
        want = sorted(set(e for e in expect if not e.startswith("S")) | ({"XSD"} if any(e.startswith("S") for e in expect) else set()))
        if name in ORACLE_BLIND:
            want = sorted(set(e for e in expect if not e.startswith("S")))
        if got != want:
            ok = False
            print(f"invalid/{name}: expected {want}, oracle says {got}: {xsd[:2]}")
        manifest["invalid"][f"{name}.scene.xml"] = expect
    for name, expect, fn in ASSET_CASES + WARN_CASES:
        is_warn = (name, expect, fn) in WARN_CASES
        text = fn(base)
        data = write(CORPUS / ("valid" if is_warn else "invalid") / f"{name}.scene.xml", text, expect)
        xsd, sch = verdict(data)
        if xsd or sch:
            ok = False
            print(f"{name}: oracle should accept: {xsd} {sch}")
        manifest["warnings" if is_warn else "invalid"][f"{name}.scene.xml"] = expect
    (CORPUS / "manifest.json").write_text(json.dumps(manifest, indent=2, sort_keys=True) + "\n")
    n = sum(len(v) for v in manifest.values())
    print(f"{n} documents written; oracle agreement: {'yes' if ok else 'NO'}")
    return 0 if ok else 1

if __name__ == "__main__":
    sys.exit(main())
