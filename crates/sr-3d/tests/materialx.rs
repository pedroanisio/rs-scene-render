use sr_3d::mtlx;
use std::path::Path;

#[test]
fn graph_outputs_and_arithmetic_drive_surface_inputs() {
    let xml = r#"<materialx><nodegraph name="NG">
      <constant name="colour" type="color3"><input name="value" type="color3" value="0.2, 0.4, 0.8"/></constant>
      <multiply name="tint" type="color3"><input name="in1" nodename="colour"/><input name="in2" type="float" value="0.5"/></multiply>
      <output name="out" type="color3" nodename="tint"/>
    </nodegraph><standard_surface name="surface"><input name="base_color" type="color3" nodegraph="NG" output="out"/></standard_surface></materialx>"#;
    let material = mtlx::parse(xml, Path::new(".")).unwrap();
    assert_eq!(&material.params.base_color[..3], &[0.1, 0.2, 0.4]);
    assert!(material.warnings.is_empty(), "{:?}", material.warnings);
}

#[test]
fn graph_cycles_and_missing_connections_are_errors() {
    for body in [
        r#"<add name="a" type="float"><input name="in1" nodename="a"/><input name="in2" value="1"/></add>"#,
        r#"<add name="a" type="float"><input name="in1" nodename="missing"/><input name="in2" value="1"/></add>"#,
    ] {
        let xml = format!(
            r#"<materialx>{body}<standard_surface name="s"><input name="specular_roughness" nodename="a"/></standard_surface></materialx>"#
        );
        assert!(mtlx::parse(&xml, Path::new(".")).is_err(), "{body}");
    }
}

#[test]
fn image_graphs_bake_linear_pixels_and_preserve_hdr_factors() {
    let dir = std::env::temp_dir().join(format!("sr-materialx-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    image::RgbaImage::from_raw(2, 1, vec![255, 0, 0, 255, 0, 255, 0, 255])
        .unwrap()
        .save(dir.join("colour.png"))
        .unwrap();
    let xml = r#"<materialx><nodegraph name="NG"><image name="im" type="color3" colorspace="lin_rec709"><input name="file" type="filename" value="colour.png"/></image><multiply name="bright" type="color3"><input name="in1" nodename="im"/><input name="in2" type="float" value="2"/></multiply><output name="out" nodename="bright"/></nodegraph><standard_surface name="s"><input name="base_color" nodegraph="NG" output="out"/></standard_surface></materialx>"#;
    let material = mtlx::parse(xml, &dir).unwrap();
    assert!(material.warnings.is_empty());
    let texture = material.generated_maps[0].as_ref().expect("graph texture");
    assert_eq!((texture.width, texture.height), (2, 1));
    assert!(!texture.srgb);
    assert_eq!(&material.params.base_color[..3], &[2.0, 2.0, 1.0]);
    assert_eq!(&texture.rgba[..8], &[255, 0, 0, 255, 0, 255, 0, 255]);
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn normal_graphs_decode_vectors_and_apply_strength() {
    let xml = r#"<materialx><constant name="encoded" type="color3"><input name="value" type="color3" value="1,0.5,1"/></constant><normalmap name="n" type="vector3"><input name="in" nodename="encoded"/><input name="scale" type="float" value="0"/></normalmap><standard_surface name="s"><input name="normal" nodename="n"/></standard_surface></materialx>"#;
    let material = mtlx::parse(xml, Path::new(".")).unwrap();
    let map = material.generated_maps[1].as_ref().expect("normal graph must become a map");
    assert_eq!(&map.rgba[..3], &[128, 128, 255]);
}

#[test]
fn connected_maps_preserve_surface_weights_and_emission() {
    let dir = std::env::temp_dir().join(format!("sr-materialx-weights-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    image::RgbaImage::from_pixel(1, 1, image::Rgba([255, 255, 255, 255])).save(dir.join("white.png")).unwrap();
    let material = mtlx::parse(r#"<materialx>
      <image name="im" type="color3" colorspace="lin_rec709"><input name="file" type="filename" value="white.png"/></image>
      <multiply name="scaled" type="color3"><input name="in1" nodename="im"/><input name="in2" value="2"/></multiply>
      <standard_surface name="s"><input name="base" value="0.25"/><input name="base_color" nodename="scaled"/>
        <input name="emission" value="3"/><input name="emission_color" nodename="im"/>
        <input name="specular_roughness" nodename="im"/>
      </standard_surface></materialx>"#, &dir).unwrap();
    assert_eq!(&material.params.base_color[..3], &[0.5; 3]);
    assert_eq!(material.params.roughness, 1.0);
    assert_eq!(material.params.emissive, [3.0; 3]);
    assert!(material.generated_maps[4].is_some());
    assert!(material.warnings.is_empty());
    std::fs::remove_dir_all(dir).unwrap();
}

fn roughness_graph(body: &str) -> f32 {
    let xml = format!(
        r#"<materialx>{body}<standard_surface name="s"><input name="specular_roughness" nodename="result"/></standard_surface></materialx>"#
    );
    mtlx::parse(&xml, Path::new(".")).unwrap().params.roughness
}

#[test]
fn arithmetic_uses_operator_defaults() {
    for op in ["multiply", "divide", "power"] {
        let value =
            roughness_graph(&format!(r#"<{op} name="result" type="float"><input name="in1" value="0.4"/></{op}>"#));
        assert!((value - 0.4).abs() < 1e-6, "{op}: {value}");
    }
}

#[test]
fn vector_operations_use_all_declared_components() {
    let value = roughness_graph(
        r#"<constant name="v" type="vector4"><input name="value" type="vector4" value="0,0,0,0.5"/></constant><magnitude name="result" type="float"><input name="in" nodename="v"/></magnitude>"#,
    );
    assert_eq!(value, 0.5);
    let value = roughness_graph(
        r#"<dotproduct name="result" type="float"><input name="in1" type="vector4" value="0,0,0,0.5"/><input name="in2" type="vector4" value="0,0,0,0.5"/></dotproduct>"#,
    );
    assert_eq!(value, 0.25);
    let value = roughness_graph(
        r#"<normalize name="n" type="vector4"><input name="in" type="vector4" value="0,0,0,2"/></normalize><extract name="result" type="float"><input name="in" nodename="n"/><input name="index" value="3"/></extract>"#,
    );
    assert_eq!(value, 1.0);
}

#[test]
fn remap_preserves_descending_ranges_and_handles_zero_span() {
    for (high, expected) in [("0", 0.75), ("1", 0.0)] {
        let value = roughness_graph(&format!(
            r#"<remap name="result" type="float"><input name="in" value="0.25"/><input name="inlow" value="1"/><input name="inhigh" value="{high}"/></remap>"#
        ));
        assert_eq!(value, expected);
    }
}

#[test]
fn image_graphs_honor_address_filter_and_tiling_controls() {
    let dir = std::env::temp_dir().join(format!("sr-materialx-sampling-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    image::RgbaImage::from_raw(2, 1, vec![255, 0, 0, 255, 0, 255, 0, 255]).unwrap().save(dir.join("rg.png")).unwrap();
    for (op, inputs, expected) in [
        ("tiledimage", r#"<input name="uvoffset" type="vector2" value="0.5,0"/>"#, [0, 255, 0]),
        ("tiledimage", r#"<input name="uvtiling" type="vector2" value="3,1"/>"#, [0, 255, 0]),
        (
            "image",
            r#"<input name="texcoord" type="vector2" value="1.25,0.5"/><input name="uaddressmode" type="string" value="clamp"/>"#,
            [0, 255, 0],
        ),
        (
            "image",
            r#"<input name="texcoord" type="vector2" value="1.25,0.5"/><input name="uaddressmode" type="string" value="mirror"/>"#,
            [0, 255, 0],
        ),
        (
            "image",
            r#"<input name="texcoord" type="vector2" value="1.25,0.5"/><input name="uaddressmode" type="string" value="constant"/><input name="default" type="color3" value="0,0,1"/>"#,
            [0, 0, 255],
        ),
        (
            "image",
            r#"<input name="texcoord" type="vector2" value="0.4,0.5"/><input name="filtertype" type="string" value="closest"/>"#,
            [255, 0, 0],
        ),
    ] {
        let xml = format!(
            r#"<materialx><{op} name="im" type="color3" colorspace="lin_rec709"><input name="file" type="filename" value="rg.png"/>{inputs}</{op}><standard_surface name="s"><input name="base_color" nodename="im"/></standard_surface></materialx>"#
        );
        let material = mtlx::parse(&xml, &dir).unwrap();
        let map = material.generated_maps[0].as_ref().unwrap();
        assert_eq!(&map.rgba[..3], &expected, "{op}: {inputs}");
    }
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn range_applies_gamma_and_boolean_clamping() {
    assert_eq!(
        roughness_graph(
            r#"<range name="result" type="float"><input name="in" value="0"/><input name="gamma" value="0"/></range>"#
        ),
        0.0
    );
    assert_eq!(
        roughness_graph(
            r#"<range name="result" type="float"><input name="in" value="0.25"/><input name="gamma" value="2"/></range>"#
        ),
        0.5
    );
    assert_eq!(
        roughness_graph(
            r#"<range name="result" type="float"><input name="in" value="2"/><input name="doclamp" type="boolean" value="true"/></range>"#
        ),
        1.0
    );
}

#[test]
fn image_sampler_filters_borders_and_rejects_unknown_modes() {
    let dir = std::env::temp_dir().join(format!("sr-materialx-borders-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    image::RgbaImage::from_raw(2, 1, vec![255, 0, 0, 255, 0, 255, 0, 255]).unwrap().save(dir.join("rg.png")).unwrap();
    for (uv, mode, filter, expected) in [
        ("0.375,0.5", "periodic", "linear", [191, 64, 0]),
        ("0.375,0.5", "periodic", "cubic", [215, 40, 0]),
        ("-0.25,0.5", "periodic", "closest", [0, 255, 0]),
        ("-0.25,0.5", "mirror", "closest", [255, 0, 0]),
        ("0,0.5", "clamp", "linear", [255, 0, 0]),
        ("0.25,1.5", "constant", "closest", [0, 0, 0]),
    ] {
        let xml = format!(
            r#"<materialx><image name="im" type="color3" colorspace="lin_rec709"><input name="file" type="filename" value="rg.png"/><input name="texcoord" type="vector2" value="{uv}"/><input name="uaddressmode" type="string" value="{mode}"/><input name="vaddressmode" type="string" value="{mode}"/><input name="filtertype" type="string" value="{filter}"/></image><standard_surface name="s"><input name="base_color" nodename="im"/></standard_surface></materialx>"#
        );
        let material = mtlx::parse(&xml, &dir).unwrap();
        assert_eq!(&material.generated_maps[0].as_ref().unwrap().rgba[..3], &expected, "{uv} {mode} {filter}");
        assert!(mtlx::parse(&xml.replace(&format!("value=\"{filter}\""), "value=\"unknown\""), &dir).is_err());
    }
    std::fs::remove_dir_all(dir).unwrap();
}
