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
  <sch:pattern id="p1b">
    <sch:rule context="/scene[@version='1.0' or @version='1.1']">
      <sch:assert id="V5" test="not(assets/tiles|.//basemap|.//object3D/rigidBody|.//object3D/morph|.//object3D/joint|assets/strokeFont|.//shape[@shape='stroke-text']|.//object3D[@primitive='map' or @primitive='globe']|output/segment|output/audioTrack|output/captionTrack)">
        documents before version="1.2" cannot use 1.2 elements or asset kinds; set version="1.2".</sch:assert>
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
      <sch:assert id="C17" test="count(@count) + count(@over) + number(boolean(points)) = 1">repeat needs exactly one of @count, @over or a points child.</sch:assert>
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
      <sch:assert id="C20" test="@from or @to or parent::segment">transition needs @from, @to or both.</sch:assert>
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
      <sch:assert id="C33" test="not(@transcribe) or parent::output or /scene/audioMix/audioTrack[@id=current()/@transcribe]">captionTrack/@transcribe must name an audioTrack.</sch:assert>
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
      <sch:assert id="C47" test="not(@primitive='map' or @primitive='globe') or @map">object3D primitive="map" or "globe" requires @map.</sch:assert>
      <sch:assert id="R28" test="not(@map) or /scene/assets/map[@id=current()/@map]">object3D/@map must name a map asset.</sch:assert>
      <sch:assert id="R29" test="not(@terrain) or /scene/assets/tiles[@id=current()/@terrain]">object3D/@terrain must name a tiles asset.</sch:assert>
      <sch:assert id="R4" test="not(@material) or /scene/materials/material[@id=current()/@material]">object3D/@material must name a material.</sch:assert>
      <sch:assert id="R5" test="not(@mesh) or /scene/assets/*[(self::mesh or self::meshSequence) and @id=current()/@mesh]">object3D/@mesh must name a mesh asset.</sch:assert>
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
      <sch:assert id="R14" test="not(@burnCaptions) or /scene/captions/captionTrack[@id=current()/@burnCaptions] or captionTrack[@id=current()/@burnCaptions]">output/@burnCaptions must name a captionTrack.</sch:assert>
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
      <sch:assert id="R21" test="(not(@startMarker) or /scene/markers/marker[@id=current()/@startMarker] or (/scene/markers/beatGrid and ((substring-before(@startMarker,'.')='beat' or substring-before(@startMarker,'.')='bar') and string-length(substring-after(@startMarker,'.')) &gt; 0 and string-length(substring-after(@startMarker,'.')) &lt; 20 and translate(substring-after(@startMarker,'.'),'0123456789','')=''))) and (not(@endMarker) or /scene/markers/marker[@id=current()/@endMarker] or (/scene/markers/beatGrid and ((substring-before(@endMarker,'.')='beat' or substring-before(@endMarker,'.')='bar') and string-length(substring-after(@endMarker,'.')) &gt; 0 and string-length(substring-after(@endMarker,'.')) &lt; 20 and translate(substring-after(@endMarker,'.'),'0123456789','')=''))) and (not(@marker) or /scene/markers/marker[@id=current()/@marker] or (/scene/markers/beatGrid and ((substring-before(@marker,'.')='beat' or substring-before(@marker,'.')='bar') and string-length(substring-after(@marker,'.')) &gt; 0 and string-length(substring-after(@marker,'.')) &lt; 20 and translate(substring-after(@marker,'.'),'0123456789','')='')))">marker references must name a marker, or a beat.N or bar.N id of the scene's beatGrid.</sch:assert>
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
    <sch:rule context="basemap">
      <sch:assert id="R27" test="/scene/assets/tiles[@id=current()/@tiles]">basemap/@tiles must name a tiles asset.</sch:assert>
    </sch:rule>
    <sch:rule context="tiles">
      <sch:assert id="C46" test="@src or (@url and @cache and @cacheSha256)">tiles need @src, or @url with @cache and @cacheSha256.</sch:assert>
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
  <sch:pattern id="p43">
    <sch:rule context="object3D/rigidBody">
      <sch:assert id="C48" test="not(@shape='trimesh') or @type='static' or @type='kinematic'">a trimesh rigidBody must be static or kinematic.</sch:assert>
    </sch:rule>
  </sch:pattern>
  <sch:pattern id="p61">
    <sch:rule context="output">
      <sch:let name="mixTracks" value="/scene/audioMix/audioTrack/@id"/>
      <sch:let name="mixBuses" value="/scene/audioMix/bus/@id"/>
      <sch:assert id="C54" test="not(segment) or ((not(@start) or number(@start) = 0) and not(@end))">an output with segments cannot also set start or end; put the range in a segment instead.</sch:assert>
      <sch:assert id="R39" test="not(str:tokenize(normalize-space(@audioTracks),' ')[not(. = $mixTracks)]) and not(str:tokenize(normalize-space(@audioBuses),' ')[not(. = $mixBuses)])">every id in output/@audioTracks must name an audioMix track, and every id in output/@audioBuses a bus.</sch:assert>
      <sch:assert id="R40" test="not(@overlay) or /scene/symbols/symbol[@id=current()/@overlay]">output/@overlay must name a symbol.</sch:assert>
    </sch:rule>
    <sch:rule context="segment">
      <sch:assert id="C55" test="timeRemap or ((@from or @fromMarker) and (@to or @toMarker))">a segment needs from (or fromMarker) and to (or toMarker), or a timeRemap.</sch:assert>
      <sch:assert id="C56" test="not(@from and @to) or (number(@from) &gt;= 0 and number(@to) &gt; number(@from) and number(@to) &lt;= number(/scene/project/@duration))">segment from and to must satisfy 0 ≤ from &lt; to ≤ project/@duration.</sch:assert>
      <sch:assert id="C57" test="not(@from and @fromMarker) and not(@to and @toMarker)">a segment gives each end as a time or as a marker, not both.</sch:assert>
      <sch:assert id="C58" test="count(timeRemap) &lt;= 1 and count(transition) &lt;= 1">a segment has at most one timeRemap and one transition.</sch:assert>
      <sch:assert id="R38" test="(not(@fromMarker) or /scene/markers/marker[@id=current()/@fromMarker]) and (not(@toMarker) or /scene/markers/marker[@id=current()/@toMarker])">segment markers must name markers.</sch:assert>
    </sch:rule>
    <sch:rule context="segment/transition">
      <sch:assert id="C59" test="not(@from or @to) and not(@type='morph' or @type='luma')">a segment transition joins two rendered pictures: no from, no to, not morph and not luma.</sch:assert>
    </sch:rule>
    <sch:rule context="output/captionTrack[@transcribe]">
      <sch:assert id="R41" test="../audioTrack[@id=current()/@transcribe]">an output caption track transcribes one of that output's own audio tracks.</sch:assert>
    </sch:rule>
  </sch:pattern>
  <sch:pattern id="p66">
    <sch:rule context="shape[@markerStart[.!='none'] or @markerEnd[.!='none']]">
      <sch:assert id="C65" test="@shape='path' or @shape='line'">markers need an open outline: shape="path" or "line".</sch:assert>
    </sch:rule>
  </sch:pattern>
  <sch:pattern id="p70">
    <sch:rule context="/scene[@version='1.0' or @version='1.1']">
      <sch:assert id="V11" test="not(.//connector)">connector needs version="1.2".</sch:assert>
    </sch:rule>
  </sch:pattern>
  <sch:pattern id="p71">
    <sch:rule context="connector">
      <sch:assert id="C60" test="(@from or (@fromX and @fromY)) and (@to or (@toX and @toY))">a connector end needs a node (@from, @to) or a point (@fromX and @fromY, @toX and @toY).</sch:assert>
      <sch:assert id="C61" test="not(@fromAnchor[.!='auto'] and (@fromX or @fromY)) and not(@toAnchor[.!='auto'] and (@toX or @toY))">an anchor keyword and an explicit anchor point exclude each other.</sch:assert>
      <sch:assert id="C62" test="count(@fromX|@fromY) != 1 and count(@toX|@toY) != 1">@fromX and @fromY (and @toX and @toY) come together.</sch:assert>
      <sch:assert id="C63" test="not(@route='curved' and @points)">route="curved" takes no @points.</sch:assert>
      <sch:assert id="C64" test="not(animate[@property='x' or @property='y' or @property='rotation' or @property='scaleX' or @property='scaleY' or @property='anchorX' or @property='anchorY' or @property='skewX' or @property='skewY']) and not(expression[@property!='opacity'])">a connector has no transform of its own: its geometry comes from its ends.</sch:assert>
      <sch:assert id="R48-from" test="not(@from) or (ancestor::symbol and ancestor::symbol[1]//*[@id=current()/@from][self::group or self::layer or self::shape or self::instance][not(ancestor::repeat)][not(ancestor-or-self::*[@threeD='true'])]) or (not(ancestor::symbol) and /scene/composition//*[@id=current()/@from][self::group or self::layer or self::shape or self::instance][not(ancestor::repeat)][not(ancestor-or-self::*[@threeD='true'])])">@from must name a group, layer, shape or instance in the same composition or symbol, outside any repeat and not 2.5D.</sch:assert>
      <sch:assert id="R48-to" test="not(@to) or (ancestor::symbol and ancestor::symbol[1]//*[@id=current()/@to][self::group or self::layer or self::shape or self::instance][not(ancestor::repeat)][not(ancestor-or-self::*[@threeD='true'])]) or (not(ancestor::symbol) and /scene/composition//*[@id=current()/@to][self::group or self::layer or self::shape or self::instance][not(ancestor::repeat)][not(ancestor-or-self::*[@threeD='true'])])">@to must name a group, layer, shape or instance in the same composition or symbol, outside any repeat and not 2.5D.</sch:assert>
      <sch:assert id="R49" test="not(//transformConstraint[@target=current()/@id] | //*[@parent=current()/@id] | //link[starts-with(@source, concat(current()/@id, '.'))])">nothing may be positioned by a connector (transform parent, constraint target, link source).</sch:assert>
      <sch:assert id="R50" test="not(@label) or /scene/assets/text[@id=current()/@label]">@label must name a text asset.</sch:assert>
    </sch:rule>
  </sch:pattern>
  <sch:pattern id="p72">
    <sch:rule context="/scene[@version='1.0' or @version='1.1']">
      <sch:assert id="V9" test="not(assets/pdf)">pdf assets need version="1.2".</sch:assert>
    </sch:rule>
  </sch:pattern>
  <sch:pattern id="p73">
    <sch:rule context="assets/pdf">
      <sch:assert id="C66" test="@sha256">a pdf asset pins its source with @sha256.</sch:assert>
    </sch:rule>
    <sch:rule context="shape[@region]">
      <sch:assert id="C67" test="@regionLayer">@region needs @regionLayer, the layer that shows the page.</sch:assert>
      <sch:assert id="C68" test="not(@parent) and not(transformConstraint)">a shape placed on a region has no other transform parent or constraint.</sch:assert>
      <sch:assert id="R51" test="/scene/assets/pdf/region[@id=current()/@region]">@region must name a region of a pdf asset.</sch:assert>
      <sch:assert id="R52" test="//layer[@id=current()/@regionLayer][@asset=/scene/assets/pdf[region/@id=current()/@region]/@id]">@regionLayer must name a layer whose asset holds the region.</sch:assert>
    </sch:rule>
    <sch:rule context="shape">
      <sch:assert id="C69" test="@width and @height">shape needs @width and @height unless it takes its box from @region.</sch:assert>
    </sch:rule>
  </sch:pattern>
  <sch:pattern id="p74">
    <sch:rule context="/scene[project/@fontPolicy='pinned']">
      <sch:assert id="C70" test="not(//@fontFile)">fontPolicy="pinned": faces come from font assets, not @fontFile.</sch:assert>
      <sch:assert id="C71" test="not(assets/font[not(@sha256)])">fontPolicy="pinned": every font asset carries @sha256.</sch:assert>
      <sch:assert id="C72" test="not(//@font[not(. = /scene/assets/font/@family)])">fontPolicy="pinned": every @font names the family of a font asset.</sch:assert>
      <sch:assert id="C73" test="not(//@fallback[str:tokenize(., ',')[not(normalize-space(.) = current()/assets/font/@family)]])">fontPolicy="pinned": every @fallback family names a font asset.</sch:assert>
    </sch:rule>
  </sch:pattern>
  <sch:pattern id="p75">
    <sch:rule context="/scene[@version='1.0' or @version='1.1']">
      <sch:assert id="V10" test="not(.//repeat/points)">points in a repeat needs version="1.2".</sch:assert>
    </sch:rule>
  </sch:pattern>
  <sch:pattern id="p76">
    <sch:rule context="repeat">
      <sch:assert id="C74" test="count(points) &lt;= 1">a repeat takes at most one points child.</sch:assert>
      <sch:assert id="C80" test="not(points and (@from[. != 0] or @step[. != 1]))">a repeat with a points child takes no @from other than 0 and no @step other than 1.</sch:assert>
    </sch:rule>
    <sch:rule context="repeat/points">
      <sch:assert id="C75" test="not(@type='along-path') or (@path and @count)">points type="along-path" needs @path and @count.</sch:assert>
      <sch:assert id="C76" test="not(@type='scatter') or (@count and ((@path and not(@width or @height)) or (not(@path) and @width and @height)))">points type="scatter" needs @count and either @path or both @width and @height.</sch:assert>
      <sch:assert id="C77" test="not(@type='vertices') or @path">points type="vertices" needs @path.</sch:assert>
      <sch:assert id="C78" test="not(@type='list') or @at">points type="list" needs @at.</sch:assert>
      <sch:assert id="C79" test="not(animate[not(@property='spacingX' or @property='spacingY' or @property='width' or @property='height')])">points animates only spacingX, spacingY, width and height.</sch:assert>
    </sch:rule>
  </sch:pattern>
  <sch:pattern id="p62">
    <sch:rule context="paints/pattern">
      <sch:assert id="R42" test="/scene/assets/image[@id=current()/@asset]">pattern/@asset must name an image asset: a pattern tiles an image.</sch:assert>
    </sch:rule>
  </sch:pattern>
  <sch:pattern id="p63">
    <sch:rule context="*[@emitterAsset]">
      <sch:assert id="R43" test="/scene/assets/image[@id=current()/@emitterAsset]">@emitterAsset must name an image asset: particles are emitted from its opaque pixels.</sch:assert>
    </sch:rule>
  </sch:pattern>
  <sch:pattern id="p64">
    <sch:rule context="effect[@source][@type='displacement-map' or @type='difference-key' or @type='shader']">
      <sch:assert id="R44" test="/scene/composition//*[@id=current()/@source] or /scene/symbols//*[@id=current()/@source]">effect @source must name a composition node; an asset is placed on a (hidden) layer, and the layer named.</sch:assert>
    </sch:rule>
  </sch:pattern>
  <sch:pattern id="p65">
    <sch:rule context="generator[@lineWidth]">
      <sch:assert id="R47" test="@kind='grid'">@lineWidth is the line width of a grid generator.</sch:assert>
    </sch:rule>
  </sch:pattern>
  <sch:pattern id="p67">
    <sch:rule context="object3D[@tracking]">
      <sch:assert id="TXT2" test="@primitive='text'">@tracking applies to object3D primitive="text".</sch:assert>
    </sch:rule>
  </sch:pattern>
  <sch:pattern id="p68">
    <sch:rule context="object3D[@materialOverride]">
      <sch:assert id="MOV1" test="count(str:tokenize(normalize-space(@materialOverride),' ')) &gt; 0 and count(str:tokenize(normalize-space(@materialOverride),' ')) = count(str:tokenize(normalize-space(@materialOverride),' ')[contains(.,':') and substring-before(.,':')!='' and substring-after(.,':')!=''])">@materialOverride is a space-separated list of name:id pairs.</sch:assert>
      <sch:assert id="MOV2" test="not(str:tokenize(normalize-space(@materialOverride),' ')[not(substring-after(., ':') = current()/ancestor::scene/materials/material/@id)])">@materialOverride: each id after the colon must name a material.</sch:assert>
    </sch:rule>
  </sch:pattern>
  <sch:pattern id="p69">
    <sch:rule context="shape[@shape='stroke-text']">
      <sch:assert id="PEN1" test="@text and @strokeFont">shape="stroke-text" needs @text and @strokeFont.</sch:assert>
      <sch:assert id="PEN2" test="not(@strokeFont) or /scene/assets/strokeFont[@id=current()/@strokeFont]">shape/@strokeFont must name a strokeFont asset.</sch:assert>
    </sch:rule>
  </sch:pattern>

  <!-- ===================================================== volumetric and large-scale cinematic effects (SREP 40, version 1.3) -->
  <sch:pattern id="cinematic-volume">
    <sch:rule context="/scene[@version!='1.3']">
      <sch:assert id="V8" test="not(assets/volume|.//medium|.//pyro|.//object3D[@primitive='volume' or @volume])">volumetric assets and media require version="1.3".</sch:assert>
    </sch:rule>
    <sch:rule context="object3D">
      <sch:assert id="VOL1" test="not(@primitive='volume') or count(@volume|pyro)=1">volume primitive requires exactly one source: @volume or a pyro child.</sch:assert>
      <sch:assert id="VOL2" test="not(@volume) or /scene/assets/volume[@id=current()/@volume]">object3D/@volume must name a volume asset.</sch:assert>
      <sch:assert id="VOL3" test="count(medium)&lt;=1 and (@primitive='volume' or (not(medium) and not(@volume) and not(pyro)))">one medium child and @volume are permitted only on a volume primitive.</sch:assert>
      <sch:assert id="VOL5" test="not(medium[@blackbody='true' or @blackbody='1']) or pyro or /scene/assets/volume[@id=current()/@volume]/@temperatureGrid">blackbody emission requires a declared temperatureGrid on the volume asset.</sch:assert>
      <sch:assert id="VOL10" test="not(medium[@lightGridCell or @lightGridDomeDirections or @lightGridMemoryMiB]) or medium/@lighting='grid'">the lightGrid* attributes of a medium apply only with lighting="grid".</sch:assert>
    </sch:rule>
    <sch:rule context="pyro">
      <sch:let name="nx" value="number(@width) div number(@voxelSize)"/>
      <sch:let name="ny" value="number(@height) div number(@voxelSize)"/>
      <sch:let name="nz" value="number(@depth) div number(@voxelSize)"/>
      <sch:let name="colliders" value="/scene//object3D[contains(concat(' ',normalize-space(current()/@colliders),' '),concat(' ',@id,' '))]"/>
      <sch:let name="count" value="count(str:tokenize(normalize-space(@colliders),' '))"/>
      <sch:assert id="PYRO1" test="$nx&gt;=2 and $nx&lt;=1024 and $ny&gt;=2 and $ny&lt;=1024 and $nz&gt;=2 and $nz&lt;=1024 and $nx - round($nx)&lt;=0.00000001 and round($nx) - $nx&lt;=0.00000001 and $ny - round($ny)&lt;=0.00000001 and round($ny) - $ny&lt;=0.00000001 and $nz - round($nz)&lt;=0.00000001 and round($nz) - $nz&lt;=0.00000001">pyro dimensions must be integer multiples of voxelSize, with 2..1024 cells per axis.</sch:assert>
      <sch:assert id="PYRO7" test="not(@colliders) or ($count &lt;= 4096 and $count = count($colliders[@primitive='box' or @primitive='sphere' or @primitive='globe' or @primitive='plane' or @primitive='cylinder' or @primitive='cone' or @primitive='capsule' or @primitive='torus' or @primitive='mesh' or @primitive='text' or @primitive='extrude' or @primitive='clay']))">pyro colliders must name at most 4096 distinct supported surface objects, including text, extrude and clay solids.</sch:assert>
      <sch:assert id="PYRO8" test="not($colliders/*[@property='primitive' or @property='mesh' or @property='radius' or @property='width' or @property='height' or @property='depth' or @property='segments' or (@property='exaggeration' and ../@primitive='globe' and ../@terrain) or (../@primitive='text' and (@property='text' or @property='font' or @property='bevel' or @property='tracking')) or (../@primitive='extrude' and (@property='path' or @property='bevel')) or (../@primitive='clay' and (@property='resolution' or @property='fingerprints' or @property='seed' or @property='boil'))]) and not($colliders[@primitive='clay']/blob/*[@property]) and not($colliders[@primitive='clay' and number(translate(@boil,'+',''))&gt;0 and number(translate(@fingerprints,'+',''))&gt;0])">pyro collider geometry is static; animate its position, rotation or scale instead of shape parameters.</sch:assert>
    </sch:rule>
    <sch:rule context="pyroSource|pyroImpulse">
      <sch:assert id="PYRO5" test="(@shape='mesh' and /scene/assets/mesh[@id=current()/@mesh]) or (not(@shape='mesh') and not(@mesh))">a mesh pyro source must name a mesh asset; @mesh is valid only for shape=mesh.</sch:assert>
      <sch:assert id="PYRO6" test="not(*[@property='shape' or @property='mesh'])">pyro shape and mesh selection are static; animate the source transform instead.</sch:assert>
      <sch:assert id="PYRO2" test="not(@end) or (@start and number(@end)&gt;number(@start)) or (not(@start) and number(@end)&gt;0)">pyro source end must be greater than start.</sch:assert>
      <sch:assert id="PYRO3" test="not(@shape='box') or (@width and @height and @depth)">box pyro sources require width, height and depth.</sch:assert>
      <sch:assert id="PYRO4" test="not(@scaleX=0 or @scaleY=0 or @scaleZ=0)">pyro source transforms must be invertible (nonzero scales).</sch:assert>
      <sch:assert id="PYC1" test="not(@crater and (@start or @end or @time or @densityRate or @temperatureRate or @density or @temperature or @expansion or @shape or @mesh or @radius or @width or @height or @depth or @x or @y or @z or @rotation or @rotationX or @rotationY or @scaleX or @scaleY or @scaleZ))">a pyro source or impulse from a crater derives its shape, place, timing, density, temperature and expansion, so none of them may be given.</sch:assert>
      <sch:assert id="PYC2" test="@crater or not(@heatFraction or @dustFraction or @specificHeat or @maxTemperature)">heatFraction, dustFraction, specificHeat and maxTemperature belong to a pyro source or impulse from a crater.</sch:assert>
      <sch:assert id="PYC3" test="not(self::pyroImpulse) or @time or @crater">a pyro impulse requires time unless it comes from a crater.</sch:assert>
      <sch:assert id="PYC4" test="not(@crater) or /scene//crater[@id=current()/@crater and @source]">a pyro source or impulse from a crater must name a crater that grows from an impact.</sch:assert>
    </sch:rule>
    <sch:rule context="assets/volume">
      <sch:let name="first" value="number(substring(normalize-space(@first),1+number(starts-with(normalize-space(@first),'+'))))"/>
      <sch:let name="last" value="number(substring(normalize-space(@last),1+number(starts-with(normalize-space(@last),'+'))))"/>
      <sch:let name="fmt" value="substring-before(substring-after(@src,'%'),'d')"/>
      <sch:let name="tail" value="substring-after(@src,'#')"/>
      <sch:let name="nonhash" value="translate($tail,'#','')"/>
      <sch:let name="pattern" value="(contains(@src,'%') and contains(substring-after(@src,'%'),'d') and translate($fmt,'0123456789','')='' and ($fmt='' or number($fmt)&lt;=64)) or (not(contains(@src,'%')) and contains(@src,'#') and (($nonhash='' and string-length($tail)&lt;64) or ($nonhash!='' and string-length(substring-before($tail,substring($nonhash,1,1)))&lt;64)))"/>
      <sch:assert id="VOL6" test="@format='srvseq' or ((@first or @last) and @first and @last and $last&gt;=$first and $last - $first&lt;1000000 and $pattern) or (not(@first or @last) and not(@fps) and (not(@interpolation) or @interpolation='hold') and (not(@missingFrame) or @missingFrame='error'))">volume sequences require first/last, 1..1000000 ordered frames and a numbered src pattern; sequence options require a sequence.</sch:assert>
      <sch:assert id="VOL9" test="(@interpolation='advect' and count(@velocityGridX|@velocityGridY|@velocityGridZ)=3) or ((not(@interpolation) or @interpolation!='advect') and not(@velocityGridX|@velocityGridY|@velocityGridZ))">advect requires all three velocity channels; velocity channels require advect interpolation.</sch:assert>
      <sch:assert id="VOL8" test="not(@format='srvseq') or (@sha256 and not(@first or @last or @fps) and (not(@missingFrame) or @missingFrame='error'))">srvseq requires sha256 and owns its frame range and composition clock; first, last, fps and missing-frame substitution are not permitted.</sch:assert>
      <sch:assert id="VOL7" test="not(@first or @last) or not(@sha256)">volume sha256 applies to a single cache file, not a numbered sequence.</sch:assert>
      <sch:assert id="VOL4" test="not(@boundsMinX|@boundsMinY|@boundsMinZ|@boundsMaxX|@boundsMaxY|@boundsMaxZ) or (count(@boundsMinX|@boundsMinY|@boundsMinZ|@boundsMaxX|@boundsMaxY|@boundsMaxZ)=6 and number(@boundsMinX)&lt;number(@boundsMaxX) and number(@boundsMinY)&lt;number(@boundsMaxY) and number(@boundsMinZ)&lt;number(@boundsMaxZ))">volume bounds require all six finite coordinates with each minimum below its maximum.</sch:assert>
    </sch:rule>
  </sch:pattern>
  <sch:pattern id="cinematic-mesh-sequence">
    <sch:rule context="assets/meshSequence">
      <sch:let name="first" value="number(substring(normalize-space(@first),1+number(starts-with(normalize-space(@first),'+'))))"/>
      <sch:let name="last" value="number(substring(normalize-space(@last),1+number(starts-with(normalize-space(@last),'+'))))"/>
      <sch:let name="fmt" value="substring-before(substring-after(@src,'%'),'d')"/>
      <sch:let name="tail" value="substring-after(@src,'#')"/>
      <sch:let name="nonhash" value="translate($tail,'#','')"/>
      <sch:let name="pattern" value="(contains(@src,'%') and contains(substring-after(@src,'%'),'d') and translate($fmt,'0123456789','')='' and ($fmt='' or number($fmt)&lt;=64)) or (not(contains(@src,'%')) and contains(@src,'#') and (($nonhash='' and string-length($tail)&lt;64) or ($nonhash!='' and string-length(substring-before($tail,substring($nonhash,1,1)))&lt;64)))"/>
      <sch:assert id="MSQ1" test="/scene/@version='1.3'">mesh sequences require version="1.3".</sch:assert>
      <sch:assert id="MSQ2" test="$last&gt;=$first and $last - $first&lt;1000000 and $pattern">mesh sequences require 1..1000000 ordered frames and a numbered src pattern.</sch:assert>
      <sch:assert id="MSQ3" test="not(@sha256)">mesh sequence sha256 cannot identify multiple numbered files.</sch:assert>
    </sch:rule>
    <sch:rule context="object3D|pyro|particles3D">
      <sch:assert id="MSQ4" test="not(self::object3D[@primitive='mesh' and rigidBody and not(fracture and rigidBody[@type='static' or @type='kinematic']) and @mesh=/scene/assets/meshSequence/@id]) and not(/scene//object3D[@primitive='mesh' and @mesh=/scene/assets/meshSequence/@id and contains(concat(' ',normalize-space(current()/@colliders),' '),concat(' ',@id,' '))])">mesh sequences are changing render geometry; rigid collider consumers require an explicit static proxy.</sch:assert>
    </sch:rule>
  </sch:pattern>
  <sch:pattern id="cinematic-particles">
    <sch:rule context="particles3D">
      <sch:let name="start" value="sum(@emissionStart)"/>
      <sch:let name="life" value="sum(@lifetime) + 2 * not(@lifetime)"/>
      <sch:let name="dy" value="sum(@directionY) - not(@directionY)"/>
      <sch:let name="colliders" value="/scene//object3D[contains(concat(' ',normalize-space(current()/@colliders),' '),concat(' ',@id,' '))]"/>
      <sch:assert id="P3D1" test="/scene/@version='1.3'">particles3D requires version="1.3".</sch:assert>
      <sch:assert id="P3D2" test="((@emitterShape='mesh' and /scene/assets/mesh[@id=current()/@emitterMesh]) or (not(@emitterShape='mesh') and not(@emitterMesh))) and ((@shape='mesh' and /scene/assets/mesh[@id=current()/@mesh]) or (not(@shape='mesh') and not(@mesh))) and (not(@sprite) or (@shape='billboard' and /scene/assets/image[@id=current()/@sprite])) and (not(@material) or /scene/materials/material[@id=current()/@material])">particles3D mesh, emitterMesh, sprite and material must name the appropriate assets and match the selected shapes.</sch:assert>
      <sch:assert id="P3D3" test="sum(@lifetimeVariance)&lt;$life and sum(@speedVariance)&lt;=sum(@speed) and (sum(@directionX)!=0 or $dy!=0 or sum(@directionZ)!=0) and (not(@emissionEnd) or number(@emissionEnd)&gt;=$start) and not(burst[not(@crater) and not(number(@time)&gt;=$start)])">particles3D requires bounded variances, a nonzero direction, an ordered emission window and bursts at or after emissionStart.</sch:assert>
      <sch:assert id="P3D4" test="not(*[@property and not(@property='x' or @property='y' or @property='z' or @property='rotation' or @property='rotationX' or @property='rotationY' or @property='scaleX' or @property='scaleY' or @property='scaleZ' or @property='rate' or @property='opacity' or @property='size' or @property='sizeEnd' or @property='color' or @property='colorEnd' or @property='opacityEnd' or @property='trail')])">particles3D solver configuration is static; animate pose, rate or appearance instead.</sch:assert>
      <sch:assert id="P3D5" test="not(@colliders) or (count(str:tokenize(normalize-space(@colliders),' '))&lt;=4096 and count(str:tokenize(normalize-space(@colliders),' '))=count($colliders[@primitive='box' or @primitive='sphere' or @primitive='globe' or @primitive='plane' or @primitive='cylinder' or @primitive='cone' or @primitive='capsule' or @primitive='torus' or @primitive='mesh' or @primitive='text' or @primitive='extrude' or @primitive='clay']))">particles3D colliders must name at most 4096 distinct supported rigid surface objects.</sch:assert>
      <sch:assert id="P3D11" test="not(@gas) or /scene//object3D[@id=current()/@gas]/pyro">particles3D gas must name an object3D that holds a native pyro volume; a baked or asset volume has no velocity grid.</sch:assert>
      <sch:assert id="P3D6" test="not($colliders/*[@property='primitive' or @property='mesh' or @property='radius' or @property='width' or @property='height' or @property='depth' or @property='segments' or (@property='exaggeration' and ../@primitive='globe' and ../@terrain) or (../@primitive='text' and (@property='text' or @property='font' or @property='bevel' or @property='tracking')) or (../@primitive='extrude' and (@property='path' or @property='bevel')) or (../@primitive='clay' and (@property='resolution' or @property='fingerprints' or @property='seed' or @property='boil'))]) and not($colliders[@primitive='clay']/blob/*[@property]) and not($colliders[@primitive='clay' and number(translate(@boil,'+',''))&gt;0 and number(translate(@fingerprints,'+',''))&gt;0])">particles3D collider geometry must be static; animate its rigid pose instead.</sch:assert>
    </sch:rule>
    <sch:rule context="particles3D/burst">
      <sch:let name="angle" value="sum(@angle) + 45 * not(@angle)"/>
      <sch:let name="spread" value="sum(@angleSpread) + 15 * not(@angleSpread)"/>
      <sch:assert id="P3D7" test="(@crater and not(@time or @repeat or @interval)) or (not(@crater) and @time)">a burst requires time unless it comes from a crater, and a burst from a crater derives its instants, so time, repeat and interval may not be given.</sch:assert>
      <sch:assert id="P3D8" test="not(@crater) or /scene//crater[@id=current()/@crater and @source]">a burst from a crater must name a crater that grows from an impact.</sch:assert>
      <sch:assert id="P3D9" test="@crater or not(@angle or @angleSpread)">angle and angleSpread belong to a burst from a crater.</sch:assert>
      <sch:assert id="P3D10" test="not(@crater) or ($angle - $spread &gt;= 0 and $angle + $spread &lt;= 90)">the launch angle of a burst from a crater and its spread must stay between 0 and 90 degrees.</sch:assert>
    </sch:rule>
  </sch:pattern>
  <sch:pattern id="cinematic-ocean">
    <sch:rule context="ocean">
      <sch:let name="cell" value="sum(@cellSize)+not(@cellSize)"/>
      <sch:let name="nx" value="(sum(@width)+64*not(@width)) div $cell"/>
      <sch:let name="nz" value="(sum(@depth)+64*not(@depth)) div $cell"/>
      <sch:let name="colliders" value="/scene//object3D[contains(concat(' ',normalize-space(current()/@colliders),' '),concat(' ',@id,' '))]"/>
      <sch:let name="count" value="count(str:tokenize(normalize-space(@colliders),' '))"/>
      <sch:assert id="OCN1" test="/scene/@version='1.3'">ocean requires version="1.3".</sch:assert>
      <sch:assert id="OCN2" test="(not(@bathymetry) or /scene/assets/image[@id=current()/@bathymetry and not(@layer)] or /scene/assets/mesh[@id=current()/@bathymetry]) and (not(@material) or /scene/materials/material[@id=current()/@material]) and (@bathymetry or not(@bathymetryScale|@bathymetryOffset|@bathymetryEncoding)) and (not(@bathymetryEncoding) or @bathymetryEncoding='red' or /scene/assets/image[@id=current()/@bathymetry])">ocean bathymetry must name a primary-raster image (no layer selection) or mesh, material must name a material, and bathymetry options require the corresponding asset kind.</sch:assert>
      <sch:assert id="OCN3" test="$nx&gt;=1 and $nz&gt;=1 and $nx - round($nx)&lt;=0.000000001*$nx and round($nx)-$nx&lt;=0.000000001*$nx and $nz - round($nz)&lt;=0.000000001*$nz and round($nz)-$nz&lt;=0.000000001*$nz and round($nx)*round($nz)&lt;=4000000 and (not(@dt) or number(@dt)&gt;=0.000001) and not((@waterLevel|@bathymetryScale|@bathymetryOffset|@initialVelocityX|@initialVelocityZ|wave/@direction|wave/@phase|waterImpulse/@x|waterImpulse/@z|waterImpulse/@amplitude|waterImpulse/@velocityX|waterImpulse/@velocityZ)[not(number(.)-number(.)=0)]) and count(wave)&lt;=64 and count(waterImpulse)&lt;=16384 and not(wave[(sum(@wavelength)+16*not(@wavelength))&lt;2*$cell]) and not(waterImpulse[@type='add-water' and @amplitude&lt;0])">ocean requires finite inputs, integral grid dimensions (at most 4000000 cells), dt &gt;= 0.000001, resolved wavelengths, at most 64 waves/16384 impulses and nonnegative added water.</sch:assert>
      <sch:assert id="OCN4" test="not(*[@property and not(@property='x' or @property='y' or @property='z' or @property='rotation' or @property='rotationX' or @property='rotationY' or @property='scaleX' or @property='scaleY' or @property='scaleZ' or @property='opacity')])">ocean solver configuration is static; animate its pose or opacity instead.</sch:assert>
      <sch:assert id="OCN5" test="count(whitewater)&lt;=1 and not(whitewater[@end and @end &lt; sum(@start)]) and not(whitewater/@foamMaterial[not(.=/scene/materials/material/@id)]|whitewater/@sprayMaterial[not(.=/scene/materials/material/@id)])">ocean accepts one whitewater source with end &gt;= start and valid foam/spray materials.</sch:assert>
      <sch:assert id="OCN6" test="not(@colliders) or ($count &lt;= 4096 and $count = count($colliders[(crater and (@primitive='plane' or @primitive='mesh')) or (not(crater) and (@primitive='box' or @primitive='sphere' or @primitive='globe' or @primitive='cylinder' or @primitive='cone' or @primitive='capsule' or @primitive='torus' or @primitive='mesh' or @primitive='text' or @primitive='extrude' or @primitive='clay'))]))">ocean colliders must name at most 4096 distinct objects: a plane or mesh with a crater, or a closed body without one.</sch:assert>
      <sch:assert id="OCN7" test="not($colliders/*[@property='primitive' or @property='mesh' or @property='radius' or @property='width' or @property='height' or @property='depth' or @property='segments' or (@property='exaggeration' and ../@primitive='globe' and ../@terrain) or (../@primitive='text' and (@property='text' or @property='font' or @property='bevel')) or (../@primitive='extrude' and (@property='path' or @property='bevel')) or (../@primitive='clay' and (@property='resolution' or @property='fingerprints' or @property='seed' or @property='boil'))]) and not($colliders[@primitive='clay']/blob/*[@property]) and not($colliders[@primitive='clay' and number(translate(@boil,'+',''))&gt;0 and number(translate(@fingerprints,'+',''))&gt;0])">ocean collider geometry is static; animate its position, rotation or scale instead of shape parameters.</sch:assert>
      <sch:assert id="OCN13" test="not(@splash) or (count(str:tokenize(normalize-space(@splash),' '))&gt;0 and count(str:tokenize(normalize-space(@splash),' '))=count(/scene//particles3D[burst[@crater] and contains(concat(' ',normalize-space(current()/@splash),' '),concat(' ',@id,' '))]))">ocean splash must name distinct particles3D emitters that throw out the ejecta of a crater, whose particles have a mass.</sch:assert>
      <sch:assert id="OCN8" test="not(@bodyCoupling) or @bodyCoupling='none' or @colliders">ocean bodyCoupling needs colliders that list the bodies.</sch:assert>
      <sch:assert id="OCN9" test="not(@bodyDrag) or @colliders">ocean bodyDrag belongs to an ocean with colliders.</sch:assert>
    </sch:rule>
    <sch:rule context="ocean/waterImpulse">
      <sch:assert id="OCN10" test="not(@source and (@time or @x or @z or @radius or @amplitude or @velocityX or @velocityZ or @type))">a water impulse from a body derives its instant, place, size and shape, so time, x, z, radius, amplitude, velocityX, velocityZ and type may not be given.</sch:assert>
      <sch:assert id="OCN11" test="not(@source) or (contains(concat(' ',normalize-space(../@colliders),' '),concat(' ',@source,' ')) and /scene//object3D[@id=current()/@source and not(crater)]/rigidBody[not(@type) or @type='dynamic'])">the source of a water impulse must be a dynamic rigid body, without a crater, that the ocean lists in colliders.</sch:assert>
      <sch:assert id="OCN12" test="not(@source) or count(../waterImpulse[@source=current()/@source])=1">a body makes one cavity: at most one water impulse names it.</sch:assert>
    </sch:rule>
  </sch:pattern>
  <sch:pattern id="cinematic-crater">
    <sch:rule context="object3D/crater">
      <sch:let name="radius" value="number(concat(substring(normalize-space(@radius),1+number(starts-with(normalize-space(@radius),'+'))),substring('50',1,2*number(not(@radius)))))"/>
      <sch:let name="depth" value="number(concat(substring(normalize-space(@depth),1+number(starts-with(normalize-space(@depth),'+'))),substring('10',1,2*number(not(@depth)))))"/>
      <sch:let name="rimWidth" value="number(concat(substring(normalize-space(@rimWidth),1+number(starts-with(normalize-space(@rimWidth),'+'))),substring('10',1,2*number(not(@rimWidth)))))"/>
      <sch:let name="rimHeight" value="number(concat(substring(normalize-space(@rimHeight),1+number(starts-with(normalize-space(@rimHeight),'+'))),substring('2',1,1*number(not(@rimHeight)))))"/>
      <sch:let name="influenceDepth" value="number(concat(substring(normalize-space(@influenceDepth),1+number(starts-with(normalize-space(@influenceDepth),'+'))),substring('0',1,1*number(not(@influenceDepth)))))"/>
      <sch:let name="normalX" value="number(concat(substring(normalize-space(@normalX),1+number(starts-with(normalize-space(@normalX),'+'))),substring('0',1,1*number(not(@normalX)))))"/>
      <sch:let name="normalY" value="number(concat(substring(normalize-space(@normalY),1+number(starts-with(normalize-space(@normalY),'+'))),substring('0',1,1*number(not(@normalY)))))"/>
      <sch:let name="normalZ" value="number(concat(substring(normalize-space(@normalZ),1+number(starts-with(normalize-space(@normalZ),'+'))),substring('-1',1,2*number(not(@normalZ)))))"/>
      <sch:let name="start" value="number(concat(substring(normalize-space(@start),1+number(starts-with(normalize-space(@start),'+'))),substring('0',1,1*number(not(@start)))))"/>
      <sch:let name="end" value="number(concat(substring(normalize-space(@end),1+number(starts-with(normalize-space(@end),'+'))),substring('1',1,1*number(not(@end)))))"/>
      <sch:assert id="CRT1" test="/scene/@version='1.3'">crater deformation requires version="1.3".</sch:assert>
      <sch:assert id="CRT2" test="parent::object3D[not(@primitive='volume')] and count(../crater)=1">one crater belongs to a surface object3D, not a volume.</sch:assert>
      <sch:assert id="CRT3" test="$end &gt; $start">crater end must be greater than start in the object's local clock.</sch:assert>
      <sch:assert id="CRT4" test="not(@*[name()!='curve' and name()!='id' and name()!='source' and name()!='capture' and name()!='targetMaterial' and not((number(substring(normalize-space(.),1+number(starts-with(normalize-space(.),'+')))) - number(substring(normalize-space(.),1+number(starts-with(normalize-space(.),'+')))))=0)]) and ($normalX!=0 or $normalY!=0 or $normalZ!=0) and $rimWidth&lt;=$radius and (not(@influenceDepth) or ($influenceDepth*0.5 &gt;= $depth and $influenceDepth*0.5 &gt;= $rimHeight))">crater values must be finite, its normal nonzero, rimWidth &lt;= radius and influenceDepth &gt;= twice max(depth,rimHeight).</sch:assert>
      <sch:assert id="CRT6" test="not(@source and (@radius or @depth or @rimHeight or @rimWidth or @start or @end or @centerX or @centerY or @centerZ or @normalX or @normalY or @normalZ))">a crater that grows from a source derives its size, timing, centre and axis, so none of them may be given.</sch:assert>
      <sch:assert id="CRT7" test="(@source and @targetMaterial) or not(@source or @targetMaterial or @targetDensity or @strength or @gravity)">crater source requires targetMaterial, and targetMaterial, targetDensity, strength and gravity belong to a crater with a source.</sch:assert>
      <sch:assert id="CRT8" test="not(@source) or (/scene//object3D[@id=current()/@source and not(@id=current()/../@id)]/rigidBody[not(@type) or @type='dynamic'])">crater source must name another object3D whose rigidBody is dynamic.</sch:assert>
      <sch:assert id="CRT9" test="not(@capture) or @source">crater capture belongs to a crater that grows from a source.</sch:assert>
      <sch:assert id="CRT5" test="not(../rigidBody[not(@type='static' or @type='kinematic') or (@shape and not(@shape='auto' or @shape='trimesh'))])">crater rigid bodies require static/kinematic type with auto or trimesh collision geometry.</sch:assert>
    </sch:rule>
  </sch:pattern>
  <sch:pattern id="cinematic-fracture">
    <sch:rule context="fracture">
      <sch:assert id="FRX1" test="/scene/@version='1.3'">fracture requires version="1.3".</sch:assert>
      <sch:assert id="FRX2" test="parent::object3D[not(@primitive='volume' or @primitive='plane' or @primitive='map')] and count(../fracture)=1 and count(../rigidBody)=1">fracture requires one closed surface object3D owner and exactly one rigidBody.</sch:assert>
      <sch:assert id="FRX3" test="@interiorMaterial=/scene/materials/material/@id">fracture interiorMaterial must reference a declared material.</sch:assert>
      <sch:assert id="FRX4" test="not(@*[name()!='interiorMaterial' and not((number(substring(normalize-space(.),1+number(starts-with(normalize-space(.),'+')))) - number(substring(normalize-space(.),1+number(starts-with(normalize-space(.),'+')))))=0)])">fracture numeric values must be finite.</sch:assert>
    </sch:rule>
  </sch:pattern>
  <sch:pattern id="cinematic-globe">
    <sch:rule context="object3D">
      <sch:assert id="GEO1" test="/scene/@version='1.3' or not((@terrain and @primitive='globe') or @planetRadius or @terrainTileSize or @terrainZoom or @terrainMissing or @terrainMemoryMiB)">globe elevation and its physical-scale/budget options require version="1.3".</sch:assert>
      <sch:assert id="GEO2" test="(not(@planetRadius) or @primitive='globe') and (not(@terrainTileSize|@terrainZoom|@terrainMissing|@terrainMemoryMiB) or (@primitive='globe' and @terrain)) and (not(@terrainTileSize) or @terrainTileSize=1 or @terrainTileSize=2 or @terrainTileSize=4 or @terrainTileSize=8 or @terrainTileSize=16 or @terrainTileSize=32 or @terrainTileSize=64 or @terrainTileSize=128 or @terrainTileSize=256 or @terrainTileSize=512 or @terrainTileSize=1024 or @terrainTileSize=2048 or @terrainTileSize=4096)">planetRadius belongs to globes; terrain sampling options require a globe with terrain and a power-of-two tile size.</sch:assert>
      <sch:assert id="GEO3" test="not(@primitive='globe' and *[@property='terrain' or @property='terrainEncoding' or @property='planetRadius' or @property='terrainTileSize' or @property='terrainZoom' or @property='terrainMissing' or @property='terrainMemoryMiB']) and not(@primitive='globe' and @terrain and rigidBody and *[@property='radius' or @property='segments' or @property='exaggeration'])">globe terrain sampling and planetRadius are static; globe rigid collider relief cannot animate radius, segments or exaggeration.</sch:assert>
    </sch:rule>
  </sch:pattern>
</sch:schema>
