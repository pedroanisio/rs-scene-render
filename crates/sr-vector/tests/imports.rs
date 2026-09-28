use sr_vector::geom::p;
use sr_vector::lottie::Lottie;
use sr_vector::scene::{Cmd, Paint, Scene};
use sr_vector::track::{self, TrackData};
use sr_vector::{svg, tile};

fn render(scene: &Scene, w: u32, h: u32) -> Vec<[f32; 4]> {
    let e = tile::encode(scene, [w, h]);
    let paints = e.paints.clone();
    tile::render_cpu(&e, &move |i, _, _| match &paints[i as usize] {
        Paint::Solid { rgba, .. } => rgba.map(|v| v as f32),
        _ => [1.0, 0.0, 1.0, 1.0],
    })
}

#[test]
fn svg_import() {
    let doc = br##"<svg xmlns="http://www.w3.org/2000/svg" width="100" height="50" viewBox="0 0 200 100">
      <defs><linearGradient id="g" gradientUnits="userSpaceOnUse" x1="0" y1="0" x2="200" y2="0"><stop offset="0" stop-color="#f00"/><stop offset="1" stop-color="#00f"/></linearGradient>
      <clipPath id="c"><rect x="0" y="0" width="100" height="100"/></clipPath></defs>
      <rect x="0" y="0" width="200" height="100" fill="#00ff00"/>
      <g clip-path="url(#c)" opacity="0.5"><rect x="0" y="0" width="200" height="100" fill="#ff0000"/></g>
      <path d="M10 10 L190 10" stroke="url(#g)" stroke-width="4" stroke-dasharray="10 5"/>
    </svg>"##;
    let s = svg::load(doc, 0.1).unwrap();
    assert_eq!(s.size, [100.0, 50.0]);
    assert!(s.scene.cmds.iter().any(|c| matches!(c, Cmd::Push { mask_init } if *mask_init == 0.0)));
    assert!(s.scene.cmds.iter().any(|c| matches!(c, Cmd::Fill { paint: Paint::Gradient(_), .. })));
    let px = render(&s.scene, 100, 50);
    // left half: red at 50 % over green; right half: green (the viewBox maps 200 units to 100 px)
    let l = px[30 * 100 + 20];
    let r = px[30 * 100 + 80];
    assert!((l[0] - 0.5).abs() < 1e-3 && (l[1] - 0.5).abs() < 1e-3, "{l:?}");
    assert!((r[1] - 1.0).abs() < 1e-3 && r[0] < 1e-3, "{r:?}");
    assert!(svg::load(b"<svg", 0.1).is_err());
    // a luminance mask: white keeps, black removes
    let masked = br##"<svg xmlns="http://www.w3.org/2000/svg" width="40" height="20" viewBox="0 0 80 40">
      <mask id="m"><rect x="0" y="0" width="40" height="40" fill="#fff"/></mask>
      <rect x="0" y="0" width="80" height="40" fill="#0000ff" mask="url(#m)"/></svg>"##;
    let ms = svg::load(masked, 0.1).unwrap();
    let px = render(&ms.scene, 40, 20);
    assert!(px[10 * 40 + 5][2] > 0.99, "{:?}", px[10 * 40 + 5]);
    assert!(px[10 * 40 + 35][3] < 0.01, "{:?}", px[10 * 40 + 35]);
}

const LOTTIE: &str = r##"{
 "v":"5.7.0","fr":30,"ip":0,"op":60,"w":100,"h":100,
 "markers":[{"cm":"intro","tm":10,"dr":20}],
 "slots":{"fillColor":{"p":{"a":0,"k":[0,0,1,1]}}},
 "layers":[
  {"ty":4,"ind":2,"parent":1,"ip":0,"op":60,"st":0,
   "ks":{"p":{"a":0,"k":[0,0]},"a":{"a":0,"k":[0,0]},"s":{"a":0,"k":[100,100]},"r":{"a":0,"k":0},"o":{"a":0,"k":100}},
   "shapes":[{"ty":"gr","it":[
      {"ty":"rc","p":{"a":0,"k":[10,10]},"s":{"a":0,"k":[20,20]},"r":{"a":0,"k":0}},
      {"ty":"fl","c":{"sid":"fillColor","a":0,"k":[1,0,0,1]},"o":{"a":0,"k":100}},
      {"ty":"tr","p":{"a":0,"k":[0,0]},"a":{"a":0,"k":[0,0]},"s":{"a":0,"k":[100,100]},"r":{"a":0,"k":0},"o":{"a":0,"k":100}}]}]},
  {"ty":3,"ind":1,"ip":0,"op":60,
   "ks":{"p":{"a":1,"k":[{"t":0,"s":[0,0],"o":{"x":[0],"y":[0]},"i":{"x":[1],"y":[1]}},{"t":30,"s":[60,0]}]},"a":{"a":0,"k":[0,0]},"s":{"a":0,"k":[100,100]},"r":{"a":0,"k":0},"o":{"a":0,"k":100}}},
  {"ty":4,"ind":3,"ip":0,"op":60,"td":1,
   "ks":{"p":{"a":0,"k":[0,0]},"a":{"a":0,"k":[0,0]},"s":{"a":0,"k":[100,100]},"r":{"a":0,"k":0},"o":{"a":0,"k":100}},
   "shapes":[{"ty":"rc","p":{"a":0,"k":[50,80]},"s":{"a":0,"k":[100,20]},"r":{"a":0,"k":0}},{"ty":"fl","c":{"a":0,"k":[1,1,1,1]},"o":{"a":0,"k":100}}]},
  {"ty":1,"ind":4,"ip":0,"op":60,"tt":1,"sc":"#00ff00","sw":100,"sh":100,
   "ks":{"p":{"a":0,"k":[0,0]},"a":{"a":0,"k":[0,0]},"s":{"a":0,"k":[100,100]},"r":{"a":0,"k":0},"o":{"a":0,"k":100}},
   "hasMask":true,"masksProperties":[{"mode":"a","inv":false,"o":{"a":0,"k":100},
     "pt":{"a":0,"k":{"c":true,"v":[[0,70],[50,70],[50,100],[0,100]],"i":[[0,0],[0,0],[0,0],[0,0]],"o":[[0,0],[0,0],[0,0],[0,0]]}}}]},
  {"ty":4,"ind":5,"ip":0,"op":60,
   "ks":{"p":{"a":0,"k":[0,0]},"a":{"a":0,"k":[0,0]},"s":{"a":0,"k":[100,100]},"r":{"a":0,"k":0},"o":{"a":0,"k":100}},
   "shapes":[{"ty":"sh","ks":{"a":0,"k":{"c":false,"v":[[0,50],[100,50]],"i":[[0,0],[0,0]],"o":[[0,0],[0,0]]}}},
     {"ty":"st","c":{"a":0,"k":[1,1,1,1]},"o":{"a":0,"k":100},"w":{"a":0,"k":2},"lc":1,"lj":1},
     {"ty":"tm","s":{"a":0,"k":0},"e":{"a":1,"k":[{"t":0,"s":[0],"o":{"x":[0],"y":[0]},"i":{"x":[1],"y":[1]}},{"t":60,"s":[100]}]},"o":{"a":0,"k":0},"m":1}]}
 ]}"##;

#[test]
fn lottie_evaluation() {
    let l = Lottie::parse(LOTTIE.as_bytes(), None, &[]).unwrap();
    assert_eq!(l.size, [100.0, 100.0]);
    assert_eq!(l.segment("intro"), Some((10.0, 30.0)));
    assert_eq!(l.segment("5, 15"), Some((5.0, 15.0)));
    assert!((l.frame_at(0.5, Some((10.0, 30.0))) - 25.0).abs() < 1e-9);
    // frame 15: the null parent has moved 30 px; the slot colour (blue) fills the square
    let px = render(&l.render(15.0, 0.1), 100, 100);
    let at = |x: usize, y: usize| px[y * 100 + x];
    assert!(at(40, 10)[2] > 0.99 && at(40, 10)[0] < 0.01, "{:?}", at(40, 10));
    assert!(at(5, 10)[3] < 0.01);
    // the solid shows only where both its mask (x < 50, y ≥ 70) and its alpha matte (y ≥ 70) are
    assert!(at(20, 85)[1] > 0.99);
    assert!(at(70, 85)[3] < 0.01);
    assert!(at(20, 60)[1] < 0.01);
    // trim: at frame 30 half of the stroked line is drawn
    let px30 = render(&l.render(30.0, 0.1), 100, 100);
    assert!(px30[50 * 100 + 25][3] > 0.99 && px30[50 * 100 + 75][3] < 0.01);
    // overrides replace the slot
    let o = Lottie::parse(LOTTIE.as_bytes(), None, &[("fillColor".into(), "#FF0000".into())]).unwrap();
    let po = render(&o.render(15.0, 0.1), 100, 100);
    assert!(po[10 * 100 + 40][0] > 0.99);
    assert!(Lottie::parse(b"{}", None, &[]).is_err());
}

fn zip_stored(files: &[(&str, &[u8])]) -> Vec<u8> {
    let mut out = Vec::new();
    let mut central = Vec::new();
    for (name, data) in files {
        let off = out.len() as u32;
        let hdr = |sig: u32, out: &mut Vec<u8>| {
            out.extend(sig.to_le_bytes());
        };
        hdr(0x0403_4b50, &mut out);
        out.extend([20, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
        out.extend([0u8; 4]);
        out.extend((data.len() as u32).to_le_bytes());
        out.extend((data.len() as u32).to_le_bytes());
        out.extend((name.len() as u16).to_le_bytes());
        out.extend([0, 0]);
        out.extend(name.as_bytes());
        out.extend(*data);
        hdr(0x0201_4b50, &mut central);
        central.extend([20, 0, 20, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
        central.extend([0u8; 4]);
        central.extend((data.len() as u32).to_le_bytes());
        central.extend((data.len() as u32).to_le_bytes());
        central.extend((name.len() as u16).to_le_bytes());
        central.extend([0u8; 12]);
        central.extend(off.to_le_bytes());
        central.extend(name.as_bytes());
    }
    let cd = out.len() as u32;
    out.extend(&central);
    out.extend(0x0605_4b50u32.to_le_bytes());
    out.extend([0, 0, 0, 0]);
    out.extend((files.len() as u16).to_le_bytes());
    out.extend((files.len() as u16).to_le_bytes());
    out.extend((central.len() as u32).to_le_bytes());
    out.extend(cd.to_le_bytes());
    out.extend([0, 0]);
    out
}

#[test]
fn dotlottie_container() {
    let z = zip_stored(&[("manifest.json", b"{}"), ("animations/wave.json", LOTTIE.as_bytes())]);
    let l = Lottie::parse(&z, Some("wave"), &[]).unwrap();
    assert_eq!(l.fps, 30.0);
    assert!(Lottie::parse(&z, Some("missing"), &[]).unwrap_err().contains("missing"));
    assert!(Lottie::parse(&z, None, &[]).is_ok());
}

#[test]
fn tracking_formats() {
    let j = TrackData::parse(
        r#"{"fps":25,"points":{"eye":[[0,10,20],[25,20,40]]},"planes":{"sign":[[0,0,0,10,0,10,10,0,10]]}}"#,
        "json",
    )
    .unwrap();
    assert_eq!(j.point("eye", 0.5), Some(p(15.0, 30.0)));
    assert_eq!(j.point("sign", 0.0), Some(p(5.0, 5.0)));
    let c = TrackData::parse("frame,track,x,y\n0,a,0,0\n10,a,10,5\n", "csv").unwrap();
    assert_eq!(c.point("a", 5.0 / 24.0), Some(p(5.0, 2.5)));
    let cp = TrackData::parse("frame,x0,y0,x1,y1,x2,y2,x3,y3\n1,0,0,4,0,4,4,0,4\n", "csv").unwrap();
    assert_eq!(cp.plane("track", 0.0).unwrap()[2], p(4.0, 4.0));
    assert!(TrackData::parse("frame,x\n1,2\n", "csv").is_err());
    let nk = TrackData::parse("Tracker4 {\n track1 {{curve x1 10 11 12} {curve x1 20 21 22}}\n}\n", "nuke").unwrap();
    assert_eq!(nk.point("track1", 1.0 / 24.0), Some(p(10.0, 20.0)));
    assert_eq!(nk.point("track1", 3.0 / 24.0), Some(p(12.0, 22.0)));
    let chan = TrackData::parse("1 0 0 10 0 0 0 40\n2 1 0 10 0 5 0 40\n", "nuke").unwrap();
    assert_eq!(chan.camera("camera", 2.0 / 24.0).unwrap()[4], 5.0);
    let ae = "Adobe After Effects 8.0 Keyframe Data\n\n\tUnits Per Second\t30\n\tSource Width\t1920\n\nMotion Trackers\tTracker #1\tTrack Point #1\tFeature Center\n\tFrame\tX pixels\tY pixels\t\n\t0\t100\t200\t\n\t30\t130\t260\t\n\nEnd of Keyframe Data\n";
    let a = TrackData::parse(ae, "after-effects").unwrap();
    assert_eq!(a.fps, 30.0);
    assert_eq!(a.point("Track Point #1", 0.5), Some(p(115.0, 230.0)));
    let mocha = "Adobe After Effects 6.0 Keyframe Data\n\n\tUnits Per Second\t24\n\nEffects\tCC Power Pin #1\tTop Left\n\tFrame\tX pixels\tY pixels\n\t0\t0\t0\n\nEffects\tCC Power Pin #1\tTop Right\n\tFrame\tX pixels\tY pixels\n\t0\t10\t0\n\nEffects\tCC Power Pin #1\tBottom Right\n\tFrame\tX pixels\tY pixels\n\t0\t10\t10\n\nEffects\tCC Power Pin #1\tBottom Left\n\tFrame\tX pixels\tY pixels\n\t0\t0\t10\n\nEnd of Keyframe Data\n";
    let m = TrackData::parse(mocha, "mocha").unwrap();
    assert_eq!(m.plane("cornerpin", 0.0).unwrap()[1], p(10.0, 0.0));
    assert!(TrackData::parse("hello", "mocha").is_err());
    let fbx = r#"; FBX 7.4.0 project file
Objects:  {
	Model: 111, "Model::Cam", "Camera" {
	}
	AnimationCurveNode: 222, "AnimCurveNode::T", "" {
	}
	AnimationCurve: 333, "AnimCurve::", "" {
		KeyTime: *2 {
			a: 0,46186158000
		}
		KeyValueFloat: *2 {
			a: 1,3
		}
	}
}
Connections:  {
	C: "OO",222,111, "Lcl Translation"
	C: "OP",333,222, "d|X"
}
"#;
    let f = TrackData::parse(fbx, "fbx").unwrap();
    assert_eq!(f.camera("Cam", 1.0).unwrap()[0], 3.0);
    assert_eq!(f.point("Cam", 0.5), Some(p(2.0, 0.0)));
    assert!(TrackData::parse("Kaydara FBX Binary  \0", "fbx").is_err());
    let s = track::similarity(&[p(0.0, 0.0), p(1.0, 0.0)], &[p(5.0, 5.0), p(5.0, 7.0)]);
    assert!((s[0] - 2.0).abs() < 1e-9 && (s[1] - 90.0).abs() < 1e-9);
}
