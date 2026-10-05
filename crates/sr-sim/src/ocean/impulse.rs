use super::{Error, Spec, Q};

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ImpulseKind {
    /// Adds nonnegative depth with compact (1-r²)² radial falloff.
    AddWater,
    /// Transfers water between a central disc and an annulus, conserving mass.
    Displace,
    /// A cavity: moves water out of the central disc into the annulus like a negative
    /// [`ImpulseKind::Displace`], where `amplitude` (not negative) is the depth wanted removed
    /// at the centre. It moves the wanted volume, or at most [`CAVITY_SHARE`] of the water the
    /// disc holds, whichever is less, so it is limited by the water layer and never an error
    /// for lack of donor water; it removes from each column in proportion to its depth and
    /// never more than [`CAVITY_SHARE`] of it. A disc or annulus without water or without a
    /// cell centre moves nothing.
    Cavity,
    /// One part of a cavity that forms over several steps: a `share` of the whole (`0 < share <= 1`),
    /// after the fraction `before` of it (`0 <= before`, `before + share <= 1`) has formed. As
    /// [`ImpulseKind::Cavity`] with the `amplitude` of the part, limited to
    /// `CAVITY_SHARE * share / (1 - CAVITY_SHARE * rho * before)` of the water the disc holds now, where `rho`
    /// is the share of what a part takes that the disc's weighted water falls by (the mean of the kernel
    /// weight over the water, at most 1). If the water were to stay put the parts together then take the
    /// limit of one whole cavity: [`CAVITY_SHARE`] of what the disc held when it began.
    CavityPart { share: f64, before: f64 },
}
/// The most of the water of a disc, and of any column in it, that a cavity removes.
pub const CAVITY_SHARE: f64 = 0.9;
#[derive(Clone, Debug)]
pub struct Impulse {
    pub time: f64,
    pub center: [f64; 2],
    pub radius: f64,
    /// Peak depth addition; for displacement, central kernel amplitude.
    /// Negative displacement moves water from the disc into its annulus.
    pub amplitude: f64,
    /// Horizontal velocity kick with the full-disc compact radial falloff.
    pub velocity: [f64; 2],
    pub kind: ImpulseKind,
}
impl Impulse {
    pub(super) fn validate(&self) -> Result<(), Error> {
        if !self.time.is_finite()
            || self.time < 0.0
            || !self.radius.is_finite()
            || self.radius <= 0.0
            || !self.amplitude.is_finite()
            || self.center.iter().chain(&self.velocity).any(|x| !x.is_finite())
            || (matches!(self.kind, ImpulseKind::AddWater | ImpulseKind::Cavity | ImpulseKind::CavityPart { .. })
                && self.amplitude < 0.0)
            || matches!(self.kind, ImpulseKind::CavityPart { share, before } if !(share > 0.0 && before >= 0.0 && before + share <= 1.0 + 1e-9))
        {
            return Err(Error::Invalid("impulse time, shape, amplitude or velocity"));
        }
        Ok(())
    }
    pub(super) fn apply(&self, spec: &Spec, q: &mut [Q]) -> Result<(), Error> {
        let weights: Vec<_> = (0..q.len())
            .map(|i| {
                let x = spec.origin[0] + (i % spec.cells[0]) as f64 * spec.cell_size + 0.5 * spec.cell_size;
                let z = spec.origin[1] + (i / spec.cells[0]) as f64 * spec.cell_size + 0.5 * spec.cell_size;
                let r = ((x - self.center[0]) / self.radius).hypot((z - self.center[1]) / self.radius);
                let full = (1.0 - r * r).max(0.0).powi(2);
                let center = (1.0 - 4.0 * r * r).max(0.0).powi(2);
                let ring =
                    if (0.5..1.0).contains(&r) { (std::f64::consts::TAU * (r - 0.5)).sin().powi(2) } else { 0.0 };
                [full, center, ring]
            })
            .collect();
        if !matches!(self.kind, ImpulseKind::Cavity | ImpulseKind::CavityPart { .. })
            && !weights.iter().any(|w| w[0] > 0.0)
            && (self.amplitude != 0.0 || self.velocity != [0.0; 2])
        {
            return Err(Error::Invalid("impulse does not cover any cell centre"));
        }
        let mut delta = vec![0.0; q.len()];
        match self.kind {
            ImpulseKind::AddWater => {
                for (d, w) in delta.iter_mut().zip(&weights) {
                    *d = self.amplitude * w[0];
                }
            }
            ImpulseKind::Displace if self.amplitude != 0.0 => {
                let (donor, receiver) = if self.amplitude > 0.0 { (2, 1) } else { (1, 2) };
                let amount = self.amplitude.abs() * weights.iter().map(|w| w[1]).sum::<f64>();
                let capacity: f64 = weights.iter().zip(q.iter()).map(|(w, q)| w[donor] * q[0]).sum();
                let received: f64 = weights.iter().map(|w| w[receiver]).sum();
                if !amount.is_finite()
                    || !capacity.is_finite()
                    || !received.is_finite()
                    || amount == 0.0
                    || capacity < amount
                    || received == 0.0
                {
                    return Err(Error::Invalid("displacement needs resolved disc/annulus and sufficient donor water"));
                }
                for ((d, w), q) in delta.iter_mut().zip(&weights).zip(q.iter()) {
                    *d = amount * (w[receiver] / received - w[donor] * q[0] / capacity);
                }
            }
            ImpulseKind::Displace => {}
            ImpulseKind::Cavity | ImpulseKind::CavityPart { .. } if self.amplitude > 0.0 => {
                let wanted = self.amplitude * weights.iter().map(|w| w[1]).sum::<f64>();
                let capacity: f64 = weights.iter().zip(q.iter()).map(|(w, q)| w[1] * q[0]).sum();
                let received: f64 = weights.iter().map(|w| w[2]).sum();
                // the limit of the whole cavity, spread over its parts
                let limit = if let ImpulseKind::CavityPart { share, before } = self.kind {
                    let squared: f64 = weights.iter().zip(q.iter()).map(|(w, q)| w[1] * w[1] * q[0]).sum();
                    let rho = if capacity > 0.0 { squared / capacity } else { 0.0 };
                    CAVITY_SHARE * share / (1.0 - CAVITY_SHARE * rho * before)
                } else {
                    CAVITY_SHARE
                };
                let amount = wanted.min(limit * capacity);
                if amount.is_finite() && amount > 0.0 && received > 0.0 {
                    for ((d, w), q) in delta.iter_mut().zip(&weights).zip(q.iter()) {
                        *d = amount * (w[2] / received - w[1] * q[0] / capacity);
                    }
                }
            }
            ImpulseKind::Cavity | ImpulseKind::CavityPart { .. } => {}
        }
        for ((q, d), w) in q.iter_mut().zip(delta).zip(weights) {
            if w[0] == 0.0 {
                continue;
            }
            let u = if q[0] < spec.dry_tolerance { [0.0; 2] } else { [q[1] / q[0], q[2] / q[0]] };
            q[0] += d;
            for a in 0..2 {
                q[a + 1] = q[0] * (u[a] + w[0] * self.velocity[a]);
            }
            if q.iter().any(|v| !v.is_finite()) || q[0] < 0.0 {
                return Err(Error::Numerical("impulse overflow or negative depth"));
            }
        }
        Ok(())
    }
}
