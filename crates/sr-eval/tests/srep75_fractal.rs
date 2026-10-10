//! SREP 75: the escape counts of the kit (`conformance/srep_cases/srep-0075.json`), exactly. The kit computed them in
//! decimal arithmetic at 60 and 90 significant digits, at each sample pixel's centre and at the four points 2⁻²⁰ of a
//! pixel away, and kept a pixel only when all ten agreed; every conforming engine shows those counts. Zoom 6 is just
//! past single precision (the direct double method here), zoom 12 within double precision and zoom 20 past it (both by
//! perturbation).

use sr_eval::fractal::{escapes_at, method, Escape, Fractal, Method};

const AT: [(u32, u32); 9] =
    [(40, 30), (600, 30), (320, 180), (40, 330), (600, 330), (160, 90), (480, 270), (100, 200), (520, 120)];

fn kit(zoom: f64, max: u64) -> Fractal {
    Fractal {
        julia: None,
        width: 640,
        height: 360,
        center: ["-0.743643887037158704752191506114774".into(), "0.131825904205311970493132056385139".into()],
        span: 4.0,
        zoom,
        rotation: 0.0,
        max_iterations: max,
        bailout: 2.0,
    }
}

fn counts(f: &Fractal) -> Vec<Option<u64>> {
    escapes_at(f, &AT).unwrap().into_iter().map(Escape::count).collect()
}

#[test]
fn srep_0075_mandelbrot_zoom_6() {
    let f = kit(6.0, 3000);
    assert_eq!(method(&f), Method::Direct);
    assert_eq!(counts(&f), [Some(223), None, Some(1005), Some(275), Some(1308), Some(486), None, Some(307), None]);
}

#[test]
fn srep_0075_mandelbrot_zoom_12() {
    let f = kit(12.0, 12000);
    assert_eq!(method(&f), Method::Perturbation);
    assert_eq!(
        counts(&f),
        [Some(2054), Some(1986), Some(4047), Some(1985), Some(2221), Some(2052), Some(2064), Some(1972), Some(2086)]
    );
}

#[test]
fn srep_0075_mandelbrot_zoom_20() {
    let f = kit(20.0, 20000);
    assert_eq!(method(&f), Method::Perturbation);
    assert_eq!(
        counts(&f),
        [Some(8510), Some(9061), Some(10056), Some(9059), Some(8485), Some(9943), Some(9760), Some(10082), Some(9228)]
    );
}

#[test]
fn the_centre_is_read_past_the_precision_of_a_double() {
    // two centres 1e-19 apart that are the same double: at zoom 20 that is 1e-19 / p = 1600 pixels
    let a = kit(20.0, 20000);
    let mut b = a.clone();
    b.center[0] = "-0.743643887037158704752191506114774".replacen("158704752191", "158704852191", 1);
    assert_eq!(a.center[0].parse::<f64>().unwrap(), b.center[0].parse::<f64>().unwrap());
    assert_ne!(counts(&a), counts(&b));
}

#[test]
fn a_zoom_beyond_doubles_uses_the_extended_exponent() {
    // a point inside the main cardioid stays inside at any depth; a point far outside escapes at once
    let mut f = kit(400.0, 200);
    f.center = ["-0.1".into(), "0.1".into()];
    assert_eq!(method(&f), Method::PerturbationExtended);
    assert!(escapes_at(&f, &[(0, 0), (639, 359)]).unwrap().iter().all(|e| *e == Escape::Inside));
    f.center = ["3".into(), "0".into()];
    assert!(escapes_at(&f, &[(0, 0), (639, 359)]).unwrap().iter().all(|e| e.count() == Some(1)));
}
