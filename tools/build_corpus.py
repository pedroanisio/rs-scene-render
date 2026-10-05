#!/usr/bin/env python3
"""Builds tests/corpus from tools/kitchen_sink.xml.in.

* valid/kitchen-sink.scene.xml — the template with SHA-256 digests filled in;
* valid/*.scene.xml — small documents for version and edge semantics;
* invalid/<case>.scene.xml — one mutation of the kitchen sink per assert id
  and per structural/asset code, headed by `<!-- expect: CODES -->`;
* manifest.json — file -> expected codes.

Every document's XSD and Schematron verdict is checked against lxml
(tools/oracle.py). Asset codes (A01–A07) are outside the oracle's scope.
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
VOLUME = '''<scene version="1.3"><project width="32" height="32" fps="30" duration="1"/>
<assets><volume id="smoke" src="../media/uniform.srvol" boundsMinX="0" boundsMinY="0" boundsMinZ="0" boundsMaxX="32" boundsMaxY="32" boundsMaxZ="2"/></assets>
<composition><object3D id="cloud" primitive="volume" volume="smoke"><medium albedo="#000000" emissionColor="#FF4000" emissionScale="0.2"/></object3D></composition></scene>\n'''
VOLUME_SEQUENCE = VOLUME.replace('src="../media/uniform.srvol"',
    'src="../media/uniform-%02d.srvol" first="0" last="0" fps="24000/1001" interpolation="linear"')

VOLUME_BAKED = VOLUME.replace('src="../media/uniform.srvol"', 'src="../media/baked-volume/manifest.srvseq" format="srvseq" sha256="68cb87c6720ed0ac07c87a7ae48f5a3a69c326c085f8dcb655be7738035432d8"')

PYRO = '<scene version="1.3"><project width="32" height="32" fps="30" duration="1"/>\n<composition><object3D id="cloud" primitive="volume"><pyro width="8" height="8" depth="8" voxelSize="1" dt="0.1"><pyroSource shape="sphere" radius="2" densityRate="2" start="0.15" end="0.35"/><pyroImpulse shape="box" width="2" height="2" depth="2" time="0.5" density="1" temperature="3000"/></pyro><medium blackbody="true" emissionScale="0.1"/></object3D></composition></scene>\n'

PYRO_MESH = PYRO.replace("<composition>", '<assets><mesh id="source-mesh" src="../media/robot.glb"/></assets><composition>').replace('shape="sphere" radius="2"', 'shape="mesh" mesh="source-mesh"')

PYRO_COLLIDERS = PYRO.replace("<composition>", '<composition><object3D id="solid" primitive="box" width="2" height="2" depth="2"/>').replace("<pyro ", '<pyro colliders="solid" ')

SOLID_COLLIDERS = '<scene version="1.3"><project width="32" height="32" fps="10" duration="1"/><composition><object3D id="letters" primitive="text" text="O"/><object3D id="path" primitive="extrude" path="M0 0 L2 0 L2 2 Z"/><object3D id="clay" primitive="clay"><blob/></object3D><particles3D id="dust" colliders="letters path clay"/><object3D id="cloud" primitive="volume"><pyro width="4" height="4" depth="4" voxelSize="1" colliders="letters path clay"/></object3D></composition></scene>\n'

PARTICLES3D = '<scene version="1.3"><project width="64" height="64" fps="24" duration="2"/><composition><object3D id="floor" primitive="box" y="20" width="64" height="1" depth="64"/><particles3D id="dust" rate="0" speed="5" spread="120" gravityY="8" colliders="floor" lifetime="2"><burst time="0" count="12"/></particles3D></composition></scene>\n'

OCEAN = '<scene version="1.3"><project width="64" height="64" fps="24" duration="2"/><composition><ocean id="sea" width="8" depth="8" cellSize="0.5" bottomDepth="2"><waterImpulse time="0.3" radius="2" amplitude="0.1"/><wave wavelength="4" amplitude="0.1" phase="0"/></ocean></composition></scene>\n'

OCEAN_COUPLED = '<scene version="1.3"><project width="64" height="64" fps="24" duration="2"/><composition><object3D id="seabed" primitive="plane" width="40" height="40" segments="8" y="2" rotationX="-90"><crater radius="3" depth="1" rimHeight="0.2" rimWidth="1" start="0.5" end="1"/></object3D><object3D id="rock" primitive="sphere" radius="1" y="-3"/><ocean id="sea" width="8" depth="8" cellSize="0.5" bottomDepth="2" colliders="seabed rock"/></composition></scene>\n'

GLOBE = '<scene version="1.3"><project width="64" height="64" fps="24" duration="2"/><assets><tiles id="dem" src="../media/terrain.pmtiles"/><map id="m" width="64" height="32" background="#FFFFFF"/></assets><composition><object3D id="earth" primitive="globe" map="m" terrain="dem" terrainTileSize="2" terrainZoom="0" planetRadius="1000" radius="20" x="32" y="32" segments="32"/></composition></scene>\n'

TEXT3D = '<scene version="1.1"><project width="64" height="64" fps="24" duration="2"/><composition><object3D id="title" primitive="text" text="HI" height="20" tracking="120" x="32" y="32"/></composition></scene>\n'
MESH_SEQUENCE = '<scene version="1.3"><project width="64" height="64" fps="24" duration="2"/><assets><meshSequence id="frames" src="../media/mesh-frame-%d.obj" first="0" last="1" fps="1"/></assets><composition><object3D id="cache" primitive="mesh" mesh="frames" x="32" y="32"/></composition></scene>\n'

FRACTURE = '<scene version="1.3"><project width="64" height="64" fps="24" duration="3"/><materials><material id="interior" baseColor="#A06030"/></materials><composition><object3D id="rock" primitive="box" width="4" height="4" depth="4"><rigidBody mass="8"/><fracture at="1" pieces="8" seed="18446744073709551615" interiorMaterial="interior" radialImpulse="4"/></object3D></composition></scene>\n'

CRATER = '<scene version="1.3"><project width="64" height="64" fps="24" duration="3"/><composition><object3D id="ground" primitive="plane" width="20" height="20" segments="40"><crater radius="4" depth="3" rimWidth="1" rimHeight="0.5" start="1" end="2"/><rigidBody type="static"/></object3D></composition></scene>\n'

CRATER_IMPACT = '<scene version="1.3"><project width="64" height="64" fps="24" duration="3"/><composition><object3D id="rock" primitive="sphere" radius="1" y="-8"><rigidBody mass="5"/></object3D><object3D id="ground" primitive="plane" width="20" height="20" segments="40" y="2"><crater id="pit" source="rock" targetMaterial="softRock"/><rigidBody type="static"/></object3D></composition><physics pixelsPerMeter="1"/></scene>\n'

PYRO_CRATER = CRATER_IMPACT.replace('</composition>', '<object3D id="cloud" primitive="volume"><pyro width="8" height="8" depth="8" voxelSize="1" dt="0.1"><pyroSource crater="pit"/><pyroImpulse crater="pit" heatFraction="0.2"/></pyro></object3D></composition>')

OCEAN_BUOYANCY = '<scene version="1.3"><project width="64" height="64" fps="24" duration="2"/><composition><object3D id="float" primitive="sphere" radius="0.5" y="-1"><rigidBody shape="sphere" mass="200"/></object3D><ocean id="sea" width="8" depth="8" cellSize="0.5" bottomDepth="4" colliders="float" bodyCoupling="buoyancy" bodyDrag="1.5"/></composition><physics pixelsPerMeter="1"/></scene>\n'
EJECTA_CRATER = CRATER_IMPACT.replace('</composition>', '<particles3D id="debris" rate="0" gravityY="9.8" lifetime="3" maxParticles="500"><burst crater="pit" count="200"/></particles3D></composition>')

OCEAN_ENTRY = '<scene version="1.3"><project width="64" height="64" fps="24" duration="2"/><composition><object3D id="rock" primitive="sphere" radius="1" y="-4"><rigidBody mass="5"/></object3D><ocean id="sea" width="8" depth="8" cellSize="0.5" bottomDepth="2" colliders="rock"><waterImpulse source="rock"/></ocean></composition><physics pixelsPerMeter="1"/></scene>\n'

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
    ("solid-colliders-boil", ["P3D6", "PYRO8"], lambda _: SOLID_COLLIDERS.replace('primitive="clay"', 'primitive="clay" boil=" +12 " fingerprints=" +0.5 "')),
    ("solid-colliders-blob", ["P3D6", "PYRO8"], lambda _: SOLID_COLLIDERS.replace('<blob/>', '<blob><animate property="x"><key time="0" value="2"/></animate></blob>')),
    ("frx1", ["FRX1"], lambda _: FRACTURE.replace('version="1.3"', 'version="1.2"')),
    ("frx2", ["FRX2"], lambda _: FRACTURE.replace('<rigidBody mass="8"/>', '')),
    ("frx3", ["FRX3"], lambda _: FRACTURE.replace('interiorMaterial="interior"', 'interiorMaterial="rock"')),
    ("frx4", ["FRX4", "W01"], lambda _: FRACTURE.replace('radialImpulse="4"', 'radialImpulse="4" impulseX="NaN"')),
    ("crt1", ["CRT1"], lambda _: CRATER.replace('version="1.3"', 'version="1.2"')),
    ("crt2", ["CRT2"], lambda _: CRATER.replace('<crater ', '<crater/><crater ')),
    ("crt3", ["CRT3"], lambda _: CRATER.replace('end="2"', 'end="0"')),
    ("crt4", ["CRT4"], lambda _: CRATER.replace('rimWidth="1"', 'rimWidth="5"')),
    ("crt5", ["CRT5"], lambda _: CRATER.replace('type="static"', 'type="dynamic"')),
    ("crt6", ["CRT6"], lambda _: CRATER_IMPACT.replace('<crater ', '<crater depth="3" ')),
    ("crt7-material", ["CRT7"], lambda _: CRATER_IMPACT.replace(' targetMaterial="softRock"', '')),
    ("crt7-orphan", ["CRT7"], lambda _: CRATER.replace('<crater ', '<crater strength="1000" ')),
    ("crt9-capture-without-source", ["CRT9"], lambda _: CRATER.replace('<crater ', '<crater capture="true" ', 1)),
    ("crt8-static", ["CRT8"], lambda _: CRATER_IMPACT.replace('<rigidBody mass="5"/>', '<rigidBody type="static"/>')),
    ("s09-crater-id", ["S09"], lambda _: CRATER_IMPACT.replace('id="pit"', 'id="ground"')),
    ("pyc1-derived", ["PYC1"], lambda _: PYRO_CRATER.replace('<pyroSource crater="pit"/>', '<pyroSource crater="pit" start="1"/>')),
    ("pyc2-orphan", ["PYC2"], lambda _: PYRO_CRATER.replace('<pyroSource crater="pit"/>', '<pyroSource heatFraction="0.2"/>')),
    ("pyc3-time", ["PYC3"], lambda _: PYRO_CRATER.replace('<pyroImpulse crater="pit" heatFraction="0.2"/>', '<pyroImpulse density="1"/>')),
    ("pyc4-authored", ["PYC4"], lambda _: PYRO_CRATER.replace('<crater id="pit" source="rock" targetMaterial="softRock"/>', '<crater id="pit" radius="4" rimWidth="1"/>')),
    ("ocn8-coupling", ["OCN8"], lambda _: OCEAN.replace('<ocean ', '<ocean bodyCoupling="buoyancy" ')),
    ("ocn9-drag", ["OCN9"], lambda _: OCEAN.replace('<ocean ', '<ocean bodyDrag="1" ')),
    ("crt8-self", ["CRT8"], lambda _: CRATER_IMPACT.replace('source="rock"', 'source="ground"')),
    ("txt2", ["TXT2"], lambda _: TEXT3D.replace('primitive="text" text="HI"', 'primitive="box"')),
    ("solid-colliders-tracking", ["P3D6", "PYRO8"], lambda _: SOLID_COLLIDERS.replace('<object3D id="letters" primitive="text" text="O"/>', '<object3D id="letters" primitive="text" text="O"><animate property="tracking"><key time="0" value="10"/></animate></object3D>')),
    ("msq1", ["MSQ1"], lambda _: MESH_SEQUENCE.replace('version="1.3"', 'version="1.2"')),
    ("msq2", ["MSQ2", "A04"], lambda _: MESH_SEQUENCE.replace('last="1"', 'last="-1"')),
    ("msq3", ["MSQ3"], lambda _: MESH_SEQUENCE.replace('first="0"', 'sha256="' + '0' * 64 + '" first="0"')),
    ("msq4", ["MSQ4"], lambda _: MESH_SEQUENCE.replace('y="32"/>', 'y="32"><rigidBody type="static"/></object3D>')),
    ("geo1", ["GEO1"], lambda _: GLOBE.replace('version="1.3"', 'version="1.2"')),
    ("geo2", ["GEO2"], lambda _: GLOBE.replace('terrainTileSize="2"', 'terrainTileSize=" +260 "')),
    ("geo3", ["GEO3"], lambda _: GLOBE.replace('segments="32"/>', 'segments="32"><animate property="planetRadius"><key time="0" value="1000"/></animate></object3D>')),
    ("ocn1", ["OCN1"], lambda _: OCEAN.replace('version="1.3"', 'version="1.2"')),
    ("ocn2", ["OCN2"], lambda _: OCEAN.replace('bottomDepth="2"', 'bottomDepth="2" bathymetryEncoding="terrarium"')),
    ("ocn3", ["OCN3"], lambda _: OCEAN.replace('width="8"', 'width="8.1"')),
    ("ocn4", ["OCN4"], lambda _: OCEAN.replace('</ocean>', '<animate property="bottomDepth"><key time="0" value="2"/></animate></ocean>')),
    ("ocn5", ["OCN5"], lambda _: OCEAN.replace('</ocean>', '<whitewater start="1" end="0.5"/></ocean>')),
    ("pyro-advection", ["S06"], lambda _: VALID["pyro"].replace('<pyro ', '<pyro advection="rk4" ')),
    ("pyro-solver", ["S06"], lambda _: VALID["pyro"].replace('<pyro ', '<pyro solver="cg" ')),
    ("ocn6", ["OCN6"], lambda _: OCEAN_COUPLED.replace('colliders="seabed rock"', 'colliders="seabed rock seabed"')),
    ("ocn7", ["OCN7"], lambda _: OCEAN_COUPLED.replace('<object3D id="rock" primitive="sphere" radius="1" y="-3"/>', '<object3D id="rock" primitive="sphere" radius="1" y="-3"><animate property="radius"><key time="0" value="1"/><key time="1" value="2"/></animate></object3D>')),
    ("whitewater-checkpoint-range", ["S06"], lambda _: VALID["ocean"].replace('<whitewater ', '<whitewater checkpointMemoryMiB="4097" ')),
    ("ocean-order", ["S06"], lambda _: OCEAN.replace('bottomDepth="2"', 'bottomDepth="2" order="3"')),
    ("p3d1", ["P3D1"], lambda _: PARTICLES3D.replace('version="1.3"', 'version="1.2"')),
    ("p3d2", ["P3D2"], lambda _: PARTICLES3D.replace('rate="0"', 'rate="0" emitterShape="mesh"')),
    ("p3d3", ["P3D3"], lambda _: PARTICLES3D.replace('lifetime="2"', 'lifetime="2" lifetimeVariance="2"')),
    ("p3d7-derived", ["P3D7"], lambda _: EJECTA_CRATER.replace('<burst crater="pit" count="200"/>', '<burst crater="pit" time="1" count="200"/>')),
    ("p3d7-time", ["P3D3", "P3D7"], lambda _: EJECTA_CRATER.replace('<burst crater="pit" count="200"/>', '<burst count="200"/>')),
    ("p3d8-authored", ["P3D8"], lambda _: EJECTA_CRATER.replace('<crater id="pit" source="rock" targetMaterial="softRock"/>', '<crater id="pit" radius="4" rimWidth="1"/>')),
    ("p3d9-orphan", ["P3D9"], lambda _: EJECTA_CRATER.replace('<burst crater="pit" count="200"/>', '<burst time="0" count="200" angle="30"/>')),
    ("ocn13-splash", ["OCN13"], lambda _: EJECTA_CRATER.replace('</composition>', '<ocean id="sea" width="16" depth="16" cellSize="1" splash="debris pit"/></composition>')),
    ("p3d11-gas", ["P3D11"], lambda _: PARTICLES3D.replace('<particles3D id="dust"', '<particles3D id="dust" gas="floor"')),
    ("p3d10-angle", ["P3D10"], lambda _: EJECTA_CRATER.replace('count="200"/>', 'count="200" angle="80"/>')),
    ("bedresponse-value", ["S06"], lambda _: OCEAN_COUPLED.replace('<ocean ', '<ocean bedResponse="linear" ')),
    ("ocn10-derived", ["OCN10"], lambda _: OCEAN_ENTRY.replace('<waterImpulse source="rock"/>', '<waterImpulse source="rock" radius="2"/>')),
    ("ocn11-unlisted", ["OCN11"], lambda _: OCEAN_ENTRY.replace(' colliders="rock"', '')),
    ("ocn11-static", ["OCN11"], lambda _: OCEAN_ENTRY.replace('<rigidBody mass="5"/>', '<rigidBody type="static"/>')),
    ("ocn12-twice", ["OCN12"], lambda _: OCEAN_ENTRY.replace('<waterImpulse source="rock"/>', '<waterImpulse source="rock"/><waterImpulse source="rock"/>')),
    ("p3d4", ["P3D4"], lambda _: PARTICLES3D.replace('</particles3D>', '<animate property="lifetime"><key time="0" value="2"/></animate></particles3D>')),
    ("p3d5", ["P3D5"], lambda _: PARTICLES3D.replace('colliders="floor"', 'colliders="dust"')),
    ("p3d6", ["P3D6"], lambda _: PARTICLES3D.replace('depth="64"/>', 'depth="64"><animate property="width"><key time="0" value="64"/><key time="1" value="32"/></animate></object3D>')),
    ("pyro1", ["PYRO1"], lambda _: PYRO.replace('voxelSize="1"', 'voxelSize="3"')),
    ("pyro2", ["PYRO2"], lambda _: PYRO.replace('end="0.35"', 'end="0.1"')),
    ("pyro3", ["PYRO3"], lambda _: PYRO.replace('shape="sphere" radius="2"', 'shape="box"')),
    ("pyro4", ["PYRO4"], lambda _: PYRO.replace('radius="2"', 'radius="2" scaleX="0"')),
    ("pyro5", ["PYRO5"], lambda _: PYRO.replace('shape="sphere" radius="2"', 'shape="mesh"')),
    ("pyro6", ["PYRO6"], lambda _: PYRO_MESH.replace('end="0.35"/>', 'end="0.35"><animate property="mesh"><key time="0" value="source-mesh"/></animate></pyroSource>')),
    ("pyro7", ["PYRO7"], lambda _: PYRO_COLLIDERS.replace('colliders="solid"', 'colliders="cloud"')),
    ("pyro8", ["PYRO8"], lambda _: PYRO_COLLIDERS.replace('depth="2"/>', 'depth="2"><animate property="width"><key time="0" value="2"/><key time="1" value="4"/></animate></object3D>')),
    ("v8-volumes", ["V8"], lambda _: VOLUME.replace('version="1.3"', 'version="1.2"')),
    ("vol1", ["VOL1"], lambda _: VOLUME.replace(' volume="smoke"', '')),
    ("vol2", ["VOL2"], lambda _: VOLUME.replace('volume="smoke"', 'volume="cloud"')),
    ("vol3", ["VOL3"], lambda _: VOLUME.replace('primitive="volume"', 'primitive="box"')),
    ("vol4", ["VOL4"], lambda _: VOLUME.replace(' boundsMaxZ="2"', '')),
    ("vol5", ["VOL5"], lambda _: VOLUME.replace('<medium ', '<medium blackbody="true" ')),
    ("vol6", ["VOL6"], lambda _: VOLUME_SEQUENCE.replace(' last="0"', '')),
    ("vol9", ["VOL9"], lambda _: VOLUME_SEQUENCE.replace('interpolation="linear"', 'interpolation="advect" velocityGridX="velocity.x" velocityGridY="velocity.y"')),
    ("vol8", ["VOL8"], lambda _: VOLUME_BAKED.replace('format="srvseq"', 'format="srvseq" fps="1"')),
    ("vol10", ["VOL10"], lambda _: VOLUME.replace('<medium ', '<medium lightGridCell="2" ')),
    ("vol-lighting-enum", ["S06"], lambda _: VOLUME.replace('<medium ', '<medium lighting="fast" ')),
    ("vol-light-grid-cell", ["S06"], lambda _: VOLUME.replace('<medium ', '<medium lighting="grid" lightGridCell="0" ')),
    ("vol-light-grid-directions", ["S06"], lambda _: VOLUME.replace('<medium ', '<medium lighting="grid" lightGridDomeDirections="4" ')),
    ("vol-light-grid-memory", ["S06"], lambda _: VOLUME.replace('<medium ', '<medium lighting="grid" lightGridMemoryMiB="0" ')),
    ("vol7", ["VOL7"], lambda _: VOLUME_SEQUENCE.replace('id="smoke"', 'id="smoke" sha256="' + '0' * 64 + '"')),
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
    ("v5-elements", ["V5"], sub('<scene version="1.2">', '<scene version="1.1">')),
    ("v4-assets", ["V4"], lambda t: '<scene version="1.0"><project width="640" height="360" fps="25" duration="2"/><assets><formula id="f" tex="x" width="10" height="10"/></assets><composition/></scene>\n'),
    # ---- co-occurrence
    ("c1", ["C1"], sub('<vector id="icon" shape="path" path="M0 0 L10 0 L5 10 Z"', '<vector id="icon" shape="path"')),
    ("c2", ["C2"], sub('<vector id="icon" shape="path" path="M0 0 L10 0 L5 10 Z"', '<vector id="icon" shape="svg"')),
    ("c3", ["C3"], sub('<shape id="dot" shape="ellipse"', '<shape id="dot" shape="path"')),
    ("c65", ["C65"], sub('<shape id="dot" shape="ellipse"', '<shape id="dot" shape="ellipse" markerEnd="arrow"')),
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
    # ---- output segments
    ("c54", ["C54"], sub('<output id="short" start="0" path', '<output id="short" start="1" path')),
    ("c55", ["C55"], sub('<segment fromMarker="drop" to="8"', '<segment fromMarker="drop"')),
    ("c56", ["C56"], sub('<segment from="0" to="4"', '<segment from="4" to="2"')),
    ("c57", ["C57"], sub('<segment fromMarker="drop" to="8"', '<segment from="3" fromMarker="drop" to="8"')),
    ("c58", ["C58"], sub('<transition type="crossfade" duration="0.3" alignment="end"/>', '<transition type="crossfade" duration="0.3" alignment="end"/>\n      <transition type="cut"/>')),
    ("c59", ["C59"], sub('<transition type="crossfade" duration="0.3" alignment="end"/>', '<transition type="morph" duration="0.3"/>')),
    ("r38", ["R38"], sub('fromMarker="drop"', 'fromMarker="hero"')),
    ("r39", ["R39"], sub('audioTracks="music-track"', 'audioTracks="music-track fx"')),
    ("r39-bus", ["R39"], sub('audioBuses="fx"', 'audioBuses="fx music-track"')),
    ("c59-luma", ["C59"], sub('<transition type="crossfade" duration="0.3" alignment="end"/>', '<transition type="luma" matte="card" duration="0.3"/>')),
    ("r40", ["R40"], sub('overlay="card"', 'overlay="logo"')),
    ("r41", ["R41"], sub('transcribe="short-vo"', 'transcribe="vo-track"')),
    # ---- schema 1.1.3
    ("v6-elements", ["V6"], lambda t: v10('<particleEmitter id="p"><burst time="1" count="2"/></particleEmitter>')),
    ("v7-primitives", ["V7"], lambda t: v10('<object3D id="o" primitive="torus"/>')),
    ("r10-any-node", ["R10"], ins_comp('<flock id="fk" width="10" height="10" parent="fk"/>')),
    ("r30-outline", ["R30-outline"], sub('outline="#FFFFFF"', 'outline="url(#nope)"')),
    ("r31-baseColor", ["R31-baseColor"], sub('<material id="chrome" metallic="1"', '<material id="chrome" baseColor="var(--nope)" metallic="1"')),
    ("c50", ["C50"], sub('<particleEmitter id="sparks" color="#ffcc00">', '<particleEmitter id="sparks" color="#ffcc00" shape="sprite">')),
    ("r32", ["R32"], sub('<particleEmitter id="sparks" color="#ffcc00">', '<particleEmitter id="sparks" color="#ffcc00" sprite="music">')),
    ("r42", ["R42"], sub('<pattern id="checker" asset="logo"/>', '<pattern id="checker" asset="music"/>')),
    ("r43", ["R43"], sub('<particleEmitter id="sparks" color="#ffcc00">', '<particleEmitter id="sparks" color="#ffcc00" emitterAsset="music">')),
    ("r44", ["R44"], sub('<effect id="grade" type="lut" src="../media/grade.cube"/>', '<effect id="grade" type="lut" src="../media/grade.cube"/>\n    <effect id="dm" type="displacement-map" source="logo"/>')),
    ("r45", ["R45"], sub('<generator id="noise" kind="fractal-noise" width="512" height="512">', '<generator id="noise" kind="fractal-noise" width="512" height="512" lineWidth="2">')),
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
    ("c48", ["C48"], sub('<rigidBody type="static" shape="trimesh"/>', '<rigidBody shape="trimesh"/>')),
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
    ("w02-open-face", ["W02"], lambda _: PYRO.replace('width="8" height="8" depth="8" voxelSize="1"', 'width="64" height="64" depth="64" voxelSize="1" boundary="open"').replace('<pyroSource shape="sphere" radius="2"', '<pyroSource shape="sphere" y="-28" radius="2"')),
    ("w01-non-finite", ["W01"], sub('<marker id="drop" time="4.2"', '<marker id="drop" time="4.2" duration="1"/>\n    <marker id="late" time="INF"')),
]

VALID = {
    "solid-colliders": SOLID_COLLIDERS,
    "fracture": FRACTURE,
    "openvdb": VOLUME.replace('src="../media/uniform.srvol"', 'src="../media/impact-0.vdb" format="openvdb" temperatureGrid="temperature"').replace('<medium ', '<medium blackbody="true" '),
    "openvdb-sequence": VOLUME.replace('src="../media/uniform.srvol"', 'src="../media/impact-%d.vdb" format="openvdb" first="0" last="1" interpolation="linear"'),
    "crater": CRATER,
    "crater-impact": CRATER_IMPACT,
    "pyro-crater": PYRO_CRATER,
    "ejecta-crater": EJECTA_CRATER,
    "ocean-entry": OCEAN_ENTRY,
    "ocean-hydrostatic": OCEAN_COUPLED.replace('<ocean ', '<ocean bedResponse="hydrostatic" '),
    "ocean-depth-filtered-drag": OCEAN_COUPLED.replace('<ocean ', '<ocean bedResponse="depthFiltered" bodyDrag="2" '),
    "text3d-tracking": TEXT3D,
    "mesh-sequence": MESH_SEQUENCE,
    "globe-relief": GLOBE,
    "particles3d": PARTICLES3D,
    "ocean-colliders": OCEAN_COUPLED,
    "ocean-buoyancy": OCEAN_BUOYANCY,
    "physics-internal-edges": OCEAN_BUOYANCY.replace('<physics pixelsPerMeter="1"/>', '<physics pixelsPerMeter="1" fixInternalEdges="true"/>'),
    "crater-capture": CRATER_IMPACT.replace('targetMaterial="softRock"', 'targetMaterial="softRock" capture="true"'),
    "ocean-splash": EJECTA_CRATER.replace('</composition>', '<ocean id="sea" width="16" depth="16" cellSize="1" splash="debris"/></composition>'),
    "particles3d-gas": PARTICLES3D.replace('<composition>', '<composition><object3D id="smoke" primitive="volume"><pyro width="8" height="8" depth="8" voxelSize="1" boundary="open"/></object3D>').replace('<particles3D id="dust"', '<particles3D id="dust" drag="1" gas="smoke"'),
    "ocean-full": OCEAN_BUOYANCY.replace('bodyCoupling="buoyancy"', 'bodyCoupling="full"'),
    "ocean-order2": OCEAN.replace('bottomDepth="2"', 'bottomDepth="2" order="2"'),
    "ocean": OCEAN.replace('</ocean>', '<whitewater emissionRate="2" threshold="0.3"/></ocean>'),
    "whitewater-checkpoint": OCEAN.replace('</ocean>', '<whitewater emissionRate="2" threshold="0.3" checkpointMemoryMiB="0"/></ocean>'),
    "pyro": PYRO,
    "pyro-multigrid": PYRO.replace('<pyro ', '<pyro solver="multigrid" '),
    "pyro-maccormack": PYRO.replace('<pyro ', '<pyro solver="multigrid" advection="maccormack" '),
    "pyro-mesh": PYRO_MESH,
    "pyro-colliders": PYRO_COLLIDERS,
    "pyro-fields": PYRO.replace('<pyro ', '<pyro forceFields="wind" useForceFields="true" ').replace('</scene>', '<physics><forceField id="wind" type="wind" forceX="1" affects="particles" start="0.1" end="0.8"/></physics></scene>'),
    "baked-volume": VOLUME_BAKED,
    "volume": VOLUME,
    "volume-light-grid": VOLUME.replace('<medium ', '<medium lighting="grid" lightGridCell="2" lightGridDomeDirections="32" lightGridMemoryMiB="64" '),
    "advected-volume": VOLUME_SEQUENCE.replace('uniform-%02d', 'advected-%d').replace('last="0"', 'last="1"').replace('fps="24000/1001"', 'fps="1"').replace('interpolation="linear"', 'interpolation="advect" velocityGridX="velocity.x" velocityGridY="velocity.y" velocityGridZ="velocity.z"').replace(' boundsMinX="0" boundsMinY="0" boundsMinZ="0" boundsMaxX="32" boundsMaxY="32" boundsMaxZ="2"', '').replace('<composition>', '<composition><camera id="camera" x="0" y="0" z="-30" projection="orthographic"/>'),
    "volume-sequence": VOLUME_SEQUENCE.replace('first="0" last="0"', 'first=" +0 " last=" 0000 "').replace('uniform-%02d', 'uniform-0%d').replace('</assets>', '<volume id="hash" src="../media/uniform-##.srvol" first="0" last="0"/></assets>'),
    "thermal-volume": VOLUME.replace('id="smoke" src=', 'id="smoke" temperatureGrid="density" src=').replace('<medium ', '<medium blackbody="true" temperatureScale="5000" '),
    "kitchen-sink": base,
    "minimal": MINIMAL,
    "version-1.0": v10('<layer id="l" asset="img" x="10" y="10"/>').replace("<composition>", '<assets><image id="img" src="../media/logo.png" width="16" height="16"/></assets><composition>'),
}

# libxml2 (the oracle) skips two XSD 1.0 checks that sr-model enforces:
# IDREF resolution (Part 1, Validation Rule: Validation Root Valid (ID/IDREF
# Table)) and minLength 1 on whitespace-only IDREFS lists.
ORACLE_BLIND = {"s06-idrefs-empty", "s10-dangling-idref"}

CASES.append(("openvdb-format", ["S06"], lambda _: VALID["openvdb"].replace('format="openvdb"', 'format="guess"')))

def write(path, text, expect):
    header = f"<!-- expect: {' '.join(expect) if expect else 'valid'} -->\n"
    body = text.split("\n", 1)[1] if text.startswith("<?xml") else text
    decl = '<?xml version="1.0" encoding="UTF-8"?>\n'
    path.write_text(decl + header + body)
    return (decl + header + body).encode()

def oracle_codes(expected, blind=False):
    """Translate Rust diagnostics into the independent schema oracle's scope."""
    codes = {code for code in expected if not code.startswith(("S", "A", "W"))}
    if not blind and any(code.startswith("S") for code in expected):
        codes.add("XSD")
    return sorted(codes)


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
        # Asset diagnostics and renderer warnings are outside XSD/Schematron.
        want = oracle_codes(expect, blind=name in ORACLE_BLIND)
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
