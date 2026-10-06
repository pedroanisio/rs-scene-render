// Light grids (lighting="grid"): appended to the path tracer's shader in place of the exact
// volume_incident (see volume_grid_source in pathtrace.rs). A world-space lattice of nodes covers
// the media that ask for it; per analytic light a scalar grid holds the volume's transmittance
// times the surfaces' visibility from the node toward the light's centre (averaged over points
// jittered within a cell), and the dome is held either as radiance pre-integrated over fixed directions (isotropic scattering) or as one scalar
// grid per fixed direction (any anisotropy). All are built once a frame by cs_light_grid and
// cs_dome_grid, and read by trilinear interpolation; outside the lattice the exact march runs.
@group(1) @binding(0) var<storage, read_write> lgrid: array<vec4<f32>>;
// rows: 0 origin (xyz) and node spacing; 1 node counts (xyz) and flags (bit 0: dome radiance,
// bit 1: dome directions); 2 first scalar row, rows per scalar slot, dome directions, lights;
// 3 first radiance row. Scalar slots: one per light, then one per dome direction.
// Surface visibility toward an analytic light is averaged over this many points jittered within
// one cell around the node, so a surface thinner than a cell is a smooth shadow, not an aliased one.
const LIGHT_VISIBILITY_SAMPLES: u32 = 8u;
fn hash_u32(x: u32) -> u32 {
    var h=x; h^=h>>16u; h*=0x7feb352du; h^=h>>15u; h*=0x846ca68bu; h^=h>>16u; return h;
}
fn dome_direction(i: u32, count: u32) -> vec3<f32> {
    let y=1.0-2.0*(f32(i)+0.5)/f32(count); let r=sqrt(max(0.0,1.0-y*y)); let phi=f32(i)*2.3999632;
    return vec3(r*cos(phi),y,r*sin(phi));
}
// Transmittance times visibility from `p` toward scalar slot `slot`, interpolated, or -1 outside.
fn volume_grid_transmittance(slot: u32, p: vec3<f32>) -> f32 {
    let head=lgrid[0]; let dims=bitcast<vec3<u32>>(lgrid[1].xyz);
    let q=(p-head.xyz)/head.w;
    if (any(q<vec3(0.0)) || any(q>vec3<f32>(dims-vec3(1u)))) { return -1.0; }
    let i0=min(vec3<u32>(floor(q)),dims-vec3(2u)); let f=q-vec3<f32>(i0);
    let info=bitcast<vec2<u32>>(lgrid[2].xy);
    let data=info.x+slot*info.y;
    var value=0.0;
    for (var z=0u; z<2u; z++) { for (var y=0u; y<2u; y++) { for (var x=0u; x<2u; x++) {
        let n=(i0.x+x)+dims.x*((i0.y+y)+dims.y*(i0.z+z));
        let w=select(1.0-f.x,f.x,x==1u)*select(1.0-f.y,f.y,y==1u)*select(1.0-f.z,f.z,z==1u);
        value+=w*lgrid[data+n/4u][n%4u];
    }}}
    return value;
}
// Pre-integrated dome radiance at `p`, interpolated; w is -1 outside the lattice.
fn volume_grid_dome(p: vec3<f32>) -> vec4<f32> {
    let head=lgrid[0]; let dims=bitcast<vec3<u32>>(lgrid[1].xyz);
    let q=(p-head.xyz)/head.w;
    if (any(q<vec3(0.0)) || any(q>vec3<f32>(dims-vec3(1u)))) { return vec4(0.0,0.0,0.0,-1.0); }
    let i0=min(vec3<u32>(floor(q)),dims-vec3(2u)); let f=q-vec3<f32>(i0);
    let first=bitcast<u32>(lgrid[3].x);
    var value=vec3(0.0);
    for (var z=0u; z<2u; z++) { for (var y=0u; y<2u; y++) { for (var x=0u; x<2u; x++) {
        let n=(i0.x+x)+dims.x*((i0.y+y)+dims.y*(i0.z+z));
        let w=select(1.0-f.x,f.x,x==1u)*select(1.0-f.y,f.y,y==1u)*select(1.0-f.z,f.z,z==1u);
        value+=w*lgrid[first+n].rgb;
    }}}
    return vec4(value,1.0);
}
// One thread per row of four nodes of the scalar slot in pp.size.z.
@compute @workgroup_size(64)
fn cs_light_grid(@builtin(global_invocation_id) gid: vec3<u32>) {
    let head=lgrid[0]; let dims=bitcast<vec3<u32>>(lgrid[1].xyz);
    let info=bitcast<vec4<u32>>(lgrid[2]);
    let slot=u32(pp.size.z);
    let row=gid.x;
    if (row>=info.y) { return; }
    var out=vec4(1.0);
    let is_light=slot<info.w;
    if (!is_light || plights[slot].pos.w>=1.0) {
        for (var c=0u; c<4u; c++) {
            let n=row*4u+c;
            if (n>=dims.x*dims.y*dims.z) { continue; }
            let ix=n%dims.x; let iy=(n/dims.x)%dims.y; let iz=n/(dims.x*dims.y);
            let p=head.xyz+vec3<f32>(vec3(ix,iy,iz))*head.w;
            var d=vec3(0.0,1.0,0.0); var dist=1e30; var casts=true;
            if (!is_light) { d=dome_direction(slot-info.w,info.z); }
            else {
                let lt=plights[slot];
                casts=lt.size.y>0.5;
                if (u32(lt.pos.w)==1u) { d=-lt.dir.xyz; }
                else { let to=lt.pos.xyz-p; dist=length(to); d=to/max(dist,1e-4); }
            }
            let reach=max(0.0,dist-1e-3);
            var surface=1.0;
            if (casts) {
                if (!is_light) { surface=visibility(p,d,reach); }
                else {
                    var sum=0.0;
                    for (var s=0u; s<LIGHT_VISIBILITY_SAMPLES; s++) {
                        let h=hash_u32(n*747796405u+s*2891336453u+slot*1664525u);
                        let j=vec3(f32(h&1023u),f32((h>>10u)&1023u),f32((h>>20u)&1023u))/1023.0-vec3(0.5);
                        sum+=visibility(p+j*head.w,d,reach);
                    }
                    surface=sum/f32(LIGHT_VISIBILITY_SAMPLES);
                }
            }
            out[c]=volume_transmittance(p,d,reach)*surface;
        }
    }
    lgrid[info.x+slot*info.y+row]=out;
}
// One thread per node: the dome's radiance reaching it, averaged over the fixed directions.
@compute @workgroup_size(64)
fn cs_dome_grid(@builtin(global_invocation_id) gid: vec3<u32>) {
    let head=lgrid[0]; let dims=bitcast<vec3<u32>>(lgrid[1].xyz);
    let info=bitcast<vec4<u32>>(lgrid[2]);
    let first=bitcast<u32>(lgrid[3].x);
    let n=gid.x;
    if (n>=dims.x*dims.y*dims.z) { return; }
    let ix=n%dims.x; let iy=(n/dims.x)%dims.y; let iz=n/(dims.x*dims.y);
    let p=head.xyz+vec3<f32>(vec3(ix,iy,iz))*head.w;
    var sum=vec3(0.0);
    for (var k=0u; k<info.z; k++) {
        let d=dome_direction(k,info.z);
        let q=normalize((pp.env_rot*vec4(d,0.0)).xyz);
        let uv=vec2((atan2(q.x,q.z)+PI)/(2.0*PI),acos(clamp(-q.y,-1.0,1.0))/PI);
        let env=textureSampleLevel(env_tex,smp,uv,0.0).rgb*pp.env.y;
        sum+=env*(visibility(p,d,1e30)*volume_transmittance(p,d,1e30));
    }
    lgrid[first+n]=vec4(sum/f32(info.z),1.0);
}

// The exact lighting of a domain that does not ask for the grid.
fn volume_incident_exact(p: vec3<f32>, outgoing: vec3<f32>, g: f32, shadows: bool) -> vec3<f32> {
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
// Whether `p` lies on the lattice, where every grid can be read.
fn volume_grid_contains(p: vec3<f32>) -> bool {
    let head=lgrid[0]; let dims=bitcast<vec3<u32>>(lgrid[1].xyz);
    let q=(p-head.xyz)/head.w;
    return all(q>=vec3(0.0)) && all(q<=vec3<f32>(dims-vec3(1u)));
}
// In-scattered radiance at `p` of the domain at `base`: from the grids when that domain asks for
// them and `p` is on the lattice, else by the exact march.
fn volume_incident(p: vec3<f32>, outgoing: vec3<f32>, g: f32, shadows: bool, base: u32) -> vec3<f32> {
    if (tverts[base+13u].z<0.5 || !shadows || !volume_grid_contains(p)) { return volume_incident_exact(p,outgoing,g,shadows); }
    var light=vec3(0.0);
    for (var i=0u; i<u32(pp.ambient.w); i++) {
        let lt=plights[i];
        if (lt.pos.w<0.0 || light_lobes(lt).x==0.0) { continue; }
        if (lt.pos.w==0.0) { light+=lt.color.rgb; continue; }
        let sample=light_sample(lt,p);
        light+=light_radiance(lt,sample.xyz,sample.w)*volume_grid_transmittance(i,p)*volume_phase(dot(-sample.xyz,outgoing),g);
    }
    if (pp.env.x>0.5) {
        let flags=bitcast<u32>(lgrid[1].w);
        // isotropic scattering reads the pre-integrated radiance
        if (tverts[base+6u].w==0.0 && (flags&1u)!=0u) { return light+volume_grid_dome(p).rgb; }
        // otherwise one of the fixed directions, each with its own grid
        let info=bitcast<vec4<u32>>(lgrid[2]);
        let k=min(u32(rnd()*f32(info.z)),info.z-1u);
        let d=dome_direction(k,info.z);
        let q=normalize((pp.env_rot*vec4(d,0.0)).xyz);
        let uv=vec2((atan2(q.x,q.z)+PI)/(2.0*PI),acos(clamp(-q.y,-1.0,1.0))/PI);
        let env=textureSampleLevel(env_tex,smp,uv,0.0).rgb*pp.env.y;
        light+=env*(4.0*PI*volume_phase(dot(-d,outgoing),g)*volume_grid_transmittance(info.w+k,p));
    }
    return light;
}
