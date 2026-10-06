//! Writes the CPU image of a Schwarzschild black hole with a disc as a binary PPM, for looking at: `cargo run --release
//! -p sr-sim --example blackhole_cpu -- out.ppm [inclination degrees] [width] [height]`. The brightness of the disc is
//! the bolometric `(g T)^4` of its temperature, so the side that approaches is the brighter.
use sr_sim::gr::image::{render, Camera, Class, Disk};
use sr_sim::gr::oracle;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let path = args.get(1).expect("the file to write");
    let degrees: f64 = args.get(2).map_or(75.0, |v| v.parse().expect("degrees"));
    let width: usize = args.get(3).map_or(640, |v| v.parse().expect("width"));
    let height: usize = args.get(4).map_or(360, |v| v.parse().expect("height"));
    let mass = 1.0;
    let camera = Camera::orbiting(60.0 * mass, degrees.to_radians(), 0.35, [width, height]);
    let disk = Disk::flat(oracle::isco(mass), 20.0 * mass);
    let peak = oracle::disc_temperature(oracle::isco(mass) * 49.0 / 36.0, oracle::isco(mass), 1.0);
    let mut out = format!("P6\n{width} {height}\n255\n").into_bytes();
    for p in render(&camera, mass, Some(&disk)) {
        let value = match p.class {
            Class::Captured => 0.0,
            Class::Background => 0.0,
            Class::Disk => {
                let t = oracle::disc_temperature(p.r, disk.r_in, 1.0) / peak;
                (p.g * t).powi(4)
            }
        };
        let byte = (255.0 * (value / 1.6).clamp(0.0, 1.0).powf(1.0 / 2.2)) as u8;
        out.extend([byte, byte, byte]);
    }
    std::fs::write(path, out).expect("write the image");
}
