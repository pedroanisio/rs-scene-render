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
