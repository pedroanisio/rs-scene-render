import sys
sys.path.insert(0, sys.argv[1])
from oracle import verdict
HEAD = '<scene version="1.3"><project width="64" height="64" fps="24" duration="3"/><assets><voxelAsset id="model" src="../media/voxels.vox" maxCells="100000"/></assets><materials><material id="stone" baseColor="#808080"/></materials><composition><object3D id="ball" primitive="sphere" radius="1" y="-8"><rigidBody mass="1"/></object3D>'
TAIL = '</composition><physics pixelsPerMeter="1"/></scene>\n'
def obj(attrs, body, kids):
    return HEAD + f'<object3D id="block" primitive="voxels" voxels="model" cellSize="2" material="stone"{attrs}><rigidBody {body}/>{kids}</object3D>' + TAIL
FR = '<fracture source="ball" {}/>'
CR = '<crater id="pit" source="ball" targetMaterial="softRock" {}/>'
P = lambda n: ' '.join(['1 0 0 4']*n)
cases = {
 'shape=voxels explicit': obj('', 'shape="voxels" density="2400"', ''),
 'shape=box mass on voxels': obj('', 'shape="box" mass="3"', ''),
 'shape=box density': obj('', 'shape="box" density="3"', ''),
 'voronoi explicit': obj('', 'density="1"', FR.format('partition="voronoi" pieces="4" seed="2"')),
 'planes 63': obj('', 'density="1"', FR.format(f'partition="planes" planes="{P(63)}"')),
 'planes 64': obj('', 'density="1"', FR.format(f'partition="planes" planes="{P(64)}"')),
 'planes 1e3': obj('', 'density="1"', FR.format('partition="planes" planes="1e3 0 0 1"')),
 'planes Infinity': obj('', 'density="1"', FR.format('partition="planes" planes="Infinity 0 0 1"')),
 'planes +1': obj('', 'density="1"', FR.format('partition="planes" planes="+1 0 0 1"')),
 'planes 0 0 0 0': obj('', 'density="1"', FR.format('partition="planes" planes="0 0 0 0"')),
 'planes huge': obj('', 'density="1"', FR.format('partition="planes" planes="1'+'0'*400+' 0 0 1"')),
 'planes tab sep': obj('', 'density="1"', FR.format('partition="planes" planes="1&#9;0 0 1"')),
 'planes empty': obj('', 'density="1"', FR.format('partition="planes" planes=""')),
 'scale +2': obj(' scaleX="+2" scaleY="+2" scaleZ="+2"', 'type="static" density="1"', CR.format('')),
 'scale 2 2.0 2': obj(' scaleX="2" scaleY="2.0" scaleZ="2"', 'type="static" density="1"', CR.format('')),
 'crater start': obj('', 'type="static" density="1"', CR.format('start="0.1"')),
 'crater dynamic': obj('', 'density="1"', CR.format('')),
 'uvscale': obj('', 'density="1"', FR.format('interiorUvScale="2"')),
 'fracture scale nonuniform': obj(' scaleX="2"', 'density="1"', FR.format('')),
 'fracture shape=box nonuniform': obj(' scaleX="2"', 'shape="box" mass="1"', FR.format('')),
 'crater + slots on shape=box': obj('', 'type="static" shape="trimesh" maxFragments="3"', CR.format('')),
 'anchor fracture only': obj('', 'density="1" anchor="base"', FR.format('')),
 'density 0': obj('', 'density="0"', ''),
 'maxFragments 4097': obj('', 'density="1" maxFragments="4097"', FR.format('')),
 'fragmentMinCells 0': obj('', 'density="1" fragmentMinCells="0"', FR.format('')),
 'seed -1': obj('', 'density="1"', FR.format('seed="-1"')),
 'labels on voronoi': obj('', 'density="1"', FR.format('labels="material"')),
 'burst cell crater angle': obj('', 'type="static" density="1"', CR.format('')).replace(TAIL, '<particles3D id="d" rate="0" lifetime="3"><burst crater="pit" angle="30"/></particles3D>'+TAIL),
}
for k, d in cases.items():
    x, s = verdict(d.encode())
    print(f'{k:32} xsd={[m[:70] for _,m in x]} sch={sorted(set(i for i,_ in s))}')
