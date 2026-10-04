// Participating media packed into tverts. Each domain has 64 vec4 rows; each
// sparse 8^3 brick has one signed integer key row and 128 scalar-data rows.
fn volume_base(i: u32) -> u32 { return pp.media.x + i * 64u; }
fn volume_matrix(base: u32) -> mat4x4<f32> {
    return mat4x4(tverts[base],tverts[base+1u],tverts[base+2u],tverts[base+3u]);
}
fn volume_interval(base: u32, origin: vec3<f32>, direction: vec3<f32>, distance: f32) -> vec2<f32> {
    let inverse = volume_matrix(base);
    let o = (inverse * vec4(origin,1.0)).xyz;
    let d = (inverse * vec4(direction,0.0)).xyz;
    let lo = tverts[base+4u].xyz; let hi = tverts[base+5u].xyz;
    if (any(lo >= hi)) { return vec2(0.0); }
    var near = 0.0; var far = distance;
    for (var axis=0u; axis<3u; axis++) {
        if (d[axis] == 0.0) {
            if (o[axis]<lo[axis] || o[axis]>hi[axis]) { return vec2(0.0); }
        } else {
            let a=(lo[axis]-o[axis])/d[axis]; let b=(hi[axis]-o[axis])/d[axis];
            near=max(near,min(a,b)); far=min(far,max(a,b));
        }
    }
    if (far <= near) { return vec2(0.0); }
    return vec2(near,far);
}
fn volume_key_less(a: vec3<i32>, b: vec3<i32>) -> bool {
    return a.x<b.x || (a.x==b.x && (a.y<b.y || (a.y==b.y && a.z<b.z)));
}
fn volume_voxel(info: vec4<f32>, index: vec3<i32>) -> f32 {
    let key=vec3<i32>(floor(vec3<f32>(index)/8.0));
    var lo=0u; var hi=bitcast<u32>(info.y); let offset=bitcast<u32>(info.z);
    loop {
        if (lo>=hi) { break; }
        let mid=lo+(hi-lo)/2u;
        let k=bitcast<vec3<i32>>(tverts[offset+mid*129u].xyz);
        if (volume_key_less(k,key)) { lo=mid+1u; } else { hi=mid; }
    }
    if (lo>=bitcast<u32>(info.y)) { return info.x; }
    let row=offset+lo*129u;
    if (any(bitcast<vec3<i32>>(tverts[row].xyz)!=key)) { return info.x; }
    let q=vec3<u32>(index-key*8);
    let scalar=q.x+q.y*8u+q.z*64u;
    return tverts[row+1u+scalar/4u][scalar%4u];
}
fn volume_grid_sample(info: vec4<f32>, p: vec3<f32>) -> f32 {
    if (any(abs(p)>vec3(8388600.0))) { return info.x; }
    let lo=vec3<i32>(floor(p)); let f=fract(p);
    var value=0.0;
    for (var z=0; z<2; z++) { for (var y=0; y<2; y++) { for (var x=0; x<2; x++) {
        let delta=vec3(x,y,z);
        let weights=select(vec3(1.0)-f,f,delta==vec3(1));
        value+=volume_voxel(info,lo+delta)*weights.x*weights.y*weights.z;
    }}}
    return value;
}
fn volume_grid(base: u32, point: vec3<f32>) -> f32 {
    return volume_grid_sample(tverts[base+4u],(volume_matrix(base)*vec4(point,1.0)).xyz);
}
fn volume_velocity(row: u32, local: vec3<f32>) -> vec3<f32> {
    let p=(volume_matrix(row)*vec4(local,1.0)).xyz;
    // Runtime loop bounds keep driver inlining from duplicating the sparse
    // search for every component and RK stage at every transport call site.
    var velocity=vec3(0.0);
    for (var c=0u; c<u32(tverts[row+7u].y); c++) {
        velocity[c]=volume_grid_sample(tverts[row+4u+c],p);
    }
    return velocity;
}
fn volume_advected_point(base: u32, point: vec3<f32>, endpoint: u32) -> vec3<f32> {
    if (tverts[base+14u].z<0.5) { return point; }
    let row=base+40u+endpoint*8u; let elapsed=tverts[row+7u].x;
    if (elapsed==0.0) { return point; }
    let local=(volume_matrix(base)*vec4(point,1.0)).xyz;
    var traced=local;
    for (var stage=0u; stage<u32(tverts[row+7u].z); stage++) {
        traced=local-(elapsed*(0.5*f32(stage+1u)))*volume_velocity(row,traced);
    }
    return (volume_matrix(base+56u)*vec4(traced,1.0)).xyz;
}
fn volume_density(base: u32, point: vec3<f32>) -> f32 {
    let local=(volume_matrix(base)*vec4(point,1.0)).xyz;
    if (any(local<tverts[base+4u].xyz) || any(local>tverts[base+5u].xyz)) { return 0.0; }
    var density=volume_grid(base+8u,volume_advected_point(base,point,0u));
    if (tverts[base+14u].y>0.5) { density=mix(density,volume_grid(base+24u,volume_advected_point(base,point,1u)),tverts[base+14u].x); }
    return density*tverts[base+4u].w;
}
fn volume_emission(base: u32, point: vec3<f32>) -> vec3<f32> {
    var rgb=tverts[base+7u].rgb;
    let thermal=tverts[base+21u];
    if (thermal.y>0.5) {
        var temperature=volume_grid(base+16u,volume_advected_point(base,point,0u));
        if (tverts[base+14u].y>0.5) { temperature=mix(temperature,volume_grid(base+32u,volume_advected_point(base,point,1u)),tverts[base+14u].x); }
        let kelvin=clamp(temperature*thermal.x,0.0,50000.0);
        let index=sqrt(kelvin/50000.0)*1024.0;
        let lo=min(u32(index),1023u); let fraction=index-f32(lo);
        let offset=bitcast<u32>(thermal.z);
        rgb+=mix(tverts[offset+lo].rgb,tverts[offset+lo+1u].rgb,fraction);
    }
    return rgb;
}
fn volume_phase(cosine: f32, g: f32) -> f32 {
    let denom=1.0+g*g-2.0*g*clamp(cosine,-1.0,1.0);
    return (1.0-g*g)/(4.0*PI*denom*sqrt(denom));
}
// Skip map of a domain's density grid (see volume.rs): the cell holding `point` in 4-voxel cells
// of grid index space. Returns the ray parameter where the ray leaves that cell when it holds only
// negligible density, else -1. The caller must have checked that the domain has no advection and
// no second frame, so a sample reads only this grid.
fn volume_empty_until(base: u32, o: vec3<f32>, d: vec3<f32>, point: vec3<f32>) -> f32 {
    let m=volume_matrix(base+8u);
    let p=(m*vec4(point,1.0)).xyz;
    let c=vec3<i32>(floor(p/4.0));
    let first=tverts[base+15u];
    let dims=bitcast<vec3<u32>>(tverts[base+22u].xyz);
    let rel=c-bitcast<vec3<i32>>(first.xyz);
    if (all(rel>=vec3<i32>(0)) && all(vec3<u32>(rel)<dims)) {
        let cell=u32(rel.x)+dims.x*(u32(rel.y)+dims.y*u32(rel.z));
        let word=bitcast<u32>(tverts[bitcast<u32>(first.w)+cell/128u][(cell/32u)%4u]);
        if (((word>>(cell%32u))&1u)!=0u) { return -1.0; }
    }
    let oi=(m*vec4(o,1.0)).xyz; let di=(m*vec4(d,0.0)).xyz;
    var t=3.0e38;
    for (var axis=0u; axis<3u; axis++) {
        if (di[axis]>0.0) { t=min(t,(f32(c[axis]*4+4)-oi[axis])/di[axis]); }
        else if (di[axis]<0.0) { t=min(t,(f32(c[axis]*4)-oi[axis])/di[axis]); }
    }
    return t;
}
// Extinction is additive across overlapping domains; their transmittances multiply.
// This separate function avoids recursive shader calls when evaluating in-scattering.
// Samples in cells whose density is negligible are jumped over (stopping a little short of the
// cell's far side to stay clear of rounding). The map is built so that, along any ray through the
// domain, everything left out adds less than 1e-10 to the optical depth (volume.rs,
// SKIPPED_OPTICAL_DEPTH): about 300 times below half a unit in the last place of an f32 near 1, so
// far below the resolution of the transmittance, but not zero.
fn volume_transmittance(o: vec3<f32>, d: vec3<f32>, distance: f32) -> f32 {
    var optical_depth=0.0;
    for (var i=0u; i<pp.media.y; i++) {
        let base=volume_base(i); let interval=volume_interval(base,o,d,distance);
        if (tverts[base+13u].x<0.5) { continue; }
        if (interval.y<=interval.x) { continue; }
        let n=max(1u,u32(ceil((interval.y-interval.x)/tverts[base+12u].w)));
        let ds=(interval.y-interval.x)/f32(n);
        let skipping=tverts[base+22u].w>0.5;
        var k=0u;
        loop {
            if (k>=n) { break; }
            let point=o+d*(interval.x+(f32(k)+0.5)*ds);
            if (skipping) {
                let until=volume_empty_until(base,o,d,point);
                if (until>-0.5) {
                    let last=floor((until-0.02-interval.x)/ds-0.5);
                    k=max(k+1u,u32(clamp(last+1.0,0.0,f32(n))));
                    continue;
                }
            }
            optical_depth+=volume_density(base,point)*tverts[base+5u].w*ds;
            k++;
        }
    }
    return exp(-optical_depth);
}
fn volume_incident(p: vec3<f32>, outgoing: vec3<f32>, g: f32, shadows: bool) -> vec3<f32> {
    var light=vec3(0.0);
    for (var i=0u; i<u32(pp.ambient.w); i++) {
        let lt=plights[i];
        if (lt.pos.w<0.0 || light_lobes(lt).x==0.0) { continue; }
        // Authored ambient is already an isotropic local radiance approximation.
        if (lt.pos.w==0.0) { light+=lt.color.rgb; continue; }
        let sample=light_sample(lt,p);
        let distance=max(0.0,sample.w-1e-3);
        var visible=1.0;
        if (shadows) {
            visible=volume_transmittance(p,sample.xyz,distance);
            if (lt.size.y>0.5) { visible*=visibility(p,sample.xyz,distance); }
        }
        light+=light_radiance(lt,sample.xyz,sample.w)*visible*volume_phase(dot(-sample.xyz,outgoing),g);
    }
    if (pp.env.x>0.5) {
        let z=2.0*rnd()-1.0; let a=2.0*PI*rnd(); let r=sqrt(max(0.0,1.0-z*z));
        let d=vec3(r*cos(a),r*sin(a),z);
        let q=normalize((pp.env_rot*vec4(d,0.0)).xyz);
        let uv=vec2((atan2(q.x,q.z)+PI)/(2.0*PI),acos(clamp(-q.y,-1.0,1.0))/PI);
        let env=textureSampleLevel(env_tex,smp,uv,0.0).rgb*pp.env.y;
        var visible=1.0;
        if (shadows) { visible=visibility(p,d,1e30)*volume_transmittance(p,d,1e30); }
        light+=env*(4.0*PI*volume_phase(dot(-d,outgoing),g)*visible);
    }
    return light;
}
// Returns premultiplied radiance and surviving transmittance (not opacity).
fn volume_transport(o: vec3<f32>, d: vec3<f32>, distance: f32) -> vec4<f32> {
    var color=vec3(0.0); var trans=1.0; var cursor=0.0;
    // At most two boundaries per domain. Split at every entry/exit so thin layers
    // cannot fall between samples and widely separated domains skip empty space.
    for (var boundary=0u; boundary<=pp.media.y*2u; boundary++) {
        var next=distance; var step=1e30; var occupied=false;
        for (var i=0u; i<pp.media.y; i++) {
            let base=volume_base(i); let interval=volume_interval(base,o,d,distance);
            if (interval.y<=interval.x || interval.y<=cursor) { continue; }
            if (interval.x>cursor) { next=min(next,interval.x); }
            else { next=min(next,interval.y); step=min(step,tverts[base+12u].w); occupied=true; }
        }
        if (next<=cursor) { break; }
        if (occupied) {
            let n=max(1u,u32(ceil((next-cursor)/step))); let ds=(next-cursor)/f32(n);
            for (var k=0u; k<n; k++) {
                let point=o+d*(cursor+(f32(k)+0.5)*ds);
                var sigma=0.0; var source=vec3(0.0);
                for (var i=0u; i<pp.media.y; i++) {
                    let base=volume_base(i); let density=volume_density(base,point);
                    if (density<=0.0) { continue; }
                    let extinction=density*tverts[base+5u].w;
                    sigma+=extinction; source+=density*volume_emission(base,point);
                    let albedo=tverts[base+6u];
                    if (MEDIUM_LIGHTING && any(albedo.rgb>vec3(0.0))) { source+=extinction*albedo.rgb*volume_incident(point,-d,albedo.w,tverts[base+13u].y>0.5); }
                }
                let optical_depth=sigma*ds;
                let attenuation=exp(-optical_depth);
                // Series avoids cancellation at small optical depths, including vacuum emission.
                var weight=ds*(1.0-0.5*optical_depth+optical_depth*optical_depth/6.0);
                if (optical_depth>1e-3) { weight=(1.0-attenuation)/sigma; }
                color+=trans*source*weight; trans*=attenuation;
            }
        }
        cursor=next;
        if (cursor>=distance) { break; }
    }
    return vec4(color,trans);
}
