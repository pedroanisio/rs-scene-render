p='schema/scene-render-1.1.xsd'
s=open(p).read()
a='''        <xs:enumeration value="convex-hull"/><xs:enumeration value="trimesh"/>
        <xs:enumeration value="decomposition"/>
      </xs:restriction></xs:simpleType>
    </xs:attribute>
    <xs:attribute name="mass" type="positiveDecimal" default="1"/>'''
assert s.count(a)==1
s=s.replace(a,'''        <xs:enumeration value="convex-hull"/><xs:enumeration value="trimesh"/>
        <xs:enumeration value="decomposition"/><xs:enumeration value="voxels"/>
      </xs:restriction></xs:simpleType>
    </xs:attribute>
    <xs:attribute name="mass" type="positiveDecimal" default="1"/>
    <xs:attribute name="density" type="positiveDecimal">
      <xs:annotation><xs:documentation>
        Kilograms per cubic metre, for a body of cells (VOX9, VOX10): its mass is the number of cells times the volume of one (cellSize times the object's
        scale, over the scene's pixelsPerMeter, cubed) times this. Required with cells as the collider, and `mass` is then not given.
      </xs:documentation></xs:annotation>
    </xs:attribute>
    <xs:attribute name="maxFragments">
      <xs:annotation><xs:documentation>
        A body of cells that a crater or a fracture can break gets this many slots (bodies that wait, one cell each, until a cut frees a part of it): 1 to 4096; the
        engine's value is 64, and it is not an XSD default so that it can be refused where there is nothing to cut (VOX11). The world refuses more fragments
        than slots (see fragmentOverflow).
      </xs:documentation></xs:annotation>
      <xs:simpleType><xs:restriction base="xs:positiveInteger"><xs:maxInclusive value="4096"/></xs:restriction></xs:simpleType>
    </xs:attribute>
    <xs:attribute name="fragmentMinCells" type="xs:positiveInteger">
      <xs:annotation><xs:documentation>
        Loose parts of a cut with fewer cells than this are dust: they leave the body and become particles (VOX11). The engine's value is 1.
      </xs:documentation></xs:annotation>
    </xs:attribute>
    <xs:attribute name="fragmentOverflow">
      <xs:annotation><xs:documentation>
        What a cut does when it frees more parts than there are slots (VOX11): `error` names both numbers, `dust` makes the smallest parts dust. The engine's value is error.
      </xs:documentation></xs:annotation>
      <xs:simpleType><xs:restriction base="xs:string"><xs:enumeration value="error"/><xs:enumeration value="dust"/></xs:restriction></xs:simpleType>
    </xs:attribute>
    <xs:attribute name="anchor">
      <xs:annotation><xs:documentation>
        Which part of what a crater's cut leaves stays the body (VOX12): `largest`, the largest connected part (the engine's value for a dynamic body), or `base`, every part
        that touches the base layer (the cells of the greatest y key, the lowest layer in the scene's axes, y down), the way ground is held by what is under it (the
        engine's value for a static or kinematic body).
      </xs:documentation></xs:annotation>
      <xs:simpleType><xs:restriction base="xs:string"><xs:enumeration value="largest"/><xs:enumeration value="base"/></xs:restriction></xs:simpleType>
    </xs:attribute>''')
a='    <xs:attribute name="interiorMaterial" type="xs:IDREF" use="required"/>\n    <xs:attribute name="interiorUvScale" type="positiveDecimal" default="1"/>'
assert s.count(a)==1
s=s.replace(a,'''    <xs:attribute name="interiorMaterial" type="xs:IDREF"/>
    <xs:attribute name="interiorUvScale" type="positiveDecimal" default="1"/>
    <xs:attribute name="partition">
      <xs:annotation><xs:documentation>
        On an object of cells (FRX9): how the body is cut into pieces, `voronoi` (the engine's value: `pieces` seeds drawn from `seed` in the box of the cells, each cell
        to the nearest), `planes` (the cells are the pieces' sides of the planes in `planes`) or `labels` (the cells that have one palette index are one part). A part that is not
        connected by faces is split into its components, so every piece is connected. `pieces` and `seed` belong to voronoi (FRX10). The pieces have the material of
        their cells, and a face that a cut exposes is drawn with the palette like any other: there is no interior material (FRX8).
      </xs:documentation></xs:annotation>
      <xs:simpleType><xs:restriction base="xs:string"><xs:enumeration value="voronoi"/><xs:enumeration value="planes"/><xs:enumeration value="labels"/></xs:restriction></xs:simpleType>
    </xs:attribute>
    <xs:attribute name="planes" type="xs:string">
      <xs:annotation><xs:documentation>
        With partition="planes" (FRX11): up to 63 planes, each four numbers `nx ny nz offset`, in object units (the object's own axes, before its scale, the
        origin at the corner of the cells): a cell is on the positive side of a plane if the dot product of the normal with the centre of the cell, `(key + 1/2) * cellSize`,
        is at least the offset. The normal is not unit and need not be; it is taken to 2^-32 of its largest component.
      </xs:documentation></xs:annotation>
    </xs:attribute>
    <xs:attribute name="labels">
      <xs:annotation><xs:documentation>
        With partition="labels" (FRX12): `material`, the palette index of a cell is its label, so that a model breaks along its materials.
      </xs:documentation></xs:annotation>
      <xs:simpleType><xs:restriction base="xs:string"><xs:enumeration value="material"/></xs:restriction></xs:simpleType>
    </xs:attribute>''')
a='    <xs:attribute name="time" type="xs:double"/>\n    <xs:attribute name="count" type="xs:positiveInteger" use="required"/>'
assert s.count(a)==1
s=s.replace(a,'    <xs:attribute name="time" type="xs:double"/>\n    <xs:attribute name="count" type="xs:positiveInteger"/>')
open(p,'w').write(s)

p='schema/scene-render-1.1.sch'
s=open(p).read()
a='''  <sch:pattern id="cinematic-voxel-objects">'''
i=s.index(a)
j=s.index("  </sch:pattern>",i)
rb='''    <sch:rule context="object3D/rigidBody">
      <sch:let name="voxels" value="boolean(parent::object3D[@primitive='voxels'])"/>
      <sch:let name="cells" value="boolean(parent::object3D[@primitive='voxels'] and (not(@shape) or @shape='auto' or @shape='voxels'))"/>
      <sch:let name="sx" value="number(concat(../@scaleX, substring('1', 1 + string-length(../@scaleX))))"/>
      <sch:let name="sy" value="number(concat(../@scaleY, substring('1', 1 + string-length(../@scaleY))))"/>
      <sch:let name="sz" value="number(concat(../@scaleZ, substring('1', 1 + string-length(../@scaleZ))))"/>
      <sch:assert id="VOX8" test="not(@shape='voxels') or $voxels">a rigidBody with shape voxels belongs to an object3D of primitive voxels.</sch:assert>
      <sch:assert id="VOX9" test="$cells or not(@density or @maxFragments or @fragmentMinCells or @fragmentOverflow or @anchor)">density, maxFragments, fragmentMinCells, fragmentOverflow and anchor belong to a rigidBody whose collider is the cells of an object of primitive voxels.</sch:assert>
      <sch:assert id="VOX10" test="not($cells) or (@density and not(@mass))">a body of cells has a density and no mass (its mass is its cells').</sch:assert>
      <sch:assert id="VOX11" test="not(@maxFragments or @fragmentMinCells or @fragmentOverflow) or ../crater or ../fracture">maxFragments, fragmentMinCells and fragmentOverflow belong to a body of cells that a crater or a fracture can break.</sch:assert>
      <sch:assert id="VOX12" test="not(@anchor) or ../crater">anchor belongs to a body of cells that has a crater.</sch:assert>
      <sch:assert id="VOX13" test="not($cells and (../crater or ../fracture)) or ($sx = $sy and $sy = $sz)">a body of cells that a crater or a fracture breaks is scaled the same on every axis.</sch:assert>
      <sch:assert id="VOX14" test="not($voxels and ../crater and ../fracture)">a body of cells has a crater or a fracture, not both.</sch:assert>
    </sch:rule>
'''
s=s[:j]+rb+s[j:]
a='''      <sch:assert id="CRT5" test="not(../rigidBody[not(@type='static' or @type='kinematic') or (@shape and not(@shape='auto' or @shape='trimesh'))])">crater rigid bodies require static/kinematic type with auto or trimesh collision geometry.</sch:assert>'''
assert a in s
s=s.replace(a,'''      <sch:assert id="CRT5" test="not(../rigidBody[not(@type='static' or @type='kinematic') or (@shape and not(@shape='auto' or @shape='trimesh' or @shape='voxels'))])">crater rigid bodies require static/kinematic type with auto, trimesh or voxels collision geometry.</sch:assert>
      <sch:assert id="CRT13" test="not(parent::object3D[@primitive='voxels']) or not(@mantle or @bulking or @repose)">a crater in an object of cells has no mantle, bulking or repose: the rim is cells (bulking 1, a fifth heaped) and not an analytic surface.</sch:assert>
      <sch:assert id="CRT14" test="not(parent::object3D[@primitive='voxels']) or not(@start or @end or @curve)">the cut of a crater in an object of cells is instantaneous at the impact, so start, end and curve have no meaning there.</sch:assert>
      <sch:assert id="CRT15" test="not(parent::object3D[@primitive='voxels']) or @source">a crater in an object of cells grows from a source: it is cut by an impact.</sch:assert>''')
a='      <sch:assert id="FRX3" test="@interiorMaterial=/scene/materials/material/@id">fracture interiorMaterial must reference a declared material.</sch:assert>'
assert a in s
s=s.replace(a,'''      <sch:assert id="FRX3" test="parent::object3D[@primitive='voxels'] or @interiorMaterial=/scene/materials/material/@id">fracture interiorMaterial must reference a declared material.</sch:assert>''')
assert "name()!='interiorMaterial' and name()!='source' and not((number(" in s
s=s.replace("name()!='interiorMaterial' and name()!='source' and not((number(","name()!='interiorMaterial' and name()!='source' and name()!='partition' and name()!='planes' and name()!='labels' and not((number(",1)
a='      <sch:assert id="FRX7"'
i=s.index(a)
j=s.index("</sch:assert>",i)+len("</sch:assert>")
add='''
      <sch:assert id="FRX8" test="not(parent::object3D[@primitive='voxels']) or not(@interiorMaterial or @interiorUvScale)">a fracture of an object of cells has no interior material: its pieces have the material of their cells.</sch:assert>
      <sch:assert id="FRX9" test="parent::object3D[@primitive='voxels'] or not(@partition or @planes or @labels)">partition, planes and labels belong to the fracture of an object of primitive voxels.</sch:assert>
      <sch:assert id="FRX10" test="not(@partition) or @partition='voronoi' or not(@pieces or @seed)">pieces and seed belong to a voronoi partition.</sch:assert>
      <sch:assert id="FRX11" test="(@partition='planes' or not(@planes)) and (not(@partition='planes') or (@planes and count(str:tokenize(normalize-space(@planes),' '))&gt;=4 and count(str:tokenize(normalize-space(@planes),' '))&lt;=252 and count(str:tokenize(normalize-space(@planes),' ')) mod 4 = 0 and count(str:tokenize(normalize-space(@planes),' ')[number(.)=number(.)]) = count(str:tokenize(normalize-space(@planes),' '))))">partition planes takes planes, from one to 63 planes of four numbers (nx ny nz offset), and planes belongs to that partition.</sch:assert>
      <sch:assert id="FRX12" test="(@partition='labels' or not(@labels)) and (not(@partition='labels') or @labels='material')">partition labels takes labels="material", and labels belongs to that partition.</sch:assert>'''
s=s[:j]+add+s[j:]
a='      <sch:assert id="P3D10"'
i=s.index(a)
j=s.index("</sch:assert>",i)+len("</sch:assert>")
s=s[:j]+'''
      <sch:assert id="CRT16" test="(@crater and /scene//object3D[@primitive='voxels' and crater/@id=current()/@crater] and not(@count)) or (not(@crater and /scene//object3D[@primitive='voxels' and crater/@id=current()/@crater]) and @count)">a burst needs count, except one from the crater of an object of cells, whose particles are the cells that the cut throws and have no count.</sch:assert>'''+s[j:]
open(p,'w').write(s)
