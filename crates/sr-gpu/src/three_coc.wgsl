fn lin_depth(d: f32) -> f32 {
    // reverse-Z perspective: d = n(f − z)/(z(f − n))  ⇒  z = n f / (d (f − n) + n)
    let n = pfr.post.z;
    let f = pfr.post.w;
    return n * f / (d * (f - n) + n);
}

fn coc(d: f32) -> f32 {
    let z = lin_depth(d);
    return min(pfr.dof.x * abs(1.0 / pfr.dof.y - 1.0 / max(z, 1e-3)), pfr.dof.z);
}
