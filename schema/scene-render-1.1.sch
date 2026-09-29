<?xml version="1.0" encoding="UTF-8"?>
<!-- scene-render 1.1 cross-field rules. Run after XSD validation.
     ISO Schematron, XPath 1.0 + EXSLT strings (supported by libxslt / lxml / Saxon). -->
<sch:schema xmlns:sch="http://purl.oclc.org/dsdl/schematron" queryBinding="xslt">
  <sch:title>scene-render 1.1 co-occurrence and reference rules</sch:title>
  <sch:ns prefix="str" uri="http://exslt.org/strings"/>

  <!-- ============================================================ version gate -->
    <sch:pattern id="p1">
    <sch:rule context="/scene[@version='1.0']">
      <sch:assert id="V1" test="not(metadata|parameters|styles|colorManagement|layouts|safeAreas|paints|symbols|markers|tracking|captions)">
        version="1.0" documents cannot use 1.1 sections; set version="1.1".</sch:assert>
      <sch:assert id="V2" test="count(output) &lt;= 1">version="1.0" allows one output element.</sch:assert>
      <sch:assert id="V3" test="not(.//sequence|.//instance|.//include|.//repeat|.//adjustment|.//transition|.//skeleton|.//expression|.//motionPath|.//link|.//timeRemap|.//textAnimator|.//textPath|.//shapeModifier|.//transformConstraint)">
        version="1.0" documents cannot use 1.1 node or animation elements.</sch:assert>
      <sch:assert id="V4" test="not(assets/imageSequence|assets/lottie|assets/font|assets/generator|assets/chart|assets/audiogram|assets/code|assets/formula|assets/generated)">
        version="1.0" documents cannot use 1.1 asset kinds.</sch:assert>
    </sch:rule>
  </sch:pattern>
  <sch:pattern id="p2">
    <sch:rule context="vector">
      <sch:assert id="C1" test="not(@shape='path') or @path">vector shape="path" requires @path.</sch:assert>
      <sch:assert id="C2" test="not(@shape='svg') or @src">vector shape="svg" requires @src.</sch:assert>
    </sch:rule>
  </sch:pattern>
  <sch:pattern id="p3">
    <sch:rule context="shape">
      <sch:assert id="C3" test="not(@shape='path') or @path">shape shape="path" requires @path.</sch:assert>
    </sch:rule>
  </sch:pattern>
  <sch:pattern id="p4">
    <sch:rule context="mask">
      <sch:assert id="C4" test="not(@type='path') or @path">mask type="path" requires @path.</sch:assert>
      <sch:assert id="C5" test="@type='path' or (@width and @height)">mask type="<sch:value-of select="@type"/>" requires @width and @height.</sch:assert>
    </sch:rule>
  </sch:pattern>
  <sch:pattern id="p5">
    <sch:rule context="object3D">
      <sch:assert id="C6" test="not(@primitive='mesh') or @mesh">object3D primitive="mesh" requires @mesh.</sch:assert>
      <sch:assert id="C7" test="not(@primitive='text') or @text">object3D primitive="text" requires @text.</sch:assert>
      <sch:assert id="C8" test="not(@primitive='extrude') or @path">object3D primitive="extrude" requires @path.</sch:assert>
    </sch:rule>
  </sch:pattern>
  <sch:pattern id="p6">
    <sch:rule context="constraint[@type='pin']">
      <sch:assert id="C9" test="not(@b)">pin constraints tie body a to a world point; @b is not allowed.</sch:assert>
      <sch:assert id="C10" test="@x and @y">pin constraints require @x and @y.</sch:assert>
    </sch:rule>
  </sch:pattern>
  <sch:pattern id="p7">
    <sch:rule context="constraint[@type!='pin']">
      <sch:assert id="C11" test="@b">constraint type="<sch:value-of select="@type"/>" requires @b.</sch:assert>
    </sch:rule>
  </sch:pattern>
  <sch:pattern id="p8">
    <sch:rule context="assets/text">
      <sch:assert id="C12" test="(@text and not(span)) or (not(@text) and span)">text asset "<sch:value-of select="@id"/>" needs exactly one of @text or span children.</sch:assert>
      <sch:assert id="C13" test="not(@minSize and @maxSize) or number(@minSize) &lt;= number(@maxSize)">minSize must not exceed maxSize.</sch:assert>
      <sch:assert id="C14" test="not(@letterSpacing) or (number(@letterSpacing) &gt;= -number(@size) and number(@letterSpacing) &lt;= 4 * number(@size))">letterSpacing must lie in [-size, 4 x size].</sch:assert>
    </sch:rule>
  </sch:pattern>
  <sch:pattern id="p9">
    <sch:rule context="layer[timeRemap]">
      <sch:assert id="C15" test="(not(@speed) or number(@speed)=1) and (not(@reverse) or @reverse='false') and (not(@loop) or @loop='0')">
        layer "<sch:value-of select="@id"/>": timeRemap replaces speed, reverse and loop; remove them.</sch:assert>
    </sch:rule>
  </sch:pattern>
  <sch:pattern id="p10">
    <sch:rule context="layer[@fit and @fit!='none'] | instance[@fit and @fit!='none']">
      <sch:assert id="C16" test="@boxWidth and @boxHeight">fit="<sch:value-of select="@fit"/>" requires @boxWidth and @boxHeight.</sch:assert>
    </sch:rule>
  </sch:pattern>
  <sch:pattern id="p11">
    <sch:rule context="repeat">
      <sch:assert id="C17" test="(@count and not(@over)) or (@over and not(@count))">repeat needs exactly one of @count or @over.</sch:assert>
      <sch:assert id="C18" test="not(@over) or /scene/parameters/data[@id=current()/@over] or /scene/parameters/param[@id=current()/@over][@type='list']">repeat/@over must name a data source or a list parameter.</sch:assert>
    </sch:rule>
  </sch:pattern>
  <sch:pattern id="p12">
    <sch:rule context="effect[@type='lut' or @type='shader' or @type='displacement-map' or @type='gradient-map']">
      <sch:assert id="C19" test="@src or @source or @paint">effect type="<sch:value-of select="@type"/>" requires @src, @source or @paint.</sch:assert>
    </sch:rule>
  </sch:pattern>
  <sch:pattern id="p13">
    <sch:rule context="transition">
      <sch:assert id="C20" test="@from or @to">transition needs @from, @to or both.</sch:assert>
      <sch:assert id="C21" test="not(@type='shader') or @shader">transition type="shader" requires @shader.</sch:assert>
      <sch:assert id="C22" test="not(@type='luma') or @matte">transition type="luma" requires @matte.</sch:assert>
      <sch:assert id="C23" test="not(@from) or count(../*[@id=current()/@from])=1">transition/@from must be a sibling of the transition.</sch:assert>
      <sch:assert id="C24" test="not(@to) or count(../*[@id=current()/@to])=1">transition/@to must be a sibling of the transition.</sch:assert>
    </sch:rule>
  </sch:pattern>
  <sch:pattern id="p14">
    <sch:rule context="modifier">
      <sch:assert id="C25" test="not(@type='skin') or @skeleton">modifier type="skin" requires @skeleton.</sch:assert>
      <sch:assert id="C26" test="not(@type='corner-pin') or count(str:tokenize(normalize-space(@corners),' '))=8">modifier type="corner-pin" requires 8 numbers in @corners.</sch:assert>
    </sch:rule>
  </sch:pattern>
  <sch:pattern id="p15">
    <sch:rule context="transformConstraint">
      <sch:assert id="C27" test="not(@type='follow-path') or @path">follow-path requires @path.</sch:assert>
      <sch:assert id="C28" test="@type='follow-path' or @target">transformConstraint type="<sch:value-of select="@type"/>" requires @target.</sch:assert>
      <sch:assert id="C29" test="not(@type='track') or /scene/tracking/trackData[@id=current()/@target]">track constraints target trackData.</sch:assert>
    </sch:rule>
  </sch:pattern>
  <sch:pattern id="p16">
    <sch:rule context="safeArea[@preset='custom' or not(@preset)]">
      <sch:assert id="C30" test="@top and @right and @bottom and @left">custom safe areas require top, right, bottom and left.</sch:assert>
    </sch:rule>
  </sch:pattern>
  <sch:pattern id="p17">
    <sch:rule context="captionTrack">
      <sch:assert id="C31" test="count(cue[1]|@src|@transcribe)=1">captionTrack "<sch:value-of select="@id"/>" needs exactly one source: cue children, @src or @transcribe.</sch:assert>
      <sch:assert id="C32" test="not(@transcribe) or (@cache and @cacheSha256)">transcribed captions require @cache and @cacheSha256 (deterministic renders).</sch:assert>
      <sch:assert id="C33" test="not(@transcribe) or /scene/audioMix/audioTrack[@id=current()/@transcribe]">captionTrack/@transcribe must name an audioTrack.</sch:assert>
    </sch:rule>
  </sch:pattern>
  <sch:pattern id="p18">
    <sch:rule context="audioMix">
      <sch:assert id="C34" test="count(master) &lt;= 1">audioMix allows at most one master.</sch:assert>
      <sch:assert id="C35" test="audioTrack">audioMix needs at least one audioTrack.</sch:assert>
    </sch:rule>
  </sch:pattern>
  <sch:pattern id="p19">
    <sch:rule context="linearGradient|radialGradient|conicGradient">
      <sch:assert id="C36" test="count(stop) &gt;= 2">gradient "<sch:value-of select="@id"/>" needs at least two stops.</sch:assert>
      <sch:assert id="C37" test="not(stop[number(@offset) &lt; number(preceding-sibling::stop[1]/@offset)])">gradient stop offsets must be non-decreasing.</sch:assert>
    </sch:rule>
  </sch:pattern>
  <sch:pattern id="p20">
    <sch:rule context="animate|timeRemap">
      <sch:assert id="C38" test="not(key[number(@time) &lt; number(preceding-sibling::key[1]/@time)])">key times must be non-decreasing.</sch:assert>
    </sch:rule>
  </sch:pattern>
  <sch:pattern id="p21">
    <sch:rule context="key[@interpolation='steps']">
      <sch:assert id="C39" test="@steps">interpolation="steps" requires @steps.</sch:assert>
    </sch:rule>
  </sch:pattern>
  <sch:pattern id="p22">
    <sch:rule context="key[@interpolation='cubic-bezier']">
      <sch:assert id="C40" test="@bezier or @easeOut or following-sibling::key[1]/@easeIn">cubic-bezier keys need @bezier or easeOut/easeIn handles.</sch:assert>
    </sch:rule>
  </sch:pattern>
  <sch:pattern id="p23">
    <sch:rule context="generated">
      <sch:assert id="C41" test="@prompt or @kind='speech'">generated media other than speech requires @prompt.</sch:assert>
    </sch:rule>
  </sch:pattern>
  <sch:pattern id="p24">
    <sch:rule context="output">
      <sch:assert id="C42" test="not(@proresProfile) or @codec='prores'">proresProfile applies only to codec="prores".</sch:assert>
      <sch:assert id="C43" test="not(@alpha='true') or @codec='prores' or @codec='vp9' or @codec='ffv1' or @codec='png-sequence' or @codec='exr-sequence' or @codec='tiff-sequence' or @codec='apng' or @codec='webp' or @codec='gif'">alpha="true" needs a codec that carries alpha.</sch:assert>
      <sch:assert id="C44" test="not(@end) or (not(@start) and number(@end) &gt; 0) or number(@end) &gt; number(@start)">output end must be after start.</sch:assert>
    </sch:rule>
  </sch:pattern>
  <sch:pattern id="p25">
    <sch:rule context="layer">
      <sch:assert id="R1" test="/scene/assets/*[@id=current()/@asset]">layer "<sch:value-of select="@id"/>": @asset must reference an element of assets.</sch:assert>
      <sch:assert id="R2" test="not(textAnimator or textPath) or /scene/assets/text[@id=current()/@asset]">textAnimator and textPath need a text asset.</sch:assert>
      <sch:assert id="R3" test="not(@audioBus) or /scene/audioMix/bus[@id=current()/@audioBus]">layer/@audioBus must name a bus.</sch:assert>
    </sch:rule>
  </sch:pattern>
  <sch:pattern id="p26">
    <sch:rule context="object3D">
      <sch:assert id="R4" test="not(@material) or /scene/materials/material[@id=current()/@material]">object3D/@material must name a material.</sch:assert>
      <sch:assert id="R5" test="not(@mesh) or /scene/assets/mesh[@id=current()/@mesh]">object3D/@mesh must name a mesh asset.</sch:assert>
    </sch:rule>
  </sch:pattern>
  <sch:pattern id="p27">
    <sch:rule context="*[@effects]">
      <sch:assert id="R6" test="count(str:tokenize(normalize-space(@effects),' ')) = count(/scene/effects/effect[contains(concat(' ',normalize-space(current()/@effects),' '), concat(' ',@id,' '))])">
        every id in @effects of "<sch:value-of select="@id"/>" must name an effect.</sch:assert>
    </sch:rule>
  </sch:pattern>
  <sch:pattern id="p28">
    <sch:rule context="effect[@lights]">
      <sch:assert id="R7" test="count(str:tokenize(normalize-space(@lights),' ')) = count(/scene/lights/light[contains(concat(' ',normalize-space(current()/@lights),' '), concat(' ',@id,' '))])">every id in effect/@lights must name a light.</sch:assert>
    </sch:rule>
  </sch:pattern>
  <sch:pattern id="p29">
    <sch:rule context="*[@matte][not(self::transition)]">
      <sch:assert id="R8" test="/scene/composition//*[@id=current()/@matte] or /scene/symbols//*[@id=current()/@matte]">@matte must name a composition node.</sch:assert>
      <sch:assert id="R9" test="@matte != @id">a node cannot be its own matte.</sch:assert>
    </sch:rule>
  </sch:pattern>
  <sch:pattern id="p30">
    <sch:rule context="*[@parent][@id]">
      <sch:assert id="R10" test="@parent != @id">a node cannot parent itself.</sch:assert>
    </sch:rule>
  </sch:pattern>
  <sch:pattern id="p31">
    <sch:rule context="instance">
      <sch:assert id="R11" test="/scene/symbols/symbol[@id=current()/@symbol]">instance/@symbol must name a symbol.</sch:assert>
    </sch:rule>
  </sch:pattern>
  <sch:pattern id="p32">
    <sch:rule context="output">
      <sch:assert id="R12" test="not(@layout) or /scene/layouts/layout[@id=current()/@layout]">output/@layout must name a layout.</sch:assert>
      <sch:assert id="R13" test="not(@variant) or /scene/parameters/variant[@id=current()/@variant]">output/@variant must name a variant.</sch:assert>
      <sch:assert id="R14" test="not(@burnCaptions) or /scene/captions/captionTrack[@id=current()/@burnCaptions]">output/@burnCaptions must name a captionTrack.</sch:assert>
    </sch:rule>
  </sch:pattern>
  <sch:pattern id="p33">
    <sch:rule context="audioTrack">
      <sch:assert id="R15" test="/scene/assets/audio[@id=current()/@asset] or /scene/assets/generated[@id=current()/@asset][@kind='speech' or @kind='music' or @kind='sound-effect'] or /scene/assets/video[@id=current()/@asset][@hasAudio='true' or @hasAudio='1']">audioTrack/@asset must name audio, generated audio, or video with hasAudio="true".</sch:assert>
      <sch:assert id="R16" test="not(@bus) or /scene/audioMix/bus[@id=current()/@bus]">audioTrack/@bus must name a bus.</sch:assert>
      <sch:assert id="R17" test="not(@duckUnder) or not(contains(concat(' ',normalize-space(@duckUnder),' '), concat(' ',@id,' ')))">a track cannot duck under itself.</sch:assert>
    </sch:rule>
  </sch:pattern>
  <sch:pattern id="p34">
    <sch:rule context="bind">
      <sch:assert id="R18" test="/scene/parameters/param[@id=current()/@param]">bind/@param must name a param.</sch:assert>
    </sch:rule>
  </sch:pattern>
  <sch:pattern id="p35">
    <sch:rule context="project[@safeArea] | layout[@safeArea] | captionTrack[@safeArea]">
      <sch:assert id="R19" test="/scene/safeAreas/safeArea[@id=current()/@safeArea]">@safeArea must name a safeArea.</sch:assert>
    </sch:rule>
  </sch:pattern>
  <sch:pattern id="p36">
    <sch:rule context="camera[@focusTarget] | camera[@target]">
      <sch:assert id="R20" test="(not(@focusTarget) or /scene/composition//*[@id=current()/@focusTarget]) and (not(@target) or /scene/composition//*[@id=current()/@target])">camera targets must name composition nodes.</sch:assert>
    </sch:rule>
  </sch:pattern>
  <sch:pattern id="p37">
    <sch:rule context="*[@startMarker] | *[@endMarker] | key[@marker]">
      <sch:assert id="R21" test="(not(@startMarker) or /scene/markers/marker[@id=current()/@startMarker]) and (not(@endMarker) or /scene/markers/marker[@id=current()/@endMarker]) and (not(@marker) or /scene/markers/marker[@id=current()/@marker])">marker references must name markers.</sch:assert>
    </sch:rule>
  </sch:pattern>
  <sch:pattern id="p38">
    <sch:rule context="textStyle[@basedOn] | *[@style][not(self::audiogram)]">
      <sch:assert id="R22" test="(not(@basedOn) or /scene/styles/textStyle[@id=current()/@basedOn]) and (not(@style) or /scene/styles/textStyle[@id=current()/@style])">text style references must name textStyle elements.</sch:assert>
    </sch:rule>
  </sch:pattern>
  <sch:pattern id="p39">
    <sch:rule context="*[@fontAsset]">
      <sch:assert id="R23" test="/scene/assets/font[@id=current()/@fontAsset]">@fontAsset must name a font asset.</sch:assert>
    </sch:rule>
  </sch:pattern>
  <sch:pattern id="p40">
        <sch:rule context="*[@fill or @stroke or @color or @background or @paint or @activeColor or @highlight or @strokeColor]">
      <sch:assert id="R24-fill" test="not(starts-with(@fill,'url(#')) or /scene/paints/*[@id=substring-before(substring-after(current()/@fill,'url(#'),')')]">@fill: url(#id) must name an element of paints.</sch:assert>
      <sch:assert id="R24-stroke" test="not(starts-with(@stroke,'url(#')) or /scene/paints/*[@id=substring-before(substring-after(current()/@stroke,'url(#'),')')]">@stroke: url(#id) must name an element of paints.</sch:assert>
      <sch:assert id="R24-color" test="not(starts-with(@color,'url(#')) or /scene/paints/*[@id=substring-before(substring-after(current()/@color,'url(#'),')')]">@color: url(#id) must name an element of paints.</sch:assert>
      <sch:assert id="R24-background" test="not(starts-with(@background,'url(#')) or /scene/paints/*[@id=substring-before(substring-after(current()/@background,'url(#'),')')]">@background: url(#id) must name an element of paints.</sch:assert>
      <sch:assert id="R24-paint" test="not(starts-with(@paint,'url(#')) or /scene/paints/*[@id=substring-before(substring-after(current()/@paint,'url(#'),')')]">@paint: url(#id) must name an element of paints.</sch:assert>
      <sch:assert id="R24-activeColor" test="not(starts-with(@activeColor,'url(#')) or /scene/paints/*[@id=substring-before(substring-after(current()/@activeColor,'url(#'),')')]">@activeColor: url(#id) must name an element of paints.</sch:assert>
      <sch:assert id="R24-highlight" test="not(starts-with(@highlight,'url(#')) or /scene/paints/*[@id=substring-before(substring-after(current()/@highlight,'url(#'),')')]">@highlight: url(#id) must name an element of paints.</sch:assert>
      <sch:assert id="R24-strokeColor" test="not(starts-with(@strokeColor,'url(#')) or /scene/paints/*[@id=substring-before(substring-after(current()/@strokeColor,'url(#'),')')]">@strokeColor: url(#id) must name an element of paints.</sch:assert>
    </sch:rule>
  </sch:pattern>
  <sch:pattern id="p41">
    <sch:rule context="*[@fill or @stroke or @color or @background or @paint or @activeColor or @highlight or @strokeColor]">
      <sch:assert id="R25-fill" test="not(starts-with(@fill,'var(--')) or /scene/styles/token[@name=substring-before(substring-after(current()/@fill,'var(--'),')')]">@fill: var(--name) must name a style token.</sch:assert>
      <sch:assert id="R25-stroke" test="not(starts-with(@stroke,'var(--')) or /scene/styles/token[@name=substring-before(substring-after(current()/@stroke,'var(--'),')')]">@stroke: var(--name) must name a style token.</sch:assert>
      <sch:assert id="R25-color" test="not(starts-with(@color,'var(--')) or /scene/styles/token[@name=substring-before(substring-after(current()/@color,'var(--'),')')]">@color: var(--name) must name a style token.</sch:assert>
      <sch:assert id="R25-background" test="not(starts-with(@background,'var(--')) or /scene/styles/token[@name=substring-before(substring-after(current()/@background,'var(--'),')')]">@background: var(--name) must name a style token.</sch:assert>
      <sch:assert id="R25-paint" test="not(starts-with(@paint,'var(--')) or /scene/styles/token[@name=substring-before(substring-after(current()/@paint,'var(--'),')')]">@paint: var(--name) must name a style token.</sch:assert>
      <sch:assert id="R25-activeColor" test="not(starts-with(@activeColor,'var(--')) or /scene/styles/token[@name=substring-before(substring-after(current()/@activeColor,'var(--'),')')]">@activeColor: var(--name) must name a style token.</sch:assert>
      <sch:assert id="R25-highlight" test="not(starts-with(@highlight,'var(--')) or /scene/styles/token[@name=substring-before(substring-after(current()/@highlight,'var(--'),')')]">@highlight: var(--name) must name a style token.</sch:assert>
      <sch:assert id="R25-strokeColor" test="not(starts-with(@strokeColor,'var(--')) or /scene/styles/token[@name=substring-before(substring-after(current()/@strokeColor,'var(--'),')')]">@strokeColor: var(--name) must name a style token.</sch:assert>
    </sch:rule>
  </sch:pattern>
  <sch:pattern id="p42">
    <sch:rule context="geoLayer">
      <sch:assert id="R36" test="/scene/assets/geo[@id=current()/@geo]">geoLayer/@geo must name a geo asset.</sch:assert>
    </sch:rule>
    <sch:rule context="route">
      <sch:assert id="R37" test="not(@geo) or /scene/assets/geo[@id=current()/@geo]">route/@geo must name a geo asset.</sch:assert>
      <sch:assert id="C45" test="@points or @geo">route needs @points or @geo.</sch:assert>
    </sch:rule>
    <sch:rule context="map">
      <sch:assert id="R26" test="not(@fit) or count(str:tokenize(normalize-space(@fit),' ')) = count(/scene/assets/geo[contains(concat(' ',normalize-space(current()/@fit),' '), concat(' ',@id,' '))])">every id in map/@fit must name a geo asset.</sch:assert>
    </sch:rule>
  </sch:pattern>
  <!-- ============================================================ SREP 8: version gate, references, bounds -->
  <sch:pattern id="p50">
    <sch:rule context="/scene[@version='1.0']">
      <sch:assert id="V6" test="not(.//audioEffect|.//blob|.//burst|.//bus|.//destination|.//erosion|.//flock|.//fluid|.//geo|.//map|.//master|.//param|.//pin|.//poster|.//representation|.//shake|.//slime|.//span|.//thumbnail)">
        version="1.0" documents cannot use 1.1 elements (simulation nodes, geo and map assets, clay blobs,
        audio buses and effects, output posters, thumbnails and destinations, camera shake, representations,
        text spans, effect params); set version="1.1".</sch:assert>
      <sch:assert id="V7" test="not(.//object3D[@primitive='capsule' or @primitive='clay' or @primitive='cone' or @primitive='cylinder' or @primitive='extrude' or @primitive='text' or @primitive='torus'])">
        version="1.0" documents cannot use the 1.1 object3D primitives (capsule, clay, cone, cylinder, extrude, text, torus); set version="1.1".</sch:assert>
    </sch:rule>
  </sch:pattern>
  <sch:pattern id="p51">
    <sch:rule context="*[@colorEnd or @colorHigh or @colorLow or @headFill or @noData or @outline or @paint2]">
      <sch:assert id="R30-colorEnd" test="not(starts-with(@colorEnd,'url(#')) or /scene/paints/*[@id=substring-before(substring-after(current()/@colorEnd,'url(#'),')')]">@colorEnd: url(#id) must name an element of paints.</sch:assert>
      <sch:assert id="R30-colorHigh" test="not(starts-with(@colorHigh,'url(#')) or /scene/paints/*[@id=substring-before(substring-after(current()/@colorHigh,'url(#'),')')]">@colorHigh: url(#id) must name an element of paints.</sch:assert>
      <sch:assert id="R30-colorLow" test="not(starts-with(@colorLow,'url(#')) or /scene/paints/*[@id=substring-before(substring-after(current()/@colorLow,'url(#'),')')]">@colorLow: url(#id) must name an element of paints.</sch:assert>
      <sch:assert id="R30-headFill" test="not(starts-with(@headFill,'url(#')) or /scene/paints/*[@id=substring-before(substring-after(current()/@headFill,'url(#'),')')]">@headFill: url(#id) must name an element of paints.</sch:assert>
      <sch:assert id="R30-noData" test="not(starts-with(@noData,'url(#')) or /scene/paints/*[@id=substring-before(substring-after(current()/@noData,'url(#'),')')]">@noData: url(#id) must name an element of paints.</sch:assert>
      <sch:assert id="R30-outline" test="not(starts-with(@outline,'url(#')) or /scene/paints/*[@id=substring-before(substring-after(current()/@outline,'url(#'),')')]">@outline: url(#id) must name an element of paints.</sch:assert>
      <sch:assert id="R30-paint2" test="not(starts-with(@paint2,'url(#')) or /scene/paints/*[@id=substring-before(substring-after(current()/@paint2,'url(#'),')')]">@paint2: url(#id) must name an element of paints.</sch:assert>
    </sch:rule>
  </sch:pattern>
  <sch:pattern id="p52">
    <sch:rule context="*[@attenuationColor or @baseColor or @colorEnd or @colorHigh or @colorLow or @emissive or @foreground or @headFill or @keyColor or @noData or @outline or @paint2 or @shadowColor or @sheenColor or @specularColor]">
      <sch:assert id="R31-attenuationColor" test="not(starts-with(@attenuationColor,'var(--')) or /scene/styles/token[@name=substring-before(substring-after(current()/@attenuationColor,'var(--'),')')]">@attenuationColor: var(--name) must name a styles/token.</sch:assert>
      <sch:assert id="R31-baseColor" test="not(starts-with(@baseColor,'var(--')) or /scene/styles/token[@name=substring-before(substring-after(current()/@baseColor,'var(--'),')')]">@baseColor: var(--name) must name a styles/token.</sch:assert>
      <sch:assert id="R31-colorEnd" test="not(starts-with(@colorEnd,'var(--')) or /scene/styles/token[@name=substring-before(substring-after(current()/@colorEnd,'var(--'),')')]">@colorEnd: var(--name) must name a styles/token.</sch:assert>
      <sch:assert id="R31-colorHigh" test="not(starts-with(@colorHigh,'var(--')) or /scene/styles/token[@name=substring-before(substring-after(current()/@colorHigh,'var(--'),')')]">@colorHigh: var(--name) must name a styles/token.</sch:assert>
      <sch:assert id="R31-colorLow" test="not(starts-with(@colorLow,'var(--')) or /scene/styles/token[@name=substring-before(substring-after(current()/@colorLow,'var(--'),')')]">@colorLow: var(--name) must name a styles/token.</sch:assert>
      <sch:assert id="R31-emissive" test="not(starts-with(@emissive,'var(--')) or /scene/styles/token[@name=substring-before(substring-after(current()/@emissive,'var(--'),')')]">@emissive: var(--name) must name a styles/token.</sch:assert>
      <sch:assert id="R31-foreground" test="not(starts-with(@foreground,'var(--')) or /scene/styles/token[@name=substring-before(substring-after(current()/@foreground,'var(--'),')')]">@foreground: var(--name) must name a styles/token.</sch:assert>
      <sch:assert id="R31-headFill" test="not(starts-with(@headFill,'var(--')) or /scene/styles/token[@name=substring-before(substring-after(current()/@headFill,'var(--'),')')]">@headFill: var(--name) must name a styles/token.</sch:assert>
      <sch:assert id="R31-keyColor" test="not(starts-with(@keyColor,'var(--')) or /scene/styles/token[@name=substring-before(substring-after(current()/@keyColor,'var(--'),')')]">@keyColor: var(--name) must name a styles/token.</sch:assert>
      <sch:assert id="R31-noData" test="not(starts-with(@noData,'var(--')) or /scene/styles/token[@name=substring-before(substring-after(current()/@noData,'var(--'),')')]">@noData: var(--name) must name a styles/token.</sch:assert>
      <sch:assert id="R31-outline" test="not(starts-with(@outline,'var(--')) or /scene/styles/token[@name=substring-before(substring-after(current()/@outline,'var(--'),')')]">@outline: var(--name) must name a styles/token.</sch:assert>
      <sch:assert id="R31-paint2" test="not(starts-with(@paint2,'var(--')) or /scene/styles/token[@name=substring-before(substring-after(current()/@paint2,'var(--'),')')]">@paint2: var(--name) must name a styles/token.</sch:assert>
      <sch:assert id="R31-shadowColor" test="not(starts-with(@shadowColor,'var(--')) or /scene/styles/token[@name=substring-before(substring-after(current()/@shadowColor,'var(--'),')')]">@shadowColor: var(--name) must name a styles/token.</sch:assert>
      <sch:assert id="R31-sheenColor" test="not(starts-with(@sheenColor,'var(--')) or /scene/styles/token[@name=substring-before(substring-after(current()/@sheenColor,'var(--'),')')]">@sheenColor: var(--name) must name a styles/token.</sch:assert>
      <sch:assert id="R31-specularColor" test="not(starts-with(@specularColor,'var(--')) or /scene/styles/token[@name=substring-before(substring-after(current()/@specularColor,'var(--'),')')]">@specularColor: var(--name) must name a styles/token.</sch:assert>
    </sch:rule>
  </sch:pattern>
  <sch:pattern id="p53">
    <sch:rule context="*[@shape='sprite'][self::flock or self::particleEmitter]">
      <sch:assert id="C50" test="@sprite">shape="sprite" requires @sprite.</sch:assert>
    </sch:rule>
  </sch:pattern>
  <sch:pattern id="p54">
    <sch:rule context="*[@sprite][self::flock or self::particleEmitter]">
      <sch:assert id="R32" test="/scene/assets/*[self::image or self::imageSequence or self::video or self::generator][@id=current()/@sprite]">@sprite must name an image, image sequence, video or generator asset.</sch:assert>
    </sch:rule>
  </sch:pattern>
  <sch:pattern id="p55">
    <sch:rule context="erosion[@heightmap]">
      <sch:assert id="R33" test="/scene/assets/image[@id=current()/@heightmap]">erosion/@heightmap must name an image asset.</sch:assert>
    </sch:rule>
  </sch:pattern>
  <sch:pattern id="p56">
    <sch:rule context="*[@forceFields]">
      <sch:assert id="R34" test="count(str:tokenize(normalize-space(@forceFields),' ')) = count(/scene/physics/forceField[contains(concat(' ',normalize-space(current()/@forceFields),' '), concat(' ',@id,' '))])">
        every id in @forceFields of "<sch:value-of select="@id"/>" must name a physics/forceField.</sch:assert>
    </sch:rule>
  </sch:pattern>
  <sch:pattern id="p57">
    <sch:rule context="*[@textStyle]">
      <sch:assert id="R35" test="/scene/styles/textStyle[@id=current()/@textStyle]">@textStyle must name a styles/textStyle.</sch:assert>
    </sch:rule>
  </sch:pattern>
  <sch:pattern id="p58">
    <sch:rule context="object3D[@primitive='clay']">
      <sch:assert id="C51" test="blob">object3D primitive="clay" needs at least one blob.</sch:assert>
    </sch:rule>
  </sch:pattern>
  <sch:pattern id="p59">
    <sch:rule context="fluidSource[@start and @end]">
      <sch:assert id="C52" test="number(@end) &gt; number(@start)">fluidSource end must be after start.</sch:assert>
    </sch:rule>
  </sch:pattern>
  <sch:pattern id="p60">
    <sch:rule context="geoLayer[@domain]">
      <sch:assert id="C53" test="not(str:tokenize(normalize-space(@domain),' ')[position() &gt; 1][number(.) &lt;= number(preceding-sibling::*[1])])">@domain values must increase.</sch:assert>
    </sch:rule>
  </sch:pattern>
</sch:schema>
