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

PYRO_FOLLOW = PYRO.replace('<pyro ', '<pyro boundary="open" follow="true" followMargin="2" followLoss="0.000001" ')
VOXELS = '<scene version="1.3"><project width="32" height="32" fps="30" duration="1"/><assets><voxelAsset id="model" src="../media/voxels.vox" maxCells="100000"/><mesh id="shape" src="../media/robot.glb"/></assets><materials><material id="stone" baseColor="#808080"/><material id="moss" baseColor="#406040"/></materials><composition><object3D id="build" primitive="voxels" voxels="model" cellSize="2" palette="stone moss" surface="blocks"/></composition></scene>\n'
VOXELS_FROM_MESH = VOXELS.replace('<voxelAsset id="model" src="../media/voxels.vox" maxCells="100000"/>', '<voxelAsset id="model" fromMesh="shape" cellSize="0.5"/>').replace(' palette="stone moss"', ' palette="file"')

PYRO_BLAST = PYRO.replace('<pyro ', '<pyro boundary="open" ').replace('</pyro>', '<pyroBlast time="0.3" energy="1000000" x="1" y="-1"/></pyro>')
# The block of cells is block.vox (16 x 4 x 16 cells of 0.25 m: 4 m by 1 m by 4 m, its origin at its minimum corner) and the ball falls at x = 2, z = 2: the middle of that block. The one
# document that scales the ground by 3 has a block of 12 m, and the ball is a third of the way across it, which is fine for what it checks (a scale written with a plus sign).
VOXEL_HEAD = '<scene version="1.3"><project width="64" height="64" fps="24" duration="3"/><assets><voxelAsset id="model" src="../media/block.vox" maxCells="100000"/></assets><materials><material id="stone" baseColor="#808080"/></materials><composition><object3D id="ball" primitive="sphere" radius="0.2" x="2" y="-8" z="2"><rigidBody mass="500" velocityY="120"/></object3D>'
VOXEL_TAIL = '</composition><physics pixelsPerMeter="1"/></scene>\n'
VOXEL_BODY = VOXEL_HEAD + '<object3D id="block" primitive="voxels" voxels="model" cellSize="0.25" material="stone"><rigidBody density="2400"/></object3D>' + VOXEL_TAIL
VOXEL_GROUND = VOXEL_HEAD + '<object3D id="ground" primitive="voxels" voxels="model" cellSize="0.25" material="stone" y="2"><rigidBody type="static" density="2400" maxFragments="128" fragmentMinCells="2" fragmentOverflow="dust" anchor="base"/><crater id="pit" source="ball" targetMaterial="softRock"/></object3D>' + VOXEL_TAIL
VOXEL_FRACTURE = VOXEL_HEAD + '<object3D id="block" primitive="voxels" voxels="model" cellSize="0.25" material="stone"><rigidBody density="2400" maxFragments="32" fragmentMinCells="1"/><fracture source="ball" pieces="8" seed="3"/></object3D>' + VOXEL_TAIL
VOXEL_STRESS = VOXEL_FRACTURE.replace('<fracture source="ball" pieces="8" seed="3"/>', '<fracture mode="stress" strength="2e6" pieces="8" seed="3"/>')
VOXEL_EJECTA = VOXEL_GROUND.replace(VOXEL_TAIL, '<particles3D id="debris" rate="0" gravityY="9.8" lifetime="3" maxParticles="500"><burst crater="pit"/></particles3D>' + VOXEL_TAIL)

PYRO_MESH = PYRO.replace("<composition>", '<assets><mesh id="source-mesh" src="../media/robot.glb"/></assets><composition>').replace('shape="sphere" radius="2"', 'shape="mesh" mesh="source-mesh"')

PYRO_COLLIDERS = PYRO.replace("<composition>", '<composition><object3D id="solid" primitive="box" width="2" height="2" depth="2"/>').replace("<pyro ", '<pyro colliders="solid" ')

SOLID_COLLIDERS = '<scene version="1.3"><project width="32" height="32" fps="10" duration="1"/><composition><object3D id="letters" primitive="text" text="O"/><object3D id="path" primitive="extrude" path="M0 0 L2 0 L2 2 Z"/><object3D id="clay" primitive="clay"><blob/></object3D><particles3D id="dust" colliders="letters path clay"/><object3D id="cloud" primitive="volume"><pyro width="4" height="4" depth="4" voxelSize="1" colliders="letters path clay"/></object3D></composition></scene>\n'

PARTICLES3D = '<scene version="1.3"><project width="64" height="64" fps="24" duration="2"/><composition><object3D id="floor" primitive="box" y="20" width="64" height="1" depth="64"/><particles3D id="dust" rate="0" speed="5" spread="120" gravityY="8" colliders="floor" lifetime="2"><burst time="0" count="12"/></particles3D></composition></scene>\n'

OCEAN = '<scene version="1.3"><project width="64" height="64" fps="24" duration="2"/><composition><ocean id="sea" width="8" depth="8" cellSize="0.5" bottomDepth="2"><waterImpulse time="0.3" radius="2" amplitude="0.1"/><wave wavelength="4" amplitude="0.1" phase="0"/></ocean></composition></scene>\n'

OCEAN_COUPLED = '<scene version="1.3"><project width="64" height="64" fps="24" duration="2"/><composition><object3D id="seabed" primitive="plane" width="40" height="40" segments="8" y="2" rotationX="-90"><crater radius="3" depth="1" rimHeight="0.2" rimWidth="1" start="0.5" end="1"/></object3D><object3D id="rock" primitive="sphere" radius="1" y="-3"/><ocean id="sea" width="8" depth="8" cellSize="0.5" bottomDepth="2" colliders="seabed rock"/></composition></scene>\n'

GLOBE = '<scene version="1.3"><project width="64" height="64" fps="24" duration="2"/><assets><tiles id="dem" src="../media/terrain.pmtiles"/><map id="m" width="64" height="32" background="#FFFFFF"/></assets><composition><object3D id="earth" primitive="globe" map="m" terrain="dem" terrainTileSize="2" terrainZoom="0" planetRadius="1000" radius="20" x="32" y="32" segments="32"/></composition></scene>\n'

MORPH = '<scene version="1.2"><project width="64" height="64" fps="24" duration="2"/><assets><mesh id="set" src="../media/robot.glb"/></assets><composition><object3D id="piece" primitive="mesh" mesh="set" x="32" y="32"><morph name="smile" weight="0.8"><animate property="weight"><key time="0" value="0"/><key time="1" value="1"/></animate></morph></object3D></composition></scene>\n'
JOINT = '<scene version="1.2"><project width="64" height="64" fps="24" duration="2"/><assets><mesh id="set" src="../media/robot.glb"/></assets><composition><shape id="t" shape="rect" x="10" y="10" width="4" height="4" fill="#FFFFFF"/><object3D id="piece" primitive="mesh" mesh="set" x="32" y="32"><joint name="head" rotation="12" lookAt="t" lookAxis="z" influence="0.8" maxAngle="35"><animate property="rotationX"><key time="0" value="0"/><key time="1" value="10"/></animate></joint></object3D></composition></scene>\n'
STROKE_TEXT = '<scene version="1.2"><project width="320" height="120" fps="24" duration="2"/><assets><strokeFont id="hand" src="../media/test.jhf"/></assets><composition><shape id="w" shape="stroke-text" text="HI AB" strokeFont="hand" fontSize="40" width="300" height="80" stroke="#FFFFFF" strokeWidth="3" strokeCap="round" trimMode="sequential"><animate property="trimEnd"><key time="0" value="0"/><key time="1" value="1"/></animate></shape></composition></scene>\n'
PINNED_FONTS = ('<scene version="1.2"><project width="64" height="64" fps="10" duration="1" fontPolicy="pinned"/>'
    '<assets><font id="inter" src="../media/inter.ttf" family="Inter" sha256="' + hashlib.sha256((MEDIA / "inter.ttf").read_bytes()).hexdigest() + '"/>'
    '<text id="t" text="Hi" width="60" height="30" size="20" font="Inter" fallback="Inter"/></assets>'
    '<composition><layer id="l" asset="t"/></composition></scene>\n')
MODEL_SELECT = '<scene version="1.2"><project width="64" height="64" fps="24" duration="2"/><assets><mesh id="set" src="../media/robot.glb"/></assets><materials><material id="clay" baseColor="#C48A5A"/></materials><composition><object3D id="piece" primitive="mesh" mesh="set" node="Knight" materialOverride="stone:clay old:clay" x="32" y="32"/></composition></scene>\n'
TEXT3D = '<scene version="1.1"><project width="64" height="64" fps="24" duration="2"/><composition><object3D id="title" primitive="text" text="HI" height="20" tracking="120" x="32" y="32"/></composition></scene>\n'
MESH_SEQUENCE = '<scene version="1.3"><project width="64" height="64" fps="24" duration="2"/><assets><meshSequence id="frames" src="../media/mesh-frame-%d.obj" first="0" last="1" fps="1"/></assets><composition><object3D id="cache" primitive="mesh" mesh="frames" x="32" y="32"/></composition></scene>\n'

FRACTURE = '<scene version="1.3"><project width="64" height="64" fps="24" duration="3"/><materials><material id="interior" baseColor="#A06030"/></materials><composition><object3D id="rock" primitive="box" width="4" height="4" depth="4"><rigidBody mass="8"/><fracture at="1" pieces="8" seed="18446744073709551615" interiorMaterial="interior" radialImpulse="4"/></object3D></composition></scene>\n'

POINTS = '<scene version="1.2"><project width="64" height="64" fps="24" duration="2"/><composition><repeat id="r" x="32" y="32"><points type="grid" columns="3" rows="2" spacingX="10" spacingY="10"><animate property="spacingX"><key time="0" value="5"/><key time="1" value="10"/></animate></points><shape id="s" shape="rect" width="4" height="4" fill="#FFFFFF"/></repeat></composition></scene>\n'

CONNECTOR = '<scene version="1.2"><project width="64" height="64" fps="24" duration="2"/><assets><text id="note" text="Hi" width="20" height="10" size="8"/></assets><composition><shape id="a" shape="rect" x="4" y="4" width="10" height="10" fill="#FF0000"/><shape id="b" shape="rect" x="44" y="40" width="10" height="10" fill="#0000FF"/><connector id="c" from="a" to="b" route="orthogonal" stroke="#00FF00" strokeWidth="2" markerEnd="arrow" label="note"><animate property="trimEnd"><key time="0" value="0"/><key time="1" value="1"/></animate></connector></composition></scene>\n'

LOGO_SHA = hashlib.sha256((MEDIA / "logo.png").read_bytes()).hexdigest()
PDF = ('<scene version="1.2"><project width="64" height="64" fps="24" duration="2"/><assets><pdf id="paper" src="../media/paper.pdf" '
       f'sha256="{"0" * 64}" dpi="72" cache="../media/logo.png" cacheSha256="{LOGO_SHA}" width="16" height="16">'
       '<region id="w" text="word" x="2" y="3" width="8" height="4"/></pdf></assets><composition><layer id="page" asset="paper" x="8" y="8"/>'
       '<shape id="mark" shape="ellipse" region="w" regionLayer="page" regionPadding="1" stroke="#FF0000" strokeWidth="1"/></composition></scene>\n')

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

BLACKHOLE = '<scene version="1.3"><project width="64" height="64" fps="24" duration="2"/><composition><shape id="sky" shape="rect" x="0" y="0" width="64" height="64" fill="#000000"/><camera id="eye" x="0" y="0" z="-60" geodesics="true"/><blackHole id="hole" mass="1" x="0" y="0" z="0"/><accretionDisk id="disk" blackHole="hole" outerRadius="20" temperatureScale="6000"/></composition></scene>\n'
FRACTURE_CONTACT = '<scene version="1.3"><project width="64" height="64" fps="24" duration="3"/><materials><material id="interior" baseColor="#A06030"/></materials><composition><object3D id="ball" primitive="sphere" radius="1" x="-8"><rigidBody mass="1"/></object3D><object3D id="rock" primitive="box" width="4" height="4" depth="4"><rigidBody mass="8"/><fracture source="ball" pieces="8" seed="3" interiorMaterial="interior"/></object3D></composition></scene>\n'

# (name, expected codes, transform-or-document, asset codes the oracle cannot see)
CASES = [
    ("solid-colliders-boil", ["P3D6", "PYRO8"], lambda _: SOLID_COLLIDERS.replace('primitive="clay"', 'primitive="clay" boil=" +12 " fingerprints=" +0.5 "')),
    ("solid-colliders-blob", ["P3D6", "PYRO8"], lambda _: SOLID_COLLIDERS.replace('<blob/>', '<blob><animate property="x"><key time="0" value="2"/></animate></blob>')),
    ("frx5-source-static", ["FRX5"], lambda _: FRACTURE_CONTACT.replace('<rigidBody mass="1"/>', '<rigidBody type="static"/>')),
    ("frx5-source-self", ["FRX5"], lambda _: FRACTURE_CONTACT.replace('source="ball"', 'source="rock"')),
    ("frx6-timed", ["FRX6"], lambda _: FRACTURE_CONTACT.replace('<fracture ', '<fracture at="1" ')),
    ("frx6-push", ["FRX6"], lambda _: FRACTURE_CONTACT.replace('<fracture ', '<fracture radialImpulse="4" ')),
    ("frx7-orphan", ["FRX7"], lambda _: FRACTURE.replace('<fracture ', '<fracture energyFraction="0.3" ')),
    ("vox1-version", ["VOX1"], lambda _: VOXELS.replace('version="1.3"', 'version="1.2"')),
    ("vox2-both-sources", ["VOX2"], lambda _: VOXELS.replace('src="../media/voxels.vox"', 'src="../media/voxels.vox" fromMesh="shape" cellSize="1"')),
    ("vox2-no-source", ["VOX2"], lambda _: VOXELS.replace(' src="../media/voxels.vox"', '')),
    ("vox2-cellsize-with-src", ["VOX2"], lambda _: VOXELS.replace('src="../media/voxels.vox"', 'src="../media/voxels.vox" cellSize="1"')),
    ("vox2-model-of-mesh", ["VOX2"], lambda _: VOXELS_FROM_MESH.replace('fromMesh="shape"', 'fromMesh="shape" model="0"')),
    ("vox2-grid-of-vox", ["VOX2"], lambda _: VOXELS.replace('src="../media/voxels.vox"', 'src="../media/voxels.vox" voxelGrid="voxels"')),
    ("vox2-model-of-srvol", ["VOX2"], lambda _: VOXELS.replace('src="../media/voxels.vox"', 'src="../media/uniform.srvol" model="0"')),
    ("vox3-frommesh", ["VOX3"], lambda _: VOXELS_FROM_MESH.replace('fromMesh="shape"', 'fromMesh="stone"')),
    ("vox4-asset", ["VOX4"], lambda _: VOXELS.replace('voxels="model"', 'voxels="shape"')),
    ("vox4-no-asset", ["VOX4"], lambda _: VOXELS.replace(' voxels="model"', '')),
    ("vox5-orphan", ["VOX5"], lambda _: VOXELS.replace('primitive="voxels"', 'primitive="box"')),
    ("vox5-surface-memory-orphan", ["VOX5"], lambda _: VOXELS.replace('primitive="voxels"', 'primitive="box" surfaceMemoryMiB="64"')),
    ("vox6-palette", ["VOX6"], lambda _: VOXELS.replace('palette="stone moss"', 'palette="stone nothing"')),
    ("vox8-shape", ["VOX8"], lambda _: CRATER_IMPACT.replace('<rigidBody type="static"/>', '<rigidBody type="static" shape="voxels"/>')),
    ("vox9-density", ["VOX9"], lambda _: FRACTURE.replace('<rigidBody mass="8"/>', '<rigidBody mass="8" density="2400"/>')),
    ("vox10-no-density", ["VOX10"], lambda _: VOXEL_BODY.replace(' density="2400"', '')),
    ("vox10-mass", ["VOX10"], lambda _: VOXEL_BODY.replace('density="2400"', 'density="2400" mass="5"')),
    ("vox11-slots", ["VOX11"], lambda _: VOXEL_BODY.replace('density="2400"', 'density="2400" maxFragments="8"')),
    ("vox12-anchor", ["VOX12"], lambda _: VOXEL_BODY.replace('density="2400"', 'density="2400" anchor="base"')),
    ("vox13-scale", ["VOX13"], lambda _: VOXEL_GROUND.replace('y="2">', 'y="2" scaleX="2">')),
    ("vox15-mesh-collider", ["VOX15"], lambda _: VOXEL_GROUND.replace('type="static" density="2400"', 'type="static" shape="trimesh"').replace(' maxFragments="128" fragmentMinCells="2" fragmentOverflow="dust" anchor="base"', '').replace('y="2">', 'y="2" scaleX="2">')),
    ("vox15-fracture-box", ["VOX15"], lambda _: VOXEL_FRACTURE.replace('<rigidBody density="2400" maxFragments="32" fragmentMinCells="1"/>', '<rigidBody shape="box"/>')),
    ("vox14-both", ["VOX14"], lambda _: VOXEL_GROUND.replace('</object3D>' + VOXEL_TAIL, '<fracture source="ball" pieces="4"/></object3D>' + VOXEL_TAIL)),
    ("crt13-mantle", ["CRT13"], lambda _: VOXEL_GROUND.replace('targetMaterial="softRock"', 'targetMaterial="softRock" mantle="true"')),
    ("crt14-curve", ["CRT14"], lambda _: VOXEL_GROUND.replace('targetMaterial="softRock"', 'targetMaterial="softRock" curve="linear"')),
    ("crt15-no-source", ["CRT15"], lambda _: VOXEL_GROUND.replace('<crater id="pit" source="ball" targetMaterial="softRock"/>', '<crater id="pit" radius="4" rimWidth="1"/>')),
    ("crt17-angle", ["CRT17"], lambda _: VOXEL_EJECTA.replace('<burst crater="pit"/>', '<burst crater="pit" angle="30" angleSpread="5"/>')),
    ("crt16-count", ["CRT16"], lambda _: VOXEL_EJECTA.replace('<burst crater="pit"/>', '<burst crater="pit" count="200"/>')),
    ("crt16-no-count", ["CRT16"], lambda _: EJECTA_CRATER.replace('<burst crater="pit" count="200"/>', '<burst crater="pit"/>')),
    ("frx8-interior", ["FRX8"], lambda _: VOXEL_FRACTURE.replace('<fracture ', '<fracture interiorMaterial="stone" ')),
    ("frx9-partition", ["FRX9"], lambda _: FRACTURE_CONTACT.replace('<fracture ', '<fracture partition="voronoi" ')),
    ("frx10-pieces", ["FRX10"], lambda _: VOXEL_FRACTURE.replace('pieces="8" seed="3"', 'partition="labels" labels="material" pieces="8"')),
    ("frx11-planes", ["FRX11"], lambda _: VOXEL_FRACTURE.replace('pieces="8" seed="3"', 'partition="planes" planes="1 0 0"')),
    ("frx11-no-partition", ["FRX11"], lambda _: VOXEL_FRACTURE.replace('pieces="8" seed="3"', 'planes="1 0 0 4"')),
    ("frx11-zero-normal", ["FRX11"], lambda _: VOXEL_FRACTURE.replace('pieces="8" seed="3"', 'partition="planes" planes="1 0 0 4 0 0 0 2"')),
    ("frx11-infinite", ["FRX11"], lambda _: VOXEL_FRACTURE.replace('pieces="8" seed="3"', 'partition="planes" planes="1 0 0 1' + '0' * 400 + '"')),
    ("frx13-no-strength", ["FRX13"], lambda _: VOXEL_STRESS.replace(' strength="2e6"', '')),
    ("frx13-strength-alone", ["FRX13"], lambda _: VOXEL_FRACTURE.replace('<fracture ', '<fracture strength="2e6" ')),
    ("frx14-static", ["FRX14"], lambda _: VOXEL_STRESS.replace('<rigidBody density="2400"', '<rigidBody type="static" density="2400"')),
    ("frx15-source", ["FRX15"], lambda _: VOXEL_STRESS.replace('<fracture ', '<fracture source="ball" ')),
    ("frx12-labels", ["FRX12"], lambda _: VOXEL_FRACTURE.replace('pieces="8" seed="3"', 'partition="labels"')),
    ("vox7-exclusion", ["VOX7"], lambda _: VOXELS.replace('surface="blocks"', 'surface="blocks" mesh="shape"')),
    ("frx1", ["FRX1"], lambda _: FRACTURE.replace('version="1.3"', 'version="1.2"')),
    ("frx2", ["FRX2"], lambda _: FRACTURE.replace('<rigidBody mass="8"/>', '')),
    ("frx3", ["FRX3"], lambda _: FRACTURE.replace('interiorMaterial="interior"', 'interiorMaterial="rock"')),
    ("frx4", ["FRX4", "W01"], lambda _: FRACTURE.replace('radialImpulse="4"', 'radialImpulse="4" impulseX="NaN"')),
    ("crt1", ["CRT1"], lambda _: CRATER.replace('version="1.3"', 'version="1.2"')),
    ("crt2", ["CRT2"], lambda _: CRATER.replace('<crater ', '<crater/><crater ')),
    ("crt3", ["CRT3"], lambda _: CRATER.replace('end="2"', 'end="0"')),
    ("crt4", ["CRT4"], lambda _: CRATER.replace('rimWidth="1"', 'rimWidth="5"')),
    ("bh1-version", ["BH1"], lambda _: BLACKHOLE.replace('version="1.3"', 'version="1.2"')),
    ("bh2-two-holes", ["BH2"], lambda _: BLACKHOLE.replace('<accretionDisk', '<blackHole id="second" mass="2"/><accretionDisk')),
    ("bh3-disk-target", ["BH3"], lambda _: BLACKHOLE.replace('blackHole="hole"', 'blackHole="eye"')),
    ("bh4-inside-isco", ["BH4"], lambda _: BLACKHOLE.replace('<accretionDisk id="disk"', '<accretionDisk id="disk" innerRadius="5"')),
    ("bh4-outer-not-beyond-inner", ["BH4"], lambda _: BLACKHOLE.replace('outerRadius="20"', 'outerRadius="6"')),
    ("bh5-geodesics-without-hole", ["BH5"], lambda _: BLACKHOLE.replace('<blackHole id="hole" mass="1" x="0" y="0" z="0"/>', '').replace('<accretionDisk id="disk" blackHole="hole" outerRadius="20" temperatureScale="6000"/>', '')),
    ("bh6-other-object", ["BH6"], lambda _: BLACKHOLE.replace('<blackHole', '<object3D id="ball" primitive="sphere" radius="1"/><blackHole')),
    ("bh7-inside-photon-sphere", ["BH7"], lambda _: BLACKHOLE.replace('z="-60" geodesics', 'z="-2.5" geodesics')),
    ("bh8-two-cameras", ["BH8"], lambda _: BLACKHOLE.replace('<blackHole', '<camera id="eye2" x="0" y="0" z="-80" geodesics="true"/><blackHole')),
    ("bh6-fluid", ["BH6"], lambda _: BLACKHOLE.replace('<blackHole', '<ocean id="sea"/><blackHole')),
    ("bh6-particles", ["BH6"], lambda _: BLACKHOLE.replace('<blackHole', '<particles3D id="dust" rate="1"/><blackHole')),
    ("crt4-envelope", ["CRT4"], lambda _: CRATER.replace('start="1"', 'influenceDepth="4" start="1"')),
    ("crt5", ["CRT5"], lambda _: CRATER.replace('type="static"', 'type="dynamic"')),
    ("crt6", ["CRT6"], lambda _: CRATER_IMPACT.replace('<crater ', '<crater depth="3" ')),
    ("crt7-material", ["CRT7"], lambda _: CRATER_IMPACT.replace(' targetMaterial="softRock"', '')),
    ("crt7-orphan", ["CRT7"], lambda _: CRATER.replace('<crater ', '<crater strength="1000" ')),
    ("crt9-capture-without-source", ["CRT9"], lambda _: CRATER.replace('<crater ', '<crater capture="true" ', 1)),
    ("crt8-static", ["CRT8"], lambda _: CRATER_IMPACT.replace('<rigidBody mass="5"/>', '<rigidBody type="static"/>')),
    ("s09-crater-id", ["S09"], lambda _: CRATER_IMPACT.replace('id="pit"', 'id="ground"')),
    ("pyro9-closed", ["PYRO9"], lambda _: PYRO_FOLLOW.replace('boundary="open" ', '')),
    ("pyro10-orphan", ["PYRO10"], lambda _: VALID["pyro"].replace('<pyro ', '<pyro followLoss="0.1" ')),
    ("pyro11-margin", ["PYRO11"], lambda _: PYRO_FOLLOW.replace('followMargin="2"', 'followMargin="4"')),
    ("pyc1-derived", ["PYC1"], lambda _: PYRO_CRATER.replace('<pyroSource crater="pit"/>', '<pyroSource crater="pit" start="1"/>')),
    ("pyc2-orphan", ["PYC2"], lambda _: PYRO_CRATER.replace('<pyroSource crater="pit"/>', '<pyroSource heatFraction="0.2"/>')),
    ("pyc3-time", ["PYC3"], lambda _: PYRO_CRATER.replace('<pyroImpulse crater="pit" heatFraction="0.2"/>', '<pyroImpulse density="1"/>')),
    ("pyc4-authored", ["PYC4"], lambda _: PYRO_CRATER.replace('<crater id="pit" source="rock" targetMaterial="softRock"/>', '<crater id="pit" radius="4" rimWidth="1"/>')),
    ("ocn8-coupling", ["OCN8"], lambda _: OCEAN.replace('<ocean ', '<ocean bodyCoupling="buoyancy" ')),
    ("ocn9-drag", ["OCN9"], lambda _: OCEAN.replace('<ocean ', '<ocean bodyDrag="1" ')),
    ("ocean-density-zero", ["S06"], lambda _: OCEAN.replace('<ocean ', '<ocean density="0" ')),
    ("crt10-mantle-without-source", ["CRT10"], lambda _: CRATER.replace('<crater ', '<crater mantle="true" ', 1)),
    ("crt10-bulking-without-source", ["CRT10", "CRT11"], lambda _: CRATER.replace('<crater ', '<crater bulking="1.1" ', 1)),
    ("crt11-bulking-without-mantle", ["CRT11"], lambda _: CRATER_IMPACT.replace('targetMaterial="softRock"', 'targetMaterial="softRock" bulking="1.1"')),
    ("crt10-repose-without-source", ["CRT10"], lambda _: CRATER.replace('<crater ', '<crater repose="35" ', 1)),
    ("crt12-repose-with-mantle", ["CRT12"], lambda _: CRATER_IMPACT.replace('targetMaterial="softRock"', 'targetMaterial="softRock" mantle="true" repose="35"')),
    ("crt12-repose-range", ["S06"], lambda _: CRATER_IMPACT.replace('targetMaterial="softRock"', 'targetMaterial="softRock" repose="75"')),
    ("crt11-bulking-range", ["S06"], lambda _: CRATER_IMPACT.replace('targetMaterial="softRock"', 'targetMaterial="softRock" mantle="true" bulking="1.5"')),
    ("crt8-self", ["CRT8"], lambda _: CRATER_IMPACT.replace('source="rock"', 'source="ground"')),
    ("v10", ["V10"], lambda _: POINTS.replace('version="1.2"', 'version="1.1"')),
    ("c17-points-count", ["C17"], lambda _: POINTS.replace('<repeat id="r"', '<repeat id="r" count="3"')),
    ("c74", ["C74"], lambda _: POINTS.replace('<shape ', '<points type="list" at="0,0"/><shape ')),
    ("c75", ["C75"], lambda _: POINTS.replace('<points type="grid" columns="3" rows="2"', '<points type="along-path" path="M0,0 L10,0"')),
    ("c76", ["C76"], lambda _: POINTS.replace('<points type="grid" columns="3" rows="2"', '<points type="scatter" count="3" width="10"')),
    ("c77", ["C77"], lambda _: POINTS.replace('<points type="grid" columns="3" rows="2"', '<points type="vertices"')),
    ("c78", ["C78"], lambda _: POINTS.replace('<points type="grid" columns="3" rows="2"', '<points type="list"')),
    ("c79", ["C79"], lambda _: POINTS.replace('<animate property="spacingX">', '<animate property="rows">')),
    ("c80", ["C80"], lambda _: POINTS.replace('<repeat id="r"', '<repeat id="r" from="1"')),
    ("v11", ["V11"], lambda _: CONNECTOR.replace('version="1.2"', 'version="1.1"')),
    ("c60", ["C60"], lambda _: CONNECTOR.replace(' to="b"', '')),
    ("c61", ["C61"], lambda _: CONNECTOR.replace(' to="b"', ' to="b" toAnchor="top" toX="1" toY="2"')),
    ("c62", ["C62"], lambda _: CONNECTOR.replace(' to="b"', ' to="b" toX="1"')),
    ("c63", ["C63"], lambda _: CONNECTOR.replace('route="orthogonal"', 'route="curved" points="1,1"')),
    ("c64", ["C64"], lambda _: CONNECTOR.replace('<animate property="trimEnd">', '<animate property="rotation">')),
    ("r48-from", ["R48-from"], lambda _: CONNECTOR.replace('<shape id="a" shape="rect"', '<shape id="a" threeD="true" shape="rect"')),
    ("r48-to", ["R48-to"], lambda _: CONNECTOR.replace('<shape id="b" shape="rect" x="44" y="40" width="10" height="10" fill="#0000FF"/>', '<repeat id="r" count="1"><shape id="b" shape="rect" x="44" y="40" width="10" height="10" fill="#0000FF"/></repeat>')),
    ("r49", ["R49"], lambda _: CONNECTOR.replace('</composition>', '<shape id="p" shape="rect" width="2" height="2" parent="c"/></composition>')),
    ("r50", ["R50"], lambda _: CONNECTOR.replace('label="note"', 'label="a"')),
    ("v9", ["V9"], lambda _: PDF.replace('version="1.2"', 'version="1.1"')),
    ("c66", ["C66"], lambda _: PDF.replace(f' sha256="{"0" * 64}"', '')),
    ("c67", ["C67", "R52"], lambda _: PDF.replace(' regionLayer="page"', '')),
    ("c68", ["C68"], lambda _: PDF.replace(' regionLayer="page"', ' regionLayer="page" parent="page"')),
    ("c69", ["C69"], lambda _: PDF.replace('</composition>', '<shape id="plain" shape="rect" width="4"/></composition>')),
    ("r51", ["R51", "R52"], lambda _: PDF.replace('region="w"', 'region="page"')),
    ("r52", ["R52"], lambda _: PDF.replace('<layer id="page" asset="paper" x="8" y="8"/>', '<layer id="page" asset="paper" x="8" y="8"/><layer id="other" asset="paper"/>').replace('regionLayer="page"', 'regionLayer="mark"')),
    ("v5-joint", ["V5"], lambda _: JOINT.replace('version="1.2"', 'version="1.1"')),
    ("v5-morph", ["V5"], lambda _: MORPH.replace('version="1.2"', 'version="1.1"')),
    ("pen1", ["PEN1"], lambda _: STROKE_TEXT.replace(' strokeFont="hand"', '')),
    # SREP 21: fontPolicy="pinned" (C70 to C73)
    ("c70-font-file", ["C70"], lambda _: PINNED_FONTS.replace(' fallback="Inter"', ' fallback="Inter" fontFile="../media/inter.ttf"')),
    ("c71-unpinned-asset", ["C71"], lambda _: re.sub(r' sha256="[0-9a-f]+"', '', PINNED_FONTS)),
    ("c72-host-family", ["C72"], lambda _: PINNED_FONTS.replace('font="Inter"', 'font="Helvetica"')),
    ("c73-host-fallback", ["C73"], lambda _: PINNED_FONTS.replace('fallback="Inter"', 'fallback="Inter, Arial"')),
    ("pen2", ["PEN2"], lambda _: STROKE_TEXT.replace('<strokeFont id="hand"', '<mesh id="hand"').replace('src="../media/test.jhf"', 'src="../media/robot.glb"')),
    ("v5-stroke-text", ["V5"], lambda _: STROKE_TEXT.replace('version="1.2"', 'version="1.1"')),
    ("mov1", ["MOV1"], lambda _: MODEL_SELECT.replace('materialOverride="stone:clay old:clay"', 'materialOverride="stone:clay :clay"')),
    ("mov2", ["MOV2"], lambda _: MODEL_SELECT.replace('materialOverride="stone:clay old:clay"', 'materialOverride="stone:clay old:nosuch"')),
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
    ("pyc5-closed", ["PYC5"], lambda _: PYRO_BLAST.replace('boundary="open" ', '')),
    ("pyc6-outside", ["PYC6"], lambda _: PYRO_BLAST.replace('x="1"', 'x="5"')),
    ("pyro-blast-gamma", ["S06"], lambda _: PYRO_BLAST.replace('<pyroBlast ', '<pyroBlast gamma="4" ')),
    ("pyro-advection", ["S06"], lambda _: VALID["pyro"].replace('<pyro ', '<pyro advection="rk4" ')),
    ("pyro-solver", ["S06"], lambda _: VALID["pyro"].replace('<pyro ', '<pyro solver="cg" ')),
    ("ocn6", ["OCN6"], lambda _: OCEAN_COUPLED.replace('colliders="seabed rock"', 'colliders="seabed rock seabed"')),
    ("ocn7", ["OCN7"], lambda _: OCEAN_COUPLED.replace('<object3D id="rock" primitive="sphere" radius="1" y="-3"/>', '<object3D id="rock" primitive="sphere" radius="1" y="-3"><animate property="radius"><key time="0" value="1"/><key time="1" value="2"/></animate></object3D>')),
    ("whitewater-checkpoint-range", ["S06"], lambda _: VALID["ocean"].replace('<whitewater ', '<whitewater checkpointMemoryMiB="4097" ')),
    ("whitewater-foam-mode", ["S06"], lambda _: VALID["ocean"].replace('<whitewater ', '<whitewater foamMode="mix" ')),
    ("ocn14-foam-radius", ["OCN14"], lambda _: VALID["ocean"].replace('</composition>', '<camera id="cam" renderer="pathtrace"/></composition>').replace('<whitewater ', '<whitewater foamMode="albedo" foamRadius="40" ')),
    ("whitewater-foam-albedo-range", ["S06"], lambda _: VALID["ocean"].replace('<whitewater ', '<whitewater foamAlbedo="1.5" ')),
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
    ("vol-scatter-bounces-range", ["S06"], lambda _: VOLUME.replace('<medium albedo="#000000" ', '<medium albedo="#808080" scatterBounces="33" ')),
    ("vol-scatter-bounces-fraction", ["S06"], lambda _: VOLUME.replace('<medium ', '<medium scatterBounces="1.5" ')),
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
    ("s06-idrefs-empty", ["S06", "R53"], sub('<adjustment id="grade-all" effects="grade"/>', '<adjustment id="grade-all" effects=" "/>')),
    ("s07-text", ["S07"], sub("<master/>", "<master>loud</master>")),
    ("s09-duplicate-id", ["S09"], sub('<bus id="fx"/>', '<bus id="fx"/>\n    <bus id="fx"/>')),
    ("s10-dangling-idref", ["S10", "R54"], sub('<audiogram id="wave" source="music-track"', '<audiogram id="wave" source="musik"')),
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
    ("r47", ["R47"], sub('<generator id="noise" kind="fractal-noise" width="512" height="512">', '<generator id="noise" kind="fractal-noise" width="512" height="512" lineWidth="2">')),
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
# SREPs 66 and 69: the version gate V12, PRG1 to PRG3 and STP1 on programs
CASES += [
    ("v12-program", ["V12"], lambda _: PROGRAMS.replace('version="1.6"', 'version="1.5"')),
    ("prg1-param-twice", ["PRG1"], lambda _: PROGRAMS.replace('<param name="side" value="1"/>', '<param name="side" value="1"/><param name="side" value="0"/>')),
    ("prg2-over-node-program", ["C18", "PRG2"], lambda _: PROGRAMS.replace('over="rows"', 'over="build"')),
    ("prg3-step-size", ["PRG3"], lambda _: PROGRAMS.replace(' width="8" height="8"', '')),
    ("stp1-build-program", ["STP1"], lambda _: PROGRAMS.replace(' seed="3"', ' seed="3" prewarm="2"')),
]
ASSET_CASES = [
    ("a01-missing-file", ["A01"], sub('src="../media/clip.mp4"', 'src="../media/clip-missing.mp4"')),
    ("a02-hash-mismatch", ["A02"], lambda t: re.sub(r'(src="\.\./media/music\.wav" sha256=")[0-9a-f]{4}', r'\g<1>0000', t)),
    ("a02-generated-cache", ["A02"], lambda t: re.sub(r'(cache="\.\./media/voice\.wav" cacheSha256=")[0-9a-f]{4}', r'\g<1>ffff', t)),
    ("a04-sequence-frames", ["A04"], sub('first="1" last="5"', 'first="1" last="9"')),
    ("prg10-program-hash", ["PRG10"], lambda _: PROGRAMS.replace(PROGRAM_SHA["rect"], "0" * 64)),
    ("prg10-program-missing", ["PRG10"], lambda _: PROGRAMS.replace("../media/rect.wasm", "../media/missing.wasm")),
]
WARN_CASES = [
    ("a03-remote", ["A03"], sub('src="../media/clip.mp4"', 'src="https://cdn.example.com/clip.mp4"')),
    ("a04-sequence-hold", ["A04"], sub('first="1" last="5"', 'first="1" last="9" missingFrame="hold"')),
    ("w02-open-face", ["W02"], lambda _: PYRO.replace('width="8" height="8" depth="8" voxelSize="1"', 'width="64" height="64" depth="64" voxelSize="1" boundary="open"').replace('<pyroSource shape="sphere" radius="2"', '<pyroSource shape="sphere" y="-28" radius="2"')),
    ("w04-denoise", ["W04"], lambda _: BLACKHOLE.replace('geodesics="true"', 'geodesics="true" denoise="true"')),
    ("w05-lights", ["W05"], lambda _: BLACKHOLE.replace('</composition>', '</composition><lights><light id="sun" type="directional"/></lights>')),
    ("w06-foam-material", ["W06"], lambda _: OCEAN.replace('</composition>', '<camera id="cam" renderer="pathtrace"/></composition>').replace('</ocean>', '<whitewater foamMode="albedo" foamMaterial="foam"/></ocean>').replace('<composition>', '<materials><material id="foam"/></materials><composition>')),
    ("w08-foam-unlit", ["W08"], lambda _: OCEAN.replace('</composition>', '<camera id="cam" renderer="pathtrace"/></composition>').replace('<ocean id="sea" ', '<ocean id="sea" material="glow" ').replace('</ocean>', '<whitewater foamMode="albedo"/></ocean>').replace('<composition>', '<materials><material id="glow" unlit="true"/></materials><composition>')),
    ("w10-scatter-black-albedo", ["W10"], lambda _: VOLUME.replace('<medium albedo="#000000" ', '<medium albedo="#000000" scatterBounces="4" ')),
    ("w07-foam-raster", ["W07"], lambda _: OCEAN.replace('</ocean>', '<whitewater foamMode="albedo"/></ocean>')),
    ("w09-voxel-cells-small", ["W09"], lambda _: VOXELS.replace('cellSize="2"', 'cellSize="0.25"', 1)),
    ("w03-no-lens", ["W03"], lambda _: BLACKHOLE.replace(' geodesics="true"', '')),
    ("w01-non-finite", ["W01"], sub('<marker id="drop" time="4.2"', '<marker id="drop" time="4.2" duration="1"/>\n    <marker id="late" time="INF"')),
]

# SREPs 66 and 69: a build program in the composition, a data program feeding a repeat and a stepping program
PROGRAM_SHA = {k: hashlib.sha256((MEDIA / f"{k}.wasm").read_bytes()).hexdigest() for k in ("rect", "rows", "counter")}
PROGRAMS = ('<scene version="1.6"><project width="64" height="64" fps="24" duration="1"/>'
    '<parameters><program id="rows" src="../media/rows.wasm" sha256="' + PROGRAM_SHA["rows"] + '"/></parameters><composition>'
    '<program id="build" src="../media/rect.wasm" sha256="' + PROGRAM_SHA["rect"] + '" seed="3" fuel="100000"><param name="side" value="1"/></program>'
    '<repeat id="rep" over="rows"><shape id="c" shape="rect" width="4" height="4" fill="#FF0000"/></repeat>'
    '<program id="steps" mode="step" width="8" height="8" src="../media/counter.wasm" sha256="' + PROGRAM_SHA["counter"] + '" stepsPerFrame="2" prewarm="3"/>'
    '</composition></scene>\n')

# SREP 70: a parametric curve, a parametric surface and a heightfield, in version 1.6
PARAMETRIC = ('<scene version="1.6"><project width="64" height="64" fps="24" duration="1"/>'
    '<materials><material id="m" baseColor="#FF0000" unlit="true"/></materials><composition>'
    '<shape id="curve" shape="parametric" stroke="#FFFFFF" strokeWidth="2"><parametricPath x="32 + 20 * Math.cos(t)" y="32 + 10 * Math.sin(t)" t0="0" t1="6.283185307179586" samples="64" closed="true"/></shape>'
    '<object3D id="surface" primitive="parametric" material="m" x="32" y="32"><parametricSurface x="u" y="v" z="0" u0="-8" u1="8" v0="-4" v1="4" uSamples="4" vSamples="4"/></object3D>'
    '<object3D id="ground" primitive="heightfield" material="m" x="32" y="48" rotationX="90"><heightfield height="2 * Math.sin(x / 4)" width="16" depth="8" xSamples="8" zSamples="4"/></object3D>'
    '</composition></scene>\n')
VALID = {
    "solid-colliders": SOLID_COLLIDERS,
    "fracture": FRACTURE,
    "openvdb": VOLUME.replace('src="../media/uniform.srvol"', 'src="../media/impact-0.vdb" format="openvdb" temperatureGrid="temperature"').replace('<medium ', '<medium blackbody="true" '),
    "openvdb-sequence": VOLUME.replace('src="../media/uniform.srvol"', 'src="../media/impact-%d.vdb" format="openvdb" first="0" last="1" interpolation="linear"'),
    "crater": CRATER,
    "crater-impact": CRATER_IMPACT,
    "pyro-crater": PYRO_CRATER,
    "ejecta-crater": EJECTA_CRATER,
    "pyro-crater-push": PYRO_CRATER.replace('<pyroSource crater="pit"/>', '<pyroSource crater="pit" velocityRateY="-2"/>').replace('<pyroImpulse crater="pit" heatFraction="0.2"/>', '<pyroImpulse crater="pit" heatFraction="0.2" velocityX="1"/>'),
    "ocean-entry": OCEAN_ENTRY,
    "ocean-hydrostatic": OCEAN_COUPLED.replace('<ocean ', '<ocean bedResponse="hydrostatic" '),
    "ocean-depth-filtered-drag": OCEAN_COUPLED.replace('<ocean ', '<ocean bedResponse="depthFiltered" bodyDrag="2" '),
    "model-select": MODEL_SELECT,
    "repeat-points": POINTS,
    "programs": PROGRAMS,
    "parametric": PARAMETRIC,
    "repeat-points-neutral-steps": POINTS.replace('<repeat id="r"', '<repeat id="r" from="0" step="1"'),
    "connector": CONNECTOR,
    "connector-points": CONNECTOR.replace(' to="b"', ' toX="50%" toY="100%"').replace('route="orthogonal"', 'route="curved" bend="-20"').replace(' label="note"', ''),
    "pdf-region": PDF,
    "stroke-text": STROKE_TEXT,
    "pinned-fonts": PINNED_FONTS,
    "morph": MORPH,
    "joint": JOINT,
    "text3d-tracking": TEXT3D,
    "mesh-sequence": MESH_SEQUENCE,
    "globe-relief": GLOBE,
    "particles3d": PARTICLES3D,
    "ocean-colliders": OCEAN_COUPLED,
    "ocean-density": OCEAN_BUOYANCY.replace('<ocean ', '<ocean density="1030" '),
    "ocean-buoyancy": OCEAN_BUOYANCY,
    "physics-internal-edges": OCEAN_BUOYANCY.replace('<physics pixelsPerMeter="1"/>', '<physics pixelsPerMeter="1" fixInternalEdges="true"/>'),
    "fracture-contact": FRACTURE_CONTACT,
    "fracture-contact-tuned": FRACTURE_CONTACT.replace('<fracture ', '<fracture minImpulse="50" energyFraction="0.5" '),
    "crater-impact-influence-small": CRATER_IMPACT.replace('targetMaterial="softRock"', 'targetMaterial="softRock" influenceDepth="5"'),
    "crater-impact-influence-large": CRATER_IMPACT.replace('targetMaterial="softRock"', 'targetMaterial="softRock" influenceDepth="30"'),
    "black-hole": BLACKHOLE,
    "black-hole-without-disk": BLACKHOLE.replace('<accretionDisk id="disk" blackHole="hole" outerRadius="20" temperatureScale="6000"/>', ''),
    "black-hole-disk-at-isco": BLACKHOLE.replace('<accretionDisk id="disk"', '<accretionDisk id="disk" innerRadius="6" seed="3" angularPattern="spiral" contrast="0.3" intensity="2" timeScale="0.5" rotationX="20" rotationY="5" rotation="-10"'),
    "crater-mantle": CRATER_IMPACT.replace('targetMaterial="softRock"', 'targetMaterial="softRock" mantle="true"'),
    "crater-repose": CRATER_IMPACT.replace('targetMaterial="softRock"', 'targetMaterial="softRock" repose="35"'),
    "crater-mantle-bulking": CRATER_IMPACT.replace('targetMaterial="softRock"', 'targetMaterial="softRock" mantle="true" bulking="1"'),
    "crater-capture": CRATER_IMPACT.replace('targetMaterial="softRock"', 'targetMaterial="softRock" capture="true"'),
    "ocean-splash": EJECTA_CRATER.replace('</composition>', '<ocean id="sea" width="16" depth="16" cellSize="1" splash="debris"/></composition>'),
    "particles3d-gas": PARTICLES3D.replace('<composition>', '<composition><object3D id="smoke" primitive="volume"><pyro width="8" height="8" depth="8" voxelSize="1" boundary="open"/></object3D>').replace('<particles3D id="dust"', '<particles3D id="dust" drag="1" gas="smoke"'),
    "ocean-full": OCEAN_BUOYANCY.replace('bodyCoupling="buoyancy"', 'bodyCoupling="full"'),
    "ocean-order2": OCEAN.replace('bottomDepth="2"', 'bottomDepth="2" order="2"'),
    "ocean": OCEAN.replace('</ocean>', '<whitewater emissionRate="2" threshold="0.3"/></ocean>'),
    "whitewater-checkpoint": OCEAN.replace('</ocean>', '<whitewater emissionRate="2" threshold="0.3" checkpointMemoryMiB="0"/></ocean>'),
    "whitewater-foam-albedo": OCEAN.replace('</composition>', '<camera id="cam" renderer="pathtrace"/></composition>').replace('</ocean>', '<whitewater emissionRate="2" threshold="0.3" foamMode="albedo" foamRadius="0.75" foamAlbedo="0.9" foamRoughness="0.8"/></ocean>'),
    "pyro": PYRO,
    "pyro-multigrid": PYRO.replace('<pyro ', '<pyro solver="multigrid" '),
    "pyro-maccormack": PYRO.replace('<pyro ', '<pyro solver="multigrid" advection="maccormack" '),
    "pyro-mesh": PYRO_MESH,
    "pyro-follow": PYRO_FOLLOW,
    "pyro-blast": PYRO_BLAST,
    "pyro-blast-gas": PYRO_BLAST.replace('<pyroBlast ', '<pyroBlast ambientDensity="0.9" ambientPressure="90000" gamma="1.67" '),
    "voxels": VOXELS,
    "voxels-from-mesh": VOXELS_FROM_MESH,
    # the extension is what says the format: a `.srvol` file with a grid, and one with the format said and no extension to say it
    "voxels-srvol": VOXELS.replace('src="../media/voxels.vox"', 'src="../media/uniform.srvol" voxelGrid="voxels"'),
    "voxels-surface-memory": VOXELS.replace('surface="blocks"', 'surface="blocks" surfaceMemoryMiB="64"'),
    "voxels-srvol-format": VOXELS.replace('src="../media/voxels.vox"', 'src="../media/uniform.srvol" format="srvol" voxelGrid="voxels"'),
    "voxel-body": VOXEL_BODY,
    "voxel-crater": VOXEL_GROUND,
    "voxel-ejecta": VOXEL_EJECTA,
    "voxel-crater-signed-scale": VOXEL_GROUND.replace('y="2">', 'y="2" scaleX="+3" scaleY="3" scaleZ=" 3.0 ">'),
    "voxel-fracture": VOXEL_FRACTURE,
    "voxel-fracture-stress": VOXEL_STRESS,
    "voxel-fracture-planes": VOXEL_FRACTURE.replace('pieces="8" seed="3"', 'partition="planes" planes="1 0 0 2 0 1 0 0.5"'),
    "voxel-fracture-labels": VOXEL_FRACTURE.replace('pieces="8" seed="3"', 'partition="labels" labels="material"'),
    "pyro-colliders": PYRO_COLLIDERS,
    "pyro-fields": PYRO.replace('<pyro ', '<pyro forceFields="wind" useForceFields="true" ').replace('</scene>', '<physics><forceField id="wind" type="wind" forceX="1" affects="particles" start="0.1" end="0.8"/></physics></scene>'),
    "baked-volume": VOLUME_BAKED,
    "volume": VOLUME,
    "volume-scatter-bounces": VOLUME.replace('<medium albedo="#000000" ', '<medium albedo="#808080" scatterBounces="4" '),
    "volume-scatter-bounces-zero": VOLUME.replace('<medium albedo="#000000" ', '<medium albedo="#808080" scatterBounces="0" '),
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
ORACLE_BLIND = {"s06-idrefs-empty", "s10-dangling-idref", "r53-empty-list"}

# ---- generated marker ids (beat.N and bar.N of a beatGrid), SREP 57
def marker_doc(markers='<beatGrid bpm="120" offset="0.25"/>', shape='', anim='', outputs=''):
    return f"""<scene version="1.1">
  <project width="480" height="270" fps="24" duration="4" background="#000000" motionBlur="false" seed="1"/>
{outputs}
  <markers>{markers}</markers>
  <composition>
    <shape id="n" shape="rect" x="100" y="100" width="60" height="60" fill="#FF0000" {shape}>{anim}</shape>
  </composition>
</scene>
"""

KEY_ANIM = '<animate property="x"><key time="0" value="0"/><key time="1.9" value="400" marker="{}"/></animate>'
POSTER = '\n  <output id="o" path="out/o.mp4" codec="h264"><poster path="out/p.png" marker="{}"/></output>'
VALID.update({
    "gm-start": marker_doc(shape='startMarker="beat.3"'),
    "gm-bar": marker_doc(shape='startMarker="bar.2"'),
    "gm-end": marker_doc(shape='startMarker="beat.0" endMarker="bar.1"'),
    "gm-key": marker_doc(anim=KEY_ANIM.format("beat.4")),
    "gm-poster": marker_doc(outputs=POSTER.format("bar.1")),
    "gm-leading-zeros": marker_doc(shape='startMarker="beat.007"'),
    "gm-explicit": marker_doc('<beatGrid bpm="120" offset="0.25"/><marker id="m3" time="1.25"/>', shape='startMarker="m3"'),
    "gm-name-without-grid": marker_doc('<marker id="beat.3" time="1.5"/>', shape='startMarker="beat.3"'),
})
CASES += [
    ("gm-no-grid", ["R21"], lambda _: marker_doc('<marker id="m3" time="1.25"/>', shape='startMarker="beat.3"')),
    ("gm-malformed-empty", ["R21"], lambda _: marker_doc(shape='startMarker="beat."')),
    ("gm-malformed-letters", ["R21"], lambda _: marker_doc(shape='startMarker="beat.x"')),
    ("gm-malformed-dots", ["R21"], lambda _: marker_doc(shape='startMarker="beat.3.1"')),
    ("gm-malformed-kind", ["R21"], lambda _: marker_doc(shape='startMarker="beats.3"')),
    ("gm-too-many-digits", ["R21"], lambda _: marker_doc(shape='startMarker="beat.12345678901234567890"')),
    ("gm-element-id", ["R21"], lambda _: marker_doc(shape='startMarker="n"')),
]
WARN_CASES += [
    ("w04-poster-no-grid", ["W04"], lambda _: marker_doc('<marker id="m3" time="1.25"/>', outputs=POSTER.format("bar.1"))),
    ("w04-poster-element-id", ["W04"], lambda _: marker_doc(outputs=POSTER.format("n"))),
    ("w04-poster-dangling", ["W04"], lambda _: marker_doc(outputs=POSTER.format("nowhere"))),
]
WARN_CASES.append(
    ("w03-marker-named-like-generated", ["W03"],
     lambda _: marker_doc('<beatGrid bpm="120" offset="0.25"/><marker id="beat.3" time="1.25"/>', shape='startMarker="beat.3"'))
)

CASES.append(("openvdb-format", ["S06"], lambda _: VALID["openvdb"].replace('format="openvdb"', 'format="guess"')))

CASES += [
    ("r53-empty-list", ["S06", "R53"], lambda _: MINIMAL.replace('<composition/>', '<composition><group id="g" effects=""/></composition>')),
    ("r54-audiogram-source", ["R54"], lambda _: MINIMAL.replace('<composition/>', '<assets><audiogram id="a" source="wrong" width="32" height="32"/></assets><composition><group id="wrong"/></composition>')),
]

# SREP 67 (document version 1.6): compute, iterate and the serial effect types
SREP67 = '<scene version="1.6"><project width="64" height="64" fps="10" duration="1"/><composition>{body}</composition>{post}</scene>\n'
SREP67_SHAPE = '<shape id="s" shape="rect" width="10" height="10" fill="#FFFFFF" effects="fx"/>'
CASES += [
    ("v13-iterate", ["V13"], lambda _: SREP67.format(body=f'<iterate id="it" steps="10">{SREP67_SHAPE}</iterate>', post='<effects><effect id="fx" type="blur"/></effects>').replace('version="1.6"', 'version="1.5"')),
    ("itr1-check-every", ["ITR1"], lambda _: SREP67.format(body=f'<iterate id="it" steps="10" checkEvery="20">{SREP67_SHAPE}</iterate>', post='<effects><effect id="fx" type="blur"/></effects>')),
    ("itr2-nested", ["ITR2"], lambda _: SREP67.format(body=f'<iterate id="a" steps="10"><iterate id="b" steps="5">{SREP67_SHAPE}</iterate></iterate>', post='<effects><effect id="fx" type="blur"/></effects>')),
    ("srt1-low-high", ["SRT1"], lambda _: SREP67.format(body=SREP67_SHAPE, post='<effects><effect id="fx" type="segmented-sort" low="0.8" high="0.2"/></effects>')),
    ("cmp13-two-tonemaps", ["CMP13"], lambda _: SREP67.format(body='<compute id="c" src="../media/srep67-three.wgsl" width="4" height="4" invocations="1"><tonemap/><tonemap/></compute>', post='')),
]

# SREP 68: stepsPerFrame and prewarm apply to shader effects only
CASES += [
    ("stp1-not-a-shader", ["STP1"], lambda _: '<scene version="1.6"><project width="64" height="64" fps="10" duration="1"/><composition><shape id="s" shape="rect" width="10" height="10" fill="#FFFFFF" effects="fx"/></composition><effects><effect id="fx" type="blur" stepsPerFrame="2"/></effects></scene>\n'),
]

# SREP 70: the version gate V14, PAR1 and PAR2, and C69's exemption of parametric shapes
CASES += [
    ("v14-path", ["V14"], lambda _: PARAMETRIC.replace('version="1.6"', 'version="1.5"')),
    ("v14-heightfield", ["V14"], lambda _: re.sub(r'<shape id="curve".*?</shape>|<object3D id="surface".*?</object3D>', '', PARAMETRIC).replace('version="1.6"', 'version="1.2"')),
    ("par1-missing", ["PAR1"], lambda _: re.sub(r'<parametricPath [^>]*/>', '', PARAMETRIC)),
    ("par1-other-shape", ["PAR1"], lambda _: PARAMETRIC.replace('shape="parametric"', 'shape="rect" width="4" height="4"')),
    ("par1-two", ["PAR1"], lambda _: re.sub(r'(<parametricPath [^>]*/>)', r'\1\1', PARAMETRIC)),
    ("par2-missing", ["PAR2"], lambda _: re.sub(r'<parametricSurface [^>]*/>', '', PARAMETRIC)),
    ("par2-other-primitive", ["PAR2"], lambda _: PARAMETRIC.replace('primitive="heightfield"', 'primitive="plane"')),
    ("par2-crossed", ["PAR2"], lambda _: PARAMETRIC.replace('primitive="parametric"', 'primitive="heightfield"')),
]

# SREP 73: flock/@orientToVelocity; false on a streak flock (the default shape) is inert (INERT-I16, information)
FLOCK_UPRIGHT = ('<scene version="1.6"><project width="64" height="64" fps="24" duration="1"/>'
    '<composition><flock id="f" width="32" height="32" count="4" seed="2" shape="disc" orientToVelocity="false"/></composition></scene>\n')
VALID["flock-upright"] = FLOCK_UPRIGHT
WARN_CASES += [
    ("inert-i16-streak-flock", ["INERT-I16"], lambda _: FLOCK_UPRIGHT.replace(' shape="disc"', '')),
]

# SREP 75: fractal assets (document version 1.6), V16 and FRC1. The centre is an xs:decimal, which sr-model reads at
# any length; libxml2 (the oracle) refuses one of more than 24 digits, a limit XSD 1.0 allows (Part 2, 3.2.3: at least
# 18), so these documents keep to 24 and the SREP's 33-digit centres are tested by sr-eval and sr-gpu only.
FRACTAL = ('<scene version="1.6"><project width="64" height="64" fps="24" duration="1"/>'
    '<assets><fractal id="f" kind="mandelbrot" width="16" height="16" centerX="-0.743643887037158704752191" '
    'centerY="+.131825904205311970493132" zoom="6" maxIterations="100" colorMode="bands" palette="#FF0000FF #0000FFFF">'
    '<animate property="zoom"><key time="0" value="6"/><key time="1" value="7"/></animate></fractal>'
    '<fractal id="j" kind="julia" width="16" height="16" juliaX="-0.8" juliaY="0.156"/></assets>'
    '<composition><layer id="l" asset="f"/><layer id="m" asset="j" x="20"/></composition></scene>\n')
VALID["fractal"] = FRACTAL
CASES += [
    ("v16-fractal", ["V16"], lambda _: FRACTAL.replace('version="1.6"', 'version="1.5"')),
    ("frc1-julia-without-c", ["FRC1"], lambda _: FRACTAL.replace(' juliaX="-0.8" juliaY="0.156"', ' juliaX="-0.8"')),
    ("frc1-mandelbrot-with-c", ["FRC1"], lambda _: FRACTAL.replace('kind="mandelbrot"', 'kind="mandelbrot" juliaX="0" juliaY="0"')),
    ("s06-decimal-exponent", ["S06"], lambda _: FRACTAL.replace('centerY="+.131825904205311970493132"', 'centerY="1.3e-1"')),
]

def write(path, text, expect):
    header = f"<!-- expect: {' '.join(expect) if expect else 'valid'} -->\n"
    body = text.split("\n", 1)[1] if text.startswith("<?xml") else text
    decl = '<?xml version="1.0" encoding="UTF-8"?>\n'
    path.write_text(decl + header + body)
    return (decl + header + body).encode()

def small_cells(text):
    """Whether a voxels object of the document has cells smaller than 0.5 in the scene (rules.rs W09): its cellSize, else its asset's, times the smallest scale."""
    import xml.etree.ElementTree as ET
    try:
        root = ET.fromstring(re.sub(r"<!--.*?-->", "", text, flags=re.S))
    except ET.ParseError:
        return False
    assets = {a.get("id"): a for a in root.iter() if a.tag.endswith("voxelAsset")}
    for o in root.iter():
        if not o.tag.endswith("object3D") or o.get("primitive") != "voxels":
            continue
        size = o.get("cellSize") or (assets[o.get("voxels")].get("cellSize") if o.get("voxels") in assets else None)
        try:
            cell = float(size.strip())
            scale = min(abs(float(o.get(k, "1").strip())) for k in ("scaleX", "scaleY", "scaleZ"))
        except (AttributeError, ValueError):
            continue
        if cell * scale < 0.5:
            return True
    return False


def with_small_cells(text, expect):
    """The expected codes of a document, with W09 when it has cells smaller than 0.5 in the scene."""
    return list(expect) + ["W09"] if small_cells(text) and "W09" not in expect else list(expect)


def oracle_codes(expected, blind=False):
    """Translate Rust diagnostics into the independent schema oracle's scope."""
    # structural (S01...), asset (A01...) and warning (W01...) codes are a letter and digits; a Schematron id may start
    # with one of those letters too (SRT1 and STP1 of SREPs 67 and 68)
    outside = re.compile(r"[SAW]\d")
    codes = {code for code in expected if not outside.match(code)}
    if not blind and any(re.match(r"S\d", code) for code in expected):
        codes.add("XSD")
    return sorted(codes)


def main():
    manifest = {"valid": {}, "invalid": {}, "warnings": {}}
    for d in ("valid", "invalid"):
        for f in (CORPUS / d).glob("*.xml"):
            f.unlink()
    ok = True
    for name, text in VALID.items():
        expect = with_small_cells(text, [])
        data = write(CORPUS / "valid" / f"{name}.scene.xml", text, expect)
        xsd, sch = verdict(data)
        if xsd or sch:
            ok = False
            print(f"valid/{name}: oracle disagrees: {xsd} {sch}")
        manifest["warnings" if expect else "valid"][f"{name}.scene.xml"] = expect
    for name, expect, fn in CASES:
        text = fn(base)
        expect = with_small_cells(text, expect)
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
        expect = with_small_cells(text, expect)
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
