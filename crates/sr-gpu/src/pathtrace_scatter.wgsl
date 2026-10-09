// One weighted reservoir over the primary march starts a bounded backward walk.
// Choosing by the largest color component keeps colored media unbiased: the
// chosen RGB coefficient is divided by its scalar selection probability.
struct ScatterReservoir {
    point: vec3<f32>,
    coefficient: vec3<f32>,
    mass: f32,
    chosen_mass: f32,
    base: u32,
}
fn scatter_offer(reservoir: ptr<function, ScatterReservoir>, point: vec3<f32>, weight: vec3<f32>) {
    for (var i=0u; i<pp.media.y; i++) {
        let base=volume_base(i);
        if (tverts[base+13u].w<=1.0) { continue; }
        let coefficient=weight*volume_density(base,point)*tverts[base+5u].w*tverts[base+6u].rgb;
        let mass=max(coefficient.x,max(coefficient.y,coefficient.z));
        if (mass<=0.0) { continue; }
        (*reservoir).mass+=mass;
        if (rnd()*(*reservoir).mass<mass) {
            (*reservoir).point=point;
            (*reservoir).coefficient=coefficient;
            (*reservoir).chosen_mass=mass;
            (*reservoir).base=base;
        }
    }
}
// Sample the direction of incoming propagation about the outgoing direction.
fn scatter_direction(outgoing: vec3<f32>, g: f32) -> vec3<f32> {
    let u=rnd();
    var c=1.0-2.0*u;
    if (abs(g)>=1e-3) {
        let s=(1.0-g*g)/(1.0-g+2.0*g*u);
        c=clamp((1.0+g*g-s*s)/(2.0*g),-1.0,1.0);
    }
    let a=2.0*PI*rnd(); let s=sqrt(max(0.0,1.0-c*c));
    let axis=select(vec3(0.0,0.0,1.0),vec3(1.0,0.0,0.0),abs(outgoing.z)>0.9);
    let x=normalize(cross(outgoing,axis)); let y=cross(outgoing,x);
    return -(outgoing*c+s*(cos(a)*x+sin(a)*y));
}
struct ScatterHit { point: vec3<f32>, base: u32, distance: f32, emission: vec3<f32>, }
// Invert the integrated optical depth of the same piecewise-constant density
// quadrature used by transport. Boundaries split thin and separated domains.
fn scatter_flight(o: vec3<f32>, d: vec3<f32>, sigma_water: vec3<f32>) -> ScatterHit {
    let distance=trace(o,d,1e30).t;
    var tau=-log(max(1.0-rnd(),1e-7));
    var cursor=0.0; var emission=vec3(0.0);
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
                    let base=volume_base(i);
                    let density=volume_density(base,point);
                    sigma+=density*tverts[base+5u].w;
                    source+=density*volume_emission(base,point);
                }
                let length=select(ds,min(ds,tau/max(sigma,1e-30)),sigma>0.0);
                emission+=source*length*exp(-sigma_water*(cursor+f32(k)*ds+0.5*length));
                if (sigma>0.0 && tau<sigma*ds) {
                    var pick=rnd()*sigma;
                    var selected=0xffffffffu;
                    for (var i=0u; i<pp.media.y; i++) {
                        let base=volume_base(i);
                        let extinction=volume_density(base,point)*tverts[base+5u].w;
                        if (extinction<=0.0) { continue; }
                        selected=base;
                        pick-=extinction;
                        if (pick<=0.0) { break; }
                    }
                    // Rounding at the end of the cumulative distribution selects
                    // its last nonempty domain, never a negative flight next step.
                    let traveled=cursor+f32(k)*ds+tau/sigma;
                    return ScatterHit(o+d*traveled,selected,traveled,emission);
                }
                tau-=sigma*ds;
            }
        }
        cursor=next;
        if (cursor>=distance) { break; }
    }
    return ScatterHit(vec3(0.0),0xffffffffu,cursor,emission);
}
fn scatter_walk(reservoir: ScatterReservoir, toward_viewer: vec3<f32>, sigma_water: vec3<f32>) -> vec3<f32> {
    if (reservoir.chosen_mass<=0.0) { return vec3(0.0); }
    var point=reservoir.point; var base=reservoir.base; var outgoing=toward_viewer;
    var beta=reservoir.coefficient*(reservoir.mass/reservoir.chosen_mass);
    var color=vec3(0.0);
    var limit=min(32u,u32(tverts[base+13u].w));
    for (var bounce=1u; bounce<limit; bounce++) {
        let direction=scatter_direction(outgoing,tverts[base+6u].w);
        let hit=scatter_flight(point,direction,sigma_water);
        color+=beta*hit.emission;
        if (hit.base==0xffffffffu) { break; }
        base=hit.base; point=hit.point; outgoing=-direction;
        limit=min(limit,u32(tverts[base+13u].w));
        if (bounce>=limit) { break; }
        beta*=tverts[base+6u].rgb*exp(-sigma_water*hit.distance);
        if (max(beta.x,max(beta.y,beta.z))<=0.0) { break; }
        color+=beta*scatter_incident(point, outgoing, base);
    }
    return color;
}
