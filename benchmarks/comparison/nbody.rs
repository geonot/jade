fn dist_sq(ax: i64, ay: i64, az: i64, bx: i64, py: i64, bz: i64) -> i64 {
    let dx = ax - bx;
    let dy = ay - py;
    let dz = az - bz;
    dx*dx + dy*dy + dz*dz
}

fn main() {
    let (mut x0,mut y0,mut z0,mut vx0,mut vy0,mut vz0,m0) = (0i64,0,0,0,0,100,1000i64);
    let (mut x1,mut y1,mut z1,mut vx1,mut vy1,mut vz1,_m1) = (1000i64,0,0,0,50,0,10i64);
    let (mut x2,mut y2,mut z2,mut vx2,mut vy2,mut vz2,_m2) = (0i64,1000,0,-50,0,0,10i64);
    let (mut x3,mut y3,mut z3,mut vx3,mut vy3,mut vz3,_m3) = (500i64,500,500,-20,20,-20,5i64);
    let (mut x4,mut y4,mut z4,mut vx4,mut vy4,mut vz4,_m4) = (-500i64,-500,-500,30,-10,30,5i64);

    for _ in 0..10_000_000 {
        let d = dist_sq(x0,y0,z0,x1,y1,z1) + 1;
        vx1 += (x0-x1)*m0/d;
        vy1 += (y0-y1)*m0/d;

        let d = dist_sq(x0,y0,z0,x2,y2,z2) + 1;
        vx2 += (x0-x2)*m0/d;
        vy2 += (y0-y2)*m0/d;

        x0+=vx0; y0+=vy0; z0+=vz0;
        x1+=vx1; y1+=vy1; z1+=vz1;
        x2+=vx2; y2+=vy2; z2+=vz2;
        x3+=vx3; y3+=vy3; z3+=vz3;
        x4+=vx4; y4+=vy4; z4+=vz4;
    }
    println!("{}", x0+x1+x2+x3+x4);
}
